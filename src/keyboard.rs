use crate::input::{self, InputEvent, KeyEvent};
use crate::io::inb;
use core::sync::atomic::{AtomicU32, Ordering};

static mut SHIFT_LEFT: bool = false;
static mut SHIFT_RIGHT: bool = false;
static mut CAPS_LOCK: bool = false;
static mut EXTENDED: bool = false;
static KEY_ACTIVITY: AtomicU32 = AtomicU32::new(0);

pub fn init() {
    unsafe {
        SHIFT_LEFT = false;
        SHIFT_RIGHT = false;
        CAPS_LOCK = false;
        EXTENDED = false;
    }
}

pub fn handle_interrupt() {
    let scancode = unsafe { inb(0x60) };
    process_scancode(scancode);
}

fn process_scancode(scancode: u8) {
    unsafe {
        if scancode == 0xE0 {
            EXTENDED = true;
            return;
        }

        let released = (scancode & 0x80) != 0;
        let key = scancode & 0x7F;
        let extended = EXTENDED;
        EXTENDED = false;

        if extended {
            if let Some(mapped) = translate_extended_scancode(key) {
                emit_key_event(mapped, released);
            }
            return;
        }

        match key {
            0x2A => {
                SHIFT_LEFT = !released;
                emit_key_event(KeyEvent::ShiftLeft, released);
                return;
            }
            0x36 => {
                SHIFT_RIGHT = !released;
                emit_key_event(KeyEvent::ShiftRight, released);
                return;
            }
            0x3A => {
                if !released {
                    CAPS_LOCK = !CAPS_LOCK;
                }
                emit_key_event(KeyEvent::CapsLock, released);
                return;
            }
            _ => {}
        }

        if let Some(mapped) = translate_scancode(key, shift_active(), CAPS_LOCK) {
            emit_key_event(mapped, released);
        }
    }
}

#[inline]
fn emit_key_event(key: KeyEvent, released: bool) {
    if !released {
        mark_key_activity();
        input::push_event(InputEvent::KeyPress { key });
    } else {
        input::push_event(InputEvent::KeyRelease { key });
    }
}

#[inline]
unsafe fn shift_active() -> bool {
    SHIFT_LEFT || SHIFT_RIGHT
}

#[inline]
fn mark_key_activity() {
    KEY_ACTIVITY.fetch_add(1, Ordering::Relaxed);
}

fn translate_extended_scancode(scancode: u8) -> Option<KeyEvent> {
    match scancode {
        0x48 => Some(KeyEvent::Up),
        0x50 => Some(KeyEvent::Down),
        0x4B => Some(KeyEvent::Left),
        0x4D => Some(KeyEvent::Right),
        0x49 => Some(KeyEvent::PageUp),
        0x51 => Some(KeyEvent::PageDown),
        _ => None,
    }
}

fn translate_scancode(scancode: u8, shift: bool, caps_lock: bool) -> Option<KeyEvent> {
    let byte = match scancode {
        0x02 => {
            if shift {
                b'!'
            } else {
                b'1'
            }
        }
        0x03 => {
            if shift {
                b'@'
            } else {
                b'2'
            }
        }
        0x04 => {
            if shift {
                b'#'
            } else {
                b'3'
            }
        }
        0x05 => {
            if shift {
                b'$'
            } else {
                b'4'
            }
        }
        0x06 => {
            if shift {
                b'%'
            } else {
                b'5'
            }
        }
        0x07 => {
            if shift {
                b'^'
            } else {
                b'6'
            }
        }
        0x08 => {
            if shift {
                b'&'
            } else {
                b'7'
            }
        }
        0x09 => {
            if shift {
                b'*'
            } else {
                b'8'
            }
        }
        0x0A => {
            if shift {
                b'('
            } else {
                b'9'
            }
        }
        0x0B => {
            if shift {
                b')'
            } else {
                b'0'
            }
        }
        0x0C => {
            if shift {
                b'_'
            } else {
                b'-'
            }
        }
        0x0D => {
            if shift {
                b'+'
            } else {
                b'='
            }
        }
        0x0E => 8,
        0x0F => b'\t',
        0x10 => b'q',
        0x11 => b'w',
        0x12 => b'e',
        0x13 => b'r',
        0x14 => b't',
        0x15 => b'y',
        0x16 => b'u',
        0x17 => b'i',
        0x18 => b'o',
        0x19 => b'p',
        0x1A => {
            if shift {
                b'{'
            } else {
                b'['
            }
        }
        0x1B => {
            if shift {
                b'}'
            } else {
                b']'
            }
        }
        0x1C => b'\n',
        0x1E => b'a',
        0x1F => b's',
        0x20 => b'd',
        0x21 => b'f',
        0x22 => b'g',
        0x23 => b'h',
        0x24 => b'j',
        0x25 => b'k',
        0x26 => b'l',
        0x27 => {
            if shift {
                b':'
            } else {
                b';'
            }
        }
        0x28 => {
            if shift {
                b'"'
            } else {
                b'\''
            }
        }
        0x29 => {
            if shift {
                b'~'
            } else {
                b'`'
            }
        }
        0x2B => {
            if shift {
                b'|'
            } else {
                b'\\'
            }
        }
        0x2C => b'z',
        0x2D => b'x',
        0x2E => b'c',
        0x2F => b'v',
        0x30 => b'b',
        0x31 => b'n',
        0x32 => b'm',
        0x33 => {
            if shift {
                b'<'
            } else {
                b','
            }
        }
        0x34 => {
            if shift {
                b'>'
            } else {
                b'.'
            }
        }
        0x35 => {
            if shift {
                b'?'
            } else {
                b'/'
            }
        }
        0x37 => b'*',
        0x39 => b' ',
        0x47 => b'7',
        0x48 => b'8',
        0x49 => b'9',
        0x4A => b'-',
        0x4B => b'4',
        0x4C => b'5',
        0x4D => b'6',
        0x4E => b'+',
        0x4F => b'1',
        0x50 => b'2',
        0x51 => b'3',
        0x52 => b'0',
        0x53 => b'.',
        _ => return None,
    };

    let translated = if (byte as char).is_ascii_lowercase() {
        if shift ^ caps_lock {
            (byte as char).to_ascii_uppercase() as u8
        } else {
            byte
        }
    } else {
        byte
    };

    Some(KeyEvent::Char(translated as char))
}

pub fn key_activity() -> u32 {
    KEY_ACTIVITY.load(Ordering::Relaxed)
}
