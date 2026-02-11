use crate::io::inb;
use core::sync::atomic::{AtomicU32, Ordering};

const BUFFER_SIZE: usize = 256;
const EVENT_UP: u8 = 0x80;
const EVENT_DOWN: u8 = 0x81;
const EVENT_LEFT: u8 = 0x82;
const EVENT_RIGHT: u8 = 0x83;
const EVENT_PAGE_UP: u8 = 0x84;
const EVENT_PAGE_DOWN: u8 = 0x85;

static mut BUFFER: [u8; BUFFER_SIZE] = [0; BUFFER_SIZE];
static mut HEAD: usize = 0;
static mut TAIL: usize = 0;
static mut SHIFT: bool = false;
static mut CAPS_LOCK: bool = false;
static mut EXTENDED: bool = false;
static KEY_ACTIVITY: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEvent {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
}

pub fn init() {}

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

        if EXTENDED {
            EXTENDED = false;
            if released {
                return;
            }
            mark_key_activity();

            match key {
                0x48 => {
                    push_byte(EVENT_UP);
                }
                0x50 => {
                    push_byte(EVENT_DOWN);
                }
                0x4B => {
                    push_byte(EVENT_LEFT);
                }
                0x4D => {
                    push_byte(EVENT_RIGHT);
                }
                0x49 => {
                    push_byte(EVENT_PAGE_UP);
                }
                0x51 => {
                    push_byte(EVENT_PAGE_DOWN);
                }
                _ => {}
            }
            return;
        }

        if !released {
            mark_key_activity();
        }

        match key {
            0x2A | 0x36 => {
                SHIFT = !released;
                return;
            }
            0x3A if !released => {
                CAPS_LOCK = !CAPS_LOCK;
                return;
            }
            _ => {}
        }

        if released {
            return;
        }

        if let Some(byte) = translate_scancode(key, SHIFT, CAPS_LOCK) {
            push_byte(byte);
        }
    }
}

#[inline]
fn mark_key_activity() {
    KEY_ACTIVITY.fetch_add(1, Ordering::Relaxed);
}

fn translate_scancode(scancode: u8, shift: bool, caps_lock: bool) -> Option<u8> {
    let byte = match scancode {
        0x02 => if shift { b'!' } else { b'1' },
        0x03 => if shift { b'@' } else { b'2' },
        0x04 => if shift { b'#' } else { b'3' },
        0x05 => if shift { b'$' } else { b'4' },
        0x06 => if shift { b'%' } else { b'5' },
        0x07 => if shift { b'^' } else { b'6' },
        0x08 => if shift { b'&' } else { b'7' },
        0x09 => if shift { b'*' } else { b'8' },
        0x0A => if shift { b'(' } else { b'9' },
        0x0B => if shift { b')' } else { b'0' },
        0x0C => if shift { b'_' } else { b'-' },
        0x0D => if shift { b'+' } else { b'=' },
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
        0x1A => if shift { b'{' } else { b'[' },
        0x1B => if shift { b'}' } else { b']' },
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
        0x27 => if shift { b':' } else { b';' },
        0x28 => if shift { b'"' } else { b'\'' },
        0x29 => if shift { b'~' } else { b'`' },
        0x2B => if shift { b'|' } else { b'\\' },
        0x2C => b'z',
        0x2D => b'x',
        0x2E => b'c',
        0x2F => b'v',
        0x30 => b'b',
        0x31 => b'n',
        0x32 => b'm',
        0x33 => if shift { b'<' } else { b',' },
        0x34 => if shift { b'>' } else { b'.' },
        0x35 => if shift { b'?' } else { b'/' },
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

    if (byte as char).is_ascii_lowercase() {
        let upper = shift ^ caps_lock;
        if upper {
            Some((byte as char).to_ascii_uppercase() as u8)
        } else {
            Some(byte)
        }
    } else {
        Some(byte)
    }
}

fn push_byte(byte: u8) {
    unsafe {
        let next = (HEAD + 1) % BUFFER_SIZE;
        if next != TAIL {
            BUFFER[HEAD] = byte;
            HEAD = next;
        }
    }
}

fn pop_byte() -> Option<u8> {
    unsafe {
        if HEAD == TAIL {
            None
        } else {
            let byte = BUFFER[TAIL];
            TAIL = (TAIL + 1) % BUFFER_SIZE;
            Some(byte)
        }
    }
}

pub fn read_key() -> Option<KeyEvent> {
    let byte = pop_byte()?;
    match byte {
        EVENT_UP => Some(KeyEvent::Up),
        EVENT_DOWN => Some(KeyEvent::Down),
        EVENT_LEFT => Some(KeyEvent::Left),
        EVENT_RIGHT => Some(KeyEvent::Right),
        EVENT_PAGE_UP => Some(KeyEvent::PageUp),
        EVENT_PAGE_DOWN => Some(KeyEvent::PageDown),
        _ if byte < 0x80 => Some(KeyEvent::Char(byte as char)),
        _ => None,
    }
}

pub fn key_activity() -> u32 {
    KEY_ACTIVITY.load(Ordering::Relaxed)
}
