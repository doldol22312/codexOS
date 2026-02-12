#![allow(dead_code)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::arch::asm;
use core::cell::UnsafeCell;
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{interrupts::InterruptFrame, timer};

pub type TaskId = u32;
pub type TaskEntry = fn(*mut u8);

const MAX_TASKS: usize = 16;
const DEFAULT_STACK_SIZE: usize = 64 * 1024;
const IDLE_STACK_SIZE: usize = 16 * 1024;
const MIN_STACK_SIZE: usize = 4096;

const KERNEL_CODE_SELECTOR: u32 = 0x08;
const KERNEL_DATA_SELECTOR: u32 = 0x10;
const INITIAL_EFLAGS: u32 = 0x202;

const SYSCALL_YIELD: u32 = 1;
const SYSCALL_SLEEP: u32 = 2;

static SCHEDULER_ONLINE: AtomicBool = AtomicBool::new(false);

struct SchedulerCell(UnsafeCell<Option<Scheduler>>);

unsafe impl Sync for SchedulerCell {}

static SCHEDULER: SchedulerCell = SchedulerCell(UnsafeCell::new(None));

#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskState {
    Ready,
    Running,
    Sleeping,
    Exited,
}

#[derive(Clone, Copy)]
enum ScheduleCause {
    Timer,
    Yield,
    Sleep(u32),
}

struct Task {
    id: TaskId,
    state: TaskState,
    wake_tick: u32,
    _stack: Option<Box<[u8]>>,
    saved_esp: u32,
    entry: TaskEntry,
    arg: *mut u8,
    is_idle: bool,
}

impl Task {
    fn is_runnable(&self) -> bool {
        matches!(self.state, TaskState::Ready | TaskState::Running)
    }
}

struct Scheduler {
    tasks: Vec<Task>,
    current_index: usize,
    next_id: TaskId,
}

impl Scheduler {
    fn new() -> Self {
        Self {
            tasks: Vec::new(),
            current_index: 0,
            next_id: 1,
        }
    }

    fn add_bootstrap_task(&mut self) {
        self.tasks.push(Task {
            id: 0,
            state: TaskState::Running,
            wake_tick: 0,
            _stack: None,
            saved_esp: 0,
            entry: bootstrap_task_entry,
            arg: ptr::null_mut(),
            is_idle: false,
        });
        self.current_index = 0;
        self.next_id = 1;
    }

    fn spawn_internal(
        &mut self,
        entry: TaskEntry,
        arg: *mut u8,
        stack_size: usize,
        is_idle: bool,
    ) -> Result<TaskId, &'static str> {
        if self.tasks.len() >= MAX_TASKS {
            return Err("task limit reached");
        }

        let stack_bytes = stack_size.max(MIN_STACK_SIZE);
        let mut stack = vec![0u8; stack_bytes].into_boxed_slice();
        let saved_esp = build_initial_frame(stack.as_mut()).ok_or("invalid task stack")?;

        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        self.tasks.push(Task {
            id,
            state: TaskState::Ready,
            wake_tick: 0,
            _stack: Some(stack),
            saved_esp,
            entry,
            arg,
            is_idle,
        });

