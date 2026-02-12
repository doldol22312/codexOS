#![allow(dead_code)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::arch::asm;
use core::cell::UnsafeCell;
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{gdt, interrupts::InterruptFrame, paging, timer};

pub type TaskId = u32;
pub type TaskEntry = fn(*mut u8);

const MAX_TASKS: usize = 16;
const DEFAULT_STACK_SIZE: usize = 64 * 1024;
const USER_KERNEL_STACK_SIZE: usize = 16 * 1024;
const IDLE_STACK_SIZE: usize = 16 * 1024;
const MIN_STACK_SIZE: usize = 4096;

const INITIAL_EFLAGS: u32 = 0x202;
const SYSCALL_WRITE_CHUNK: usize = 128;
const SYSCALL_WRITE_MAX: usize = 16 * 1024;

pub mod syscall {
    pub const EXIT: u32 = 0;
    pub const YIELD: u32 = 1;
    pub const SLEEP: u32 = 2;
    pub const WRITE: u32 = 3;

    pub const OK: u32 = 0;
    pub const ERR_UNSUPPORTED: u32 = u32::MAX;
    pub const ERR_INVALID: u32 = u32::MAX - 1;
}

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

enum TaskContext {
    Kernel { entry: TaskEntry, arg: *mut u8 },
    User,
}

enum TaskAddressSpace {
    Kernel,
    User(paging::AddressSpace),
}

struct Task {
    id: TaskId,
    state: TaskState,
    wake_tick: u32,
    _kernel_stack: Option<Box<[u8]>>,
    kernel_stack_top: u32,
    saved_esp: u32,
    context: TaskContext,
    address_space: TaskAddressSpace,
    is_idle: bool,
}

impl Task {
    fn is_runnable(&self) -> bool {
        matches!(self.state, TaskState::Ready | TaskState::Running)
    }

    fn cr3(&self) -> u32 {
        match &self.address_space {
            TaskAddressSpace::Kernel => paging::kernel_directory_phys() as u32,
            TaskAddressSpace::User(space) => space.cr3(),
        }
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
            _kernel_stack: None,
            kernel_stack_top: read_current_esp(),
            saved_esp: 0,
            context: TaskContext::Kernel {
                entry: bootstrap_task_entry,
                arg: ptr::null_mut(),
            },
            address_space: TaskAddressSpace::Kernel,
            is_idle: false,
        });
        self.current_index = 0;
        self.next_id = 1;
    }

    fn spawn_kernel_internal(
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
        let mut kernel_stack = vec![0u8; stack_bytes].into_boxed_slice();
        let saved_esp = build_initial_kernel_frame(kernel_stack.as_mut()).ok_or("invalid task stack")?;
        let kernel_stack_top = stack_top(kernel_stack.as_ref());

        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        self.tasks.push(Task {
            id,
            state: TaskState::Ready,
            wake_tick: 0,
            _kernel_stack: Some(kernel_stack),
            kernel_stack_top,
            saved_esp,
            context: TaskContext::Kernel { entry, arg },
            address_space: TaskAddressSpace::Kernel,
            is_idle,
        });

        Ok(id)
    }

    fn spawn_user_internal(
        &mut self,
        entry_point: u32,
        user_stack_top: u32,
        address_space: paging::AddressSpace,
    ) -> Result<TaskId, &'static str> {
        if self.tasks.len() >= MAX_TASKS {
            return Err("task limit reached");
        }

        let mut kernel_stack = vec![0u8; USER_KERNEL_STACK_SIZE].into_boxed_slice();
        let saved_esp = build_initial_user_frame(kernel_stack.as_mut(), entry_point, user_stack_top)
            .ok_or("invalid user task stack")?;
        let kernel_stack_top = stack_top(kernel_stack.as_ref());

        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        self.tasks.push(Task {
            id,
            state: TaskState::Ready,
            wake_tick: 0,
            _kernel_stack: Some(kernel_stack),
            kernel_stack_top,
            saved_esp,
            context: TaskContext::User,
            address_space: TaskAddressSpace::User(address_space),
            is_idle: false,
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

        let next_cr3 = self.tasks[next].cr3();
        let next_stack_top = self.tasks[next].kernel_stack_top;
        let next_esp = self.tasks[next].saved_esp;

        paging::switch_address_space(next_cr3);
        gdt::set_kernel_stack(next_stack_top);

        if next_esp == 0 {
            frame
        } else {
            next_esp as *mut InterruptFrame
        }
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
        scheduler.spawn_kernel_internal(idle_task_entry, ptr::null_mut(), IDLE_STACK_SIZE, true)?;

        gdt::set_kernel_stack(scheduler.tasks[scheduler.current_index].kernel_stack_top);
        paging::use_kernel_address_space();

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
        scheduler.spawn_kernel_internal(entry, arg, stack_size, false)
    })
}

