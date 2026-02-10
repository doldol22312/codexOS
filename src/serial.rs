use core::fmt::{self, Write};

use crate::io::{inb, outb};

const COM1: u16 = 0x3F8;

pub fn init() {
    unsafe {
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x80);
        outb(COM1 + 0, 0x03);
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03);
        outb(COM1 + 2, 0xC7);
        outb(COM1 + 4, 0x0B);
    }
}

#[inline]
fn transmitter_empty() -> bool {
    unsafe { (inb(COM1 + 5) & 0x20) != 0 }
}

#[inline]
fn data_ready() -> bool {
    unsafe { (inb(COM1 + 5) & 0x01) != 0 }
}

pub fn write_byte(byte: u8) {
    while !transmitter_empty() {}
    unsafe {
        outb(COM1, byte);
    }
}

pub fn write_str(s: &str) {
    for byte in s.bytes() {
        if byte == b'\n' {
            write_byte(b'\r');
        }
        write_byte(byte);
    }
}

pub fn read_byte() -> Option<u8> {
    if !data_ready() {
        return None;
    }
    Some(unsafe { inb(COM1) })
}

struct SerialWriter;

impl Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_str(s);
        Ok(())
    }
}

pub fn _print(args: fmt::Arguments<'_>) {
    let _ = SerialWriter.write_fmt(args);
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        $crate::serial::_print(format_args!($($arg)*));
    };
}

#[macro_export]
macro_rules! serial_println {
    () => {
        $crate::serial::_print(format_args!("\n"));
    };
    ($($arg:tt)*) => {
        $crate::serial::_print(format_args!($($arg)*));
        $crate::serial::_print(format_args!("\n"));
    };
}