        Ok(id)
    }

    fn wake_sleeping_tasks(&mut self) {
        let now = timer::ticks();
        for task in &mut self.tasks {
            if task.state == TaskState::Sleeping && tick_reached(now, task.wake_tick) {
                task.state = TaskState::Ready;
            }
        }
    }

    fn pick_next_task_index(&self, current: usize) -> usize {
        let total = self.tasks.len();
        if total == 0 {
            return current;
        }

        let mut idle_candidate = None;
        for step in 1..=total {
            let index = (current + step) % total;
            let task = &self.tasks[index];
            if !task.is_runnable() {
                continue;
            }
            if task.is_idle {
                if idle_candidate.is_none() {
                    idle_candidate = Some(index);
                }
                continue;
            }
            return index;
        }

        let current_task = &self.tasks[current];
        if current_task.is_runnable() && !current_task.is_idle {
            return current;
        }

        if let Some(index) = idle_candidate {
            return index;
        }

        if current_task.is_runnable() {
            return current;
        }

        for (index, task) in self.tasks.iter().enumerate() {
            if task.is_runnable() {
                return index;
            }
        }

        current
    }

    fn schedule(&mut self, frame: *mut InterruptFrame, cause: ScheduleCause) -> *mut InterruptFrame {
        if self.tasks.is_empty() {
            return frame;
        }

        let current = self.current_index.min(self.tasks.len() - 1);
        self.current_index = current;

        self.tasks[current].saved_esp = frame as u32;

        self.wake_sleeping_tasks();

        match cause {
            ScheduleCause::Timer | ScheduleCause::Yield => {
                if self.tasks[current].state == TaskState::Running {
                    self.tasks[current].state = TaskState::Ready;
                }
            }
            ScheduleCause::Sleep(ticks) => {
                let duration = ticks.max(1);
                self.tasks[current].state = TaskState::Sleeping;
                self.tasks[current].wake_tick = timer::ticks().wrapping_add(duration);
            }
        }

        let current = self.reap_exited_tasks(current);
        self.current_index = current;

        let next = self.pick_next_task_index(current);
        self.current_index = next;
        if self.tasks[next].state == TaskState::Ready {
            self.tasks[next].state = TaskState::Running;
        }

        let next_esp = self.tasks[next].saved_esp;
        if next_esp == 0 {
            return frame;
        }

        next_esp as *mut InterruptFrame
    }

    fn mark_current_exited(&mut self) {
        if self.current_index < self.tasks.len() {
            self.tasks[self.current_index].state = TaskState::Exited;
        }
    }

    fn reap_exited_tasks(&mut self, keep_index: usize) -> usize {
        let mut keep = keep_index.min(self.tasks.len().saturating_sub(1));
        let mut index = 0usize;
        while index < self.tasks.len() {
            if index != keep && self.tasks[index].state == TaskState::Exited {
                self.tasks.remove(index);
                if index < keep {
                    keep -= 1;
                }
                continue;
            }
            index += 1;
        }
        keep
    }
}

pub fn init() -> Result<(), &'static str> {
    with_interrupts_disabled(|| unsafe {
        let scheduler_slot = &mut *SCHEDULER.0.get();
        if scheduler_slot.is_some() {
            SCHEDULER_ONLINE.store(true, Ordering::Release);
            return Ok(());
        }

        let mut scheduler = Scheduler::new();
        scheduler.add_bootstrap_task();
        scheduler.spawn_internal(idle_task_entry, ptr::null_mut(), IDLE_STACK_SIZE, true)?;

        *scheduler_slot = Some(scheduler);
        SCHEDULER_ONLINE.store(true, Ordering::Release);
        Ok(())
    })
}

pub fn spawn_kernel(entry: TaskEntry, arg: *mut u8) -> Result<TaskId, &'static str> {
    spawn_kernel_with_stack(entry, arg, DEFAULT_STACK_SIZE)
}

pub fn spawn_kernel_with_stack(
    entry: TaskEntry,
    arg: *mut u8,
    stack_size: usize,
) -> Result<TaskId, &'static str> {
    with_interrupts_disabled(|| unsafe {
        let scheduler_slot = &mut *SCHEDULER.0.get();
        let Some(scheduler) = scheduler_slot.as_mut() else {
            return Err("scheduler not initialized");
        };
        scheduler.spawn_internal(entry, arg, stack_size, false)
    })
}

pub fn on_timer_interrupt(frame: *mut InterruptFrame) -> *mut InterruptFrame {
    if !SCHEDULER_ONLINE.load(Ordering::Acquire) {
        return frame;
    }

    with_interrupts_disabled(|| unsafe {
        let scheduler_slot = &mut *SCHEDULER.0.get();
        let Some(scheduler) = scheduler_slot.as_mut() else {
            return frame;
        };
        scheduler.schedule(frame, ScheduleCause::Timer)
    })
}

pub fn handle_syscall(frame: *mut InterruptFrame) -> *mut InterruptFrame {
    let frame_ref = unsafe { &mut *frame };

    if !SCHEDULER_ONLINE.load(Ordering::Acquire) {
        frame_ref.eax = u32::MAX;
        return frame;
    }

    with_interrupts_disabled(|| unsafe {
        let scheduler_slot = &mut *SCHEDULER.0.get();
        let Some(scheduler) = scheduler_slot.as_mut() else {
            frame_ref.eax = u32::MAX;
            return frame;
        };

        match frame_ref.eax {
            SYSCALL_YIELD => {
                frame_ref.eax = 0;
                scheduler.schedule(frame, ScheduleCause::Yield)
            }
            SYSCALL_SLEEP => {
                let ticks = frame_ref.ebx.max(1);
                frame_ref.eax = 0;
                scheduler.schedule(frame, ScheduleCause::Sleep(ticks))
            }
            _ => {
                frame_ref.eax = u32::MAX;
                frame
            }
        }
    })
}