pub fn spawn_user(
    entry_point: u32,
    user_stack_top: u32,
    address_space: paging::AddressSpace,
) -> Result<TaskId, &'static str> {
    with_interrupts_disabled(|| unsafe {
        let scheduler_slot = &mut *SCHEDULER.0.get();
        let Some(scheduler) = scheduler_slot.as_mut() else {
            return Err("scheduler not initialized");
        };
        scheduler.spawn_user_internal(entry_point, user_stack_top, address_space)
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
        frame_ref.eax = syscall::ERR_UNSUPPORTED;
        return frame;
    }

    let from_user = (frame_ref.cs & 0x3) == 0x3;

    with_interrupts_disabled(|| unsafe {
        let scheduler_slot = &mut *SCHEDULER.0.get();
        let Some(scheduler) = scheduler_slot.as_mut() else {
            frame_ref.eax = syscall::ERR_UNSUPPORTED;
            return frame;
        };

        match frame_ref.eax {
            syscall::EXIT => {
                frame_ref.eax = syscall::OK;
                scheduler.mark_current_exited();
                scheduler.schedule(frame, ScheduleCause::Yield)
            }
            syscall::YIELD => {
                frame_ref.eax = syscall::OK;
                scheduler.schedule(frame, ScheduleCause::Yield)
            }
            syscall::SLEEP => {
                let ticks = frame_ref.ebx.max(1);
                frame_ref.eax = syscall::OK;
                scheduler.schedule(frame, ScheduleCause::Sleep(ticks))
            }
            syscall::WRITE => {
                frame_ref.eax = match syscall_write(
                    frame_ref.ebx as usize,
                    frame_ref.ecx as usize,
                    from_user,
                ) {
                    Ok(written) => written as u32,
                    Err(code) => code,
                };
                frame
            }
            _ => {
                frame_ref.eax = syscall::ERR_UNSUPPORTED;
                frame
            }
        }
    })
}

pub fn handle_user_exception(frame: *mut InterruptFrame) -> Option<*mut InterruptFrame> {
    if !SCHEDULER_ONLINE.load(Ordering::Acquire) {
        return None;
    }

    Some(with_interrupts_disabled(|| unsafe {
        let scheduler_slot = &mut *SCHEDULER.0.get();
        let scheduler = scheduler_slot.as_mut()?;
        scheduler.mark_current_exited();
        Some(scheduler.schedule(frame, ScheduleCause::Yield))
    })?)
}

pub fn yield_now() {
    if !SCHEDULER_ONLINE.load(Ordering::Acquire) {
        return;
    }

    unsafe {
        asm!(
            "int 0x80",
            in("eax") syscall::YIELD,
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
            in("eax") syscall::SLEEP,
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

pub fn is_task_alive(task_id: TaskId) -> bool {
    with_interrupts_disabled(|| unsafe {
        let scheduler_slot = &*SCHEDULER.0.get();
        let Some(scheduler) = scheduler_slot.as_ref() else {
            return false;
        };

        scheduler
            .tasks
            .iter()
            .any(|task| task.id == task_id && task.state != TaskState::Exited)
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
                if let TaskContext::Kernel { entry, arg } = task.context {
                    return (entry, arg);
                }
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

#[repr(C)]
struct UserInterruptFrame {
    frame: InterruptFrame,
    user_esp: u32,
    user_ss: u32,
}

fn build_initial_kernel_frame(stack: &mut [u8]) -> Option<u32> {
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
            gs: gdt::kernel_data_selector() as u32,
            fs: gdt::kernel_data_selector() as u32,
            es: gdt::kernel_data_selector() as u32,
            ds: gdt::kernel_data_selector() as u32,
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
            cs: gdt::kernel_code_selector() as u32,
            eflags: INITIAL_EFLAGS,
        });
    }

    Some(frame_addr as u32)
}

fn build_initial_user_frame(stack: &mut [u8], entry_point: u32, user_stack_top: u32) -> Option<u32> {
    let frame_size = core::mem::size_of::<UserInterruptFrame>();
    if stack.len() < frame_size + 16 {
        return None;
    }

    let stack_start = stack.as_ptr() as usize;
    let stack_top = stack_start.checked_add(stack.len())?;
    let frame_addr = stack_top.checked_sub(frame_size)? & !0x0F;
    if frame_addr < stack_start {
        return None;
    }

    let frame = frame_addr as *mut UserInterruptFrame;
    unsafe {
        frame.write(UserInterruptFrame {
            frame: InterruptFrame {
                gs: gdt::user_data_selector() as u32,
                fs: gdt::user_data_selector() as u32,
                es: gdt::user_data_selector() as u32,
                ds: gdt::user_data_selector() as u32,
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
                eip: entry_point,
                cs: gdt::user_code_selector() as u32,
                eflags: INITIAL_EFLAGS,
            },
            user_esp: user_stack_top,
            user_ss: gdt::user_data_selector() as u32,
        });
    }

    Some(frame_addr as u32)
}

fn syscall_write(ptr: usize, len: usize, from_user: bool) -> Result<usize, u32> {
    if len == 0 {
        return Ok(0);
    }
    if len > SYSCALL_WRITE_MAX {
        return Err(syscall::ERR_INVALID);
    }

    if from_user {
        if !paging::is_user_accessible_range(ptr, len) {
            return Err(syscall::ERR_INVALID);
        }
    }

    let mut copied = 0usize;
    let mut scratch = [0u8; SYSCALL_WRITE_CHUNK];

    while copied < len {
        let chunk = (len - copied).min(scratch.len());
        unsafe {
            let src = (ptr + copied) as *const u8;
            core::ptr::copy_nonoverlapping(src, scratch.as_mut_ptr(), chunk);
        }

        if let Ok(text) = core::str::from_utf8(&scratch[..chunk]) {
            crate::print!("{}", text);
            crate::serial_print!("{}", text);
        } else {
            for byte in &scratch[..chunk] {
                let ch = match *byte {
                    b'\n' | b'\r' | b'\t' | 0x20..=0x7E => *byte as char,
                    _ => '?',
                };
                crate::print!("{}", ch);
                crate::serial_print!("{}", ch);
            }
        }

        copied += chunk;
    }

    Ok(copied)
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
fn stack_top(stack: &[u8]) -> u32 {
    (stack.as_ptr() as usize).saturating_add(stack.len()) as u32
}

#[inline]
fn read_current_esp() -> u32 {
    let esp: u32;
    unsafe {
        asm!("mov {}, esp", out(reg) esp, options(nomem, preserves_flags));
    }
    esp
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
