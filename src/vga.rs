use core::fmt::{self, Write};

use crate::io::outb;

const BUFFER_WIDTH: usize = 80;
const BUFFER_HEIGHT: usize = 25;
const TERMINAL_HEIGHT: usize = BUFFER_HEIGHT - 1;
const SCROLLBACK_ROWS: usize = 1024;
const VGA_BUFFER: *mut u16 = 0xB8000 as *mut u16;
const DEFAULT_COLOR: u8 = 0x0F;
const BLANK_CELL: u16 = ((DEFAULT_COLOR as u16) << 8) | b' ' as u16;
const VGA_CRTC_INDEX: u16 = 0x3D4;
const VGA_CRTC_DATA: u16 = 0x3D5;

static mut CURSOR_ROW: usize = 0;
static mut CURSOR_COL: usize = 0;
static mut CURRENT_COLOR: u8 = DEFAULT_COLOR;
static mut SCROLLBACK: [[u16; BUFFER_WIDTH]; SCROLLBACK_ROWS] =
    [[BLANK_CELL; BUFFER_WIDTH]; SCROLLBACK_ROWS];
static mut SCROLLBACK_HEAD: usize = 0;
static mut SCROLLBACK_COUNT: usize = 0;
static mut VIEW_OFFSET_ROWS: usize = 0;
static mut LIVE_SNAPSHOT: [[u16; BUFFER_WIDTH]; TERMINAL_HEIGHT] =
    [[BLANK_CELL; BUFFER_WIDTH]; TERMINAL_HEIGHT];
static mut LIVE_SNAPSHOT_VALID: bool = false;

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

#[inline]
unsafe fn scrollback_oldest_index() -> usize {
    (SCROLLBACK_HEAD + SCROLLBACK_ROWS - SCROLLBACK_COUNT) % SCROLLBACK_ROWS
}

#[inline]
unsafe fn push_scrollback_row_from_screen(row: usize) {
    let head = SCROLLBACK_HEAD;
    for col in 0..BUFFER_WIDTH {
        SCROLLBACK[head][col] = read_cell(row, col);
    }
    SCROLLBACK_HEAD = (head + 1) % SCROLLBACK_ROWS;
    if SCROLLBACK_COUNT < SCROLLBACK_ROWS {
        SCROLLBACK_COUNT += 1;
    }
}

#[inline]
unsafe fn snapshot_live_terminal() {
    for row in 0..TERMINAL_HEIGHT {
        for col in 0..BUFFER_WIDTH {
            LIVE_SNAPSHOT[row][col] = read_cell(row, col);
        }
    }
    LIVE_SNAPSHOT_VALID = true;
}

#[inline]
unsafe fn render_scrollback_view_locked() {
    if !LIVE_SNAPSHOT_VALID {
        snapshot_live_terminal();
    }

    let total_rows = SCROLLBACK_COUNT + TERMINAL_HEIGHT;
    let start_row = total_rows.saturating_sub(TERMINAL_HEIGHT + VIEW_OFFSET_ROWS);
    let oldest = scrollback_oldest_index();

    for screen_row in 0..TERMINAL_HEIGHT {
        let source_row = start_row + screen_row;
        if source_row < SCROLLBACK_COUNT {
            let history_row = (oldest + source_row) % SCROLLBACK_ROWS;
            for col in 0..BUFFER_WIDTH {
                write_cell(screen_row, col, SCROLLBACK[history_row][col]);
            }
        } else {
            let live_row = source_row - SCROLLBACK_COUNT;
            for col in 0..BUFFER_WIDTH {
                write_cell(screen_row, col, LIVE_SNAPSHOT[live_row][col]);
            }
        }
    }
}

#[inline]
unsafe fn restore_live_snapshot_locked() {
    if LIVE_SNAPSHOT_VALID {
        for row in 0..TERMINAL_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                write_cell(row, col, LIVE_SNAPSHOT[row][col]);
            }
        }
    }

    VIEW_OFFSET_ROWS = 0;
    LIVE_SNAPSHOT_VALID = false;
}

#[inline]
fn leave_scrollback_view_if_needed() {
    unsafe {
        if VIEW_OFFSET_ROWS == 0 && !LIVE_SNAPSHOT_VALID {
            return;
        }
        restore_live_snapshot_locked();
    }
    sync_hardware_cursor();
}

#[inline]
fn current_color() -> u8 {
    unsafe { CURRENT_COLOR }
}

pub fn color_code() -> u8 {
    current_color()
}

