use core::fmt::{self, Write};

use crate::io::outb;

const BUFFER_WIDTH: usize = 80;
const BUFFER_HEIGHT: usize = 25;
const VGA_BUFFER: *mut u16 = 0xB8000 as *mut u16;
const DEFAULT_COLOR: u8 = 0x0F;
const VGA_CRTC_INDEX: u16 = 0x3D4;
const VGA_CRTC_DATA: u16 = 0x3D5;

static mut CURSOR_ROW: usize = 0;
static mut CURSOR_COL: usize = 0;

#[inline]
const fn vga_entry(ch: u8, color: u8) -> u16 {
    ((color as u16) << 8) | ch as u16
}

#[inline]
unsafe fn write_cell(row: usize, col: usize, value: u16) {
    core::ptr::write_volatile(VGA_BUFFER.add(row * BUFFER_WIDTH + col), value);
}

#[inline]
unsafe fn read_cell(row: usize, col: usize) -> u16 {
    core::ptr::read_volatile(VGA_BUFFER.add(row * BUFFER_WIDTH + col))
}

fn sync_hardware_cursor() {
    unsafe {
        let position = (CURSOR_ROW * BUFFER_WIDTH + CURSOR_COL) as u16;
        outb(VGA_CRTC_INDEX, 0x0F);
        outb(VGA_CRTC_DATA, (position & 0xFF) as u8);
        outb(VGA_CRTC_INDEX, 0x0E);
        outb(VGA_CRTC_DATA, (position >> 8) as u8);
    }
}

fn enable_hardware_cursor() {
    unsafe {
        outb(VGA_CRTC_INDEX, 0x0A);
        outb(VGA_CRTC_DATA, 0x0E);
        outb(VGA_CRTC_INDEX, 0x0B);
        outb(VGA_CRTC_DATA, 0x0F);
    }
}

pub fn clear_screen() {
    unsafe {
        for row in 0..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                write_cell(row, col, vga_entry(b' ', DEFAULT_COLOR));
            }
        }
        CURSOR_ROW = 0;
        CURSOR_COL = 0;
    }
    enable_hardware_cursor();
    sync_hardware_cursor();
}

pub fn scroll() {
    unsafe {
        for row in 1..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                let value = read_cell(row, col);
                write_cell(row - 1, col, value);
            }
        }

        for col in 0..BUFFER_WIDTH {
            write_cell(BUFFER_HEIGHT - 1, col, vga_entry(b' ', DEFAULT_COLOR));
        }

        if CURSOR_ROW > 0 {
            CURSOR_ROW -= 1;
        }
    }
    sync_hardware_cursor();
}

pub fn backspace() {
    unsafe {
        if CURSOR_COL > 0 {
            CURSOR_COL -= 1;
        } else if CURSOR_ROW > 0 {
            CURSOR_ROW -= 1;
            CURSOR_COL = BUFFER_WIDTH - 1;
        } else {
            return;
        }

        write_cell(CURSOR_ROW, CURSOR_COL, vga_entry(b' ', DEFAULT_COLOR));
    }
    sync_hardware_cursor();
}

pub fn put_char(ch: char) {
    unsafe {
        match ch {
            '\n' => {
                CURSOR_COL = 0;
                CURSOR_ROW += 1;
            }
            '\r' => {
                CURSOR_COL = 0;
            }
            '\x08' => {
                backspace();
                return;
            }
            _ => {
                write_cell(CURSOR_ROW, CURSOR_COL, vga_entry(ch as u8, DEFAULT_COLOR));
                CURSOR_COL += 1;
                if CURSOR_COL >= BUFFER_WIDTH {
                    CURSOR_COL = 0;
                    CURSOR_ROW += 1;
                }
            }
        }

        if CURSOR_ROW >= BUFFER_HEIGHT {
            scroll();
            return;
        }
    }
    sync_hardware_cursor();
}

pub fn print_str(s: &str) {
    for byte in s.bytes() {
        match byte {
            0x20..=0x7E | b'\n' | b'\r' | b'\x08' => put_char(byte as char),
            _ => put_char('?'),
        }
    }
}

pub fn print_u32(mut value: u32) {
    if value == 0 {
        put_char('0');
        return;
    }

    let mut digits = [0u8; 10];
    let mut idx = 0usize;

    while value > 0 {
        digits[idx] = (value % 10) as u8;
        value /= 10;
        idx += 1;
    }

    while idx > 0 {
        idx -= 1;
        put_char((digits[idx] + b'0') as char);
    }
}

struct VgaWriter;

impl Write for VgaWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        print_str(s);
        Ok(())
    }
}

pub fn _print(args: fmt::Arguments<'_>) {
    let _ = VgaWriter.write_fmt(args);
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::vga::_print(format_args!($($arg)*));
    };
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::vga::_print(format_args!("\n"));
    };
    ($($arg:tt)*) => {
        $crate::vga::_print(format_args!($($arg)*));
        $crate::vga::_print(format_args!("\n"));
    };
}
