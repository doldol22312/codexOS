use core::arch::asm;
use core::sync::atomic::{AtomicU32, Ordering};

const QUEUE_SIZE: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEvent {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    ShiftLeft,
    ShiftRight,
    CapsLock,
    Unknown(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    MouseMove { x: i32, y: i32, dx: i32, dy: i32 },
    MouseDown { button: MouseButton, x: i32, y: i32 },
    MouseUp { button: MouseButton, x: i32, y: i32 },
    MouseClick { button: MouseButton, x: i32, y: i32 },
    KeyPress { key: KeyEvent },
    KeyRelease { key: KeyEvent },
}

const EMPTY_EVENT: InputEvent = InputEvent::KeyRelease {
    key: KeyEvent::Unknown(0),
};

static mut BUFFER: [InputEvent; QUEUE_SIZE] = [EMPTY_EVENT; QUEUE_SIZE];
static mut HEAD: usize = 0;
static mut TAIL: usize = 0;
static DROPPED_EVENTS: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitRegion {
    pub id: u16,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl HitRegion {
    #[inline]
    pub const fn contains(&self, px: i32, py: i32) -> bool {
        if self.width <= 0 || self.height <= 0 {
            return false;
        }

        let x1 = self.x.saturating_add(self.width);
        let y1 = self.y.saturating_add(self.height);
        px >= self.x && py >= self.y && px < x1 && py < y1
    }
}

pub fn init() {
    with_interrupts_disabled(|| unsafe {
        HEAD = 0;
        TAIL = 0;
    });
    DROPPED_EVENTS.store(0, Ordering::Relaxed);
}

pub fn push_event(event: InputEvent) {
    with_interrupts_disabled(|| unsafe {
        let next = (HEAD + 1) % QUEUE_SIZE;
        if next == TAIL {
            DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
            return;
        }

        BUFFER[HEAD] = event;
        HEAD = next;
    });
}

pub fn pop_event() -> Option<InputEvent> {
    with_interrupts_disabled(|| unsafe {
        if HEAD == TAIL {
            None
        } else {
            let event = BUFFER[TAIL];
            TAIL = (TAIL + 1) % QUEUE_SIZE;
            Some(event)
        }
    })
}

pub fn read_key_press() -> Option<KeyEvent> {
    for _ in 0..QUEUE_SIZE {
        let Some(event) = pop_event() else {
            return None;
        };
        if let InputEvent::KeyPress { key } = event {
            return Some(key);
        }
    }
    None
}

pub fn dropped_event_count() -> u32 {
    DROPPED_EVENTS.load(Ordering::Relaxed)
}

pub fn hit_test_index(regions: &[HitRegion], x: i32, y: i32) -> Option<usize> {
    for (index, region) in regions.iter().enumerate().rev() {
        if region.contains(x, y) {
            return Some(index);
        }
    }
    None
}

pub fn hit_test_id(regions: &[HitRegion], x: i32, y: i32) -> Option<u16> {
    hit_test_index(regions, x, y).map(|index| regions[index].id)
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