pub fn yield_now() {
    if !SCHEDULER_ONLINE.load(Ordering::Acquire) {
        return;
    }

    unsafe {
        asm!(
            "int 0x80",
            in("eax") SYSCALL_YIELD,
            lateout("eax") _,
        );
    }
}

pub fn sleep_ticks(ticks: u32) {
    if ticks == 0 {
        yield_now();
        return;
    }

    if !SCHEDULER_ONLINE.load(Ordering::Acquire) {
        busy_sleep_ticks(ticks);
        return;
    }

    unsafe {
        asm!(
            "int 0x80",
            in("eax") SYSCALL_SLEEP,
            in("ebx") ticks,
            lateout("eax") _,
        );
    }
}

pub fn current_task_id() -> Option<TaskId> {
    with_interrupts_disabled(|| unsafe {
        let scheduler_slot = &*SCHEDULER.0.get();
        let scheduler = scheduler_slot.as_ref()?;
        let task = scheduler.tasks.get(scheduler.current_index)?;
        Some(task.id)
    })
}

pub fn exit_current() -> ! {
    with_interrupts_disabled(|| unsafe {
        let scheduler_slot = &mut *SCHEDULER.0.get();
        if let Some(scheduler) = scheduler_slot.as_mut() {
            scheduler.mark_current_exited();
        }
    });

    yield_now();

    loop {
        unsafe {
            asm!("cli; hlt", options(nomem, nostack));
        }
    }
}

extern "C" fn task_entry_trampoline() -> ! {
    let (entry, arg) = with_interrupts_disabled(|| unsafe {
        let scheduler_slot = &*SCHEDULER.0.get();
        if let Some(scheduler) = scheduler_slot.as_ref() {
            if let Some(task) = scheduler.tasks.get(scheduler.current_index) {
                return (task.entry, task.arg);
            }
        }
        (idle_task_entry as TaskEntry, ptr::null_mut())
    });

    entry(arg);
    exit_current();
}

fn bootstrap_task_entry(_arg: *mut u8) {}

fn idle_task_entry(_arg: *mut u8) {
    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

fn build_initial_frame(stack: &mut [u8]) -> Option<u32> {
    let frame_size = core::mem::size_of::<InterruptFrame>();
    if stack.len() < frame_size + 16 {
        return None;
    }

    let stack_start = stack.as_ptr() as usize;
    let stack_top = stack_start.checked_add(stack.len())?;
    let frame_addr = stack_top.checked_sub(frame_size)? & !0x0F;
    if frame_addr < stack_start {
        return None;
    }

    let frame = frame_addr as *mut InterruptFrame;
    unsafe {
        frame.write(InterruptFrame {
            gs: KERNEL_DATA_SELECTOR,
            fs: KERNEL_DATA_SELECTOR,
            es: KERNEL_DATA_SELECTOR,
            ds: KERNEL_DATA_SELECTOR,
            edi: 0,
            esi: 0,
            ebp: 0,
            esp: 0,
            ebx: 0,
            edx: 0,
            ecx: 0,
            eax: 0,
            int_no: 0,
            err_code: 0,
            eip: task_entry_trampoline as *const () as usize as u32,
            cs: KERNEL_CODE_SELECTOR,
            eflags: INITIAL_EFLAGS,
        });
    }

    Some(frame_addr as u32)
}

fn busy_sleep_ticks(ticks: u32) {
    let wake_tick = timer::ticks().wrapping_add(ticks.max(1));
    while !tick_reached(timer::ticks(), wake_tick) {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

#[inline]
fn tick_reached(now: u32, target: u32) -> bool {
    now.wrapping_sub(target) < (u32::MAX / 2)
}

#[inline]
fn with_interrupts_disabled<R>(f: impl FnOnce() -> R) -> R {
    let flags: u32;
    unsafe {
        asm!("pushfd", "pop {}", out(reg) flags, options(nomem));
        asm!("cli", options(nomem, nostack));
    }

    let result = f();

    if (flags & (1 << 9)) != 0 {
        unsafe {
            asm!("sti", options(nomem, nostack));
        }
    }

    result
}