pub const fn status_row() -> usize {
    BUFFER_HEIGHT - 1
}

#[inline]
const fn max_terminal_cursor_index() -> usize {
    TERMINAL_HEIGHT * BUFFER_WIDTH - 1
}

pub fn move_cursor_left(count: usize) {
    leave_scrollback_view_if_needed();
    unsafe {
        let index = CURSOR_ROW * BUFFER_WIDTH + CURSOR_COL;
        let next = index.saturating_sub(count);
        CURSOR_ROW = next / BUFFER_WIDTH;
        CURSOR_COL = next % BUFFER_WIDTH;
    }
    sync_hardware_cursor();
}

pub fn move_cursor_right(count: usize) {
    leave_scrollback_view_if_needed();
    unsafe {
        let index = CURSOR_ROW * BUFFER_WIDTH + CURSOR_COL;
        let next = index.saturating_add(count).min(max_terminal_cursor_index());
        CURSOR_ROW = next / BUFFER_WIDTH;
        CURSOR_COL = next % BUFFER_WIDTH;
    }
    sync_hardware_cursor();
}

pub fn foreground_color() -> u8 {
    color_code() & 0x0F
}

pub fn background_color() -> u8 {
    (color_code() >> 4) & 0x0F
}

pub fn set_color(foreground: u8, background: u8) {
    unsafe {
        CURRENT_COLOR = ((background & 0x0F) << 4) | (foreground & 0x0F);
    }
}

pub fn set_color_code(color: u8) {
    unsafe {
        CURRENT_COLOR = color;
    }
}

pub fn clear_screen() {
    leave_scrollback_view_if_needed();
    unsafe {
        for row in 0..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                write_cell(row, col, vga_entry(b' ', current_color()));
            }
        }
        CURSOR_ROW = 0;
        CURSOR_COL = 0;
    }
    enable_hardware_cursor();
    sync_hardware_cursor();
}

pub fn scroll() {
    leave_scrollback_view_if_needed();
    unsafe {
        push_scrollback_row_from_screen(0);

        for row in 1..TERMINAL_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                let value = read_cell(row, col);
                write_cell(row - 1, col, value);
            }
        }

        for col in 0..BUFFER_WIDTH {
            write_cell(TERMINAL_HEIGHT - 1, col, vga_entry(b' ', current_color()));
        }

        if CURSOR_ROW > 0 {
            CURSOR_ROW -= 1;
        }
    }
    sync_hardware_cursor();
}

pub fn backspace() {
    leave_scrollback_view_if_needed();
    unsafe {
        if CURSOR_COL > 0 {
            CURSOR_COL -= 1;
        } else if CURSOR_ROW > 0 {
            CURSOR_ROW -= 1;
            CURSOR_COL = BUFFER_WIDTH - 1;
        } else {
            return;
        }

        write_cell(CURSOR_ROW, CURSOR_COL, vga_entry(b' ', current_color()));
    }
    sync_hardware_cursor();
}

pub fn put_char(ch: char) {
    leave_scrollback_view_if_needed();
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
                write_cell(CURSOR_ROW, CURSOR_COL, vga_entry(ch as u8, current_color()));
                CURSOR_COL += 1;
                if CURSOR_COL >= BUFFER_WIDTH {
                    CURSOR_COL = 0;
                    CURSOR_ROW += 1;
                }
            }
        }

        if CURSOR_ROW >= TERMINAL_HEIGHT {
            scroll();
            return;
        }
    }
    sync_hardware_cursor();
}

pub fn page_up() {
    unsafe {
        if SCROLLBACK_COUNT == 0 {
            return;
        }

        if VIEW_OFFSET_ROWS == 0 {
            snapshot_live_terminal();
        }

        let step = TERMINAL_HEIGHT;
        let max_offset = SCROLLBACK_COUNT;
        let next_offset = VIEW_OFFSET_ROWS.saturating_add(step).min(max_offset);
        if next_offset == VIEW_OFFSET_ROWS {
            return;
        }

        VIEW_OFFSET_ROWS = next_offset;
        render_scrollback_view_locked();
    }
    sync_hardware_cursor();
}

pub fn page_down() {
    unsafe {
        if VIEW_OFFSET_ROWS == 0 {
            return;
        }

        let step = TERMINAL_HEIGHT;
        if VIEW_OFFSET_ROWS <= step {
            restore_live_snapshot_locked();
        } else {
            VIEW_OFFSET_ROWS -= step;
            render_scrollback_view_locked();
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
