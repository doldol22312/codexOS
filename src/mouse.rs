use crate::io::{inb, outb};
use crate::vga;

const SCREEN_WIDTH: i32 = 80;
const SCREEN_HEIGHT: i32 = 25;
const STATUS_ROW: usize = 24;
const VGA_BUFFER: *mut u16 = 0xB8000 as *mut u16;

static mut PACKET: [u8; 3] = [0; 3];
static mut PACKET_INDEX: usize = 0;
static mut MOUSE_X: i32 = SCREEN_WIDTH / 2;
static mut MOUSE_Y: i32 = SCREEN_HEIGHT / 2;
static mut BUTTONS: u8 = 0;

#[derive(Clone, Copy)]
pub struct MouseState {
    pub x: i32,
    pub y: i32,
    pub left: bool,
    pub middle: bool,
    pub right: bool,
}

pub fn init() {
    flush_output_buffer();

    let _ = controller_write_command(0xA8);
    let _ = controller_write_command(0x20);

    let Some(mut command_byte) = controller_read_data() else {
        return;
    };

    command_byte |= 0x02;
    command_byte &= !0x20;

    let _ = controller_write_command(0x60);
    let _ = controller_write_data(command_byte);

    let _ = send_mouse_command(0xF6);
    let _ = send_mouse_command(0xF4);

    render_status_line();
}

pub fn handle_interrupt() {
    let byte = unsafe { inb(0x60) };

    unsafe {
        if PACKET_INDEX == 0 && (byte & 0x08) == 0 {
            return;
        }

        PACKET[PACKET_INDEX] = byte;
        PACKET_INDEX += 1;

        if PACKET_INDEX < 3 {
            return;
        }

        PACKET_INDEX = 0;

        let flags = PACKET[0];
        if (flags & 0xC0) != 0 {
            return;
        }

        let dx = PACKET[1] as i8 as i32;
        let dy = PACKET[2] as i8 as i32;

        let new_x = clamp_i32(MOUSE_X + dx, 0, SCREEN_WIDTH - 1);
        let new_y = clamp_i32(MOUSE_Y - dy, 0, SCREEN_HEIGHT - 1);

        MOUSE_X = new_x;
        MOUSE_Y = new_y;
        BUTTONS = flags & 0x07;
    }

    render_status_line();
}

pub fn state() -> MouseState {
    unsafe {
        MouseState {
            x: MOUSE_X,
            y: MOUSE_Y,
            left: (BUTTONS & 0x01) != 0,
            right: (BUTTONS & 0x02) != 0,
            middle: (BUTTONS & 0x04) != 0,
        }
    }
}

fn flush_output_buffer() {
    for _ in 0..32 {
        let status = unsafe { inb(0x64) };
        if (status & 0x01) == 0 {
            break;
        }
        let _ = unsafe { inb(0x60) };
    }
}

fn send_mouse_command(command: u8) -> bool {
    if !controller_write_command(0xD4) {
        return false;
    }
    if !controller_write_data(command) {
        return false;
    }
    matches!(controller_read_data(), Some(0xFA))
}

fn controller_write_command(command: u8) -> bool {
    if !wait_input_empty() {
        return false;
    }
    unsafe {
        outb(0x64, command);
    }
    true
}

fn controller_write_data(data: u8) -> bool {
    if !wait_input_empty() {
        return false;
    }
    unsafe {
        outb(0x60, data);
    }
    true
}

fn controller_read_data() -> Option<u8> {
    if !wait_output_full() {
        return None;
    }
    Some(unsafe { inb(0x60) })
}

fn wait_input_empty() -> bool {
    for _ in 0..100_000 {
        if (unsafe { inb(0x64) } & 0x02) == 0 {
            return true;
        }
    }
    false
}

fn wait_output_full() -> bool {
    for _ in 0..100_000 {
        if (unsafe { inb(0x64) } & 0x01) != 0 {
            return true;
        }
    }
    false
}

fn render_status_line() {
    let mouse = state();
    let color = vga::color_code();
    let mut line = [b' '; 80];
    let mut cursor = 0usize;

    cursor = append_text(&mut line, cursor, "mouse x=");
    cursor = append_u32(&mut line, cursor, mouse.x as u32);
    cursor = append_text(&mut line, cursor, " y=");
    cursor = append_u32(&mut line, cursor, mouse.y as u32);
    cursor = append_text(&mut line, cursor, " l=");
    cursor = append_bool(&mut line, cursor, mouse.left);
    cursor = append_text(&mut line, cursor, " m=");
    cursor = append_bool(&mut line, cursor, mouse.middle);
    cursor = append_text(&mut line, cursor, " r=");
    let _ = append_bool(&mut line, cursor, mouse.right);

    unsafe {
        for column in 0..80 {
            let value = ((color as u16) << 8) | line[column] as u16;
            core::ptr::write_volatile(VGA_BUFFER.add(STATUS_ROW * 80 + column), value);
        }
    }
}

fn append_text(buffer: &mut [u8; 80], mut cursor: usize, text: &str) -> usize {
    for byte in text.bytes() {
        if cursor >= buffer.len() {
            return cursor;
        }
        buffer[cursor] = byte;
        cursor += 1;
    }
    cursor
}

fn append_bool(buffer: &mut [u8; 80], cursor: usize, value: bool) -> usize {
    if value {
        append_text(buffer, cursor, "1")
    } else {
        append_text(buffer, cursor, "0")
    }
}

fn append_u32(buffer: &mut [u8; 80], mut cursor: usize, mut value: u32) -> usize {
    if cursor >= buffer.len() {
        return cursor;
    }

    if value == 0 {
        buffer[cursor] = b'0';
        return cursor + 1;
    }

    let mut digits = [0u8; 10];
    let mut count = 0usize;

    while value > 0 && count < digits.len() {
        digits[count] = (value % 10) as u8 + b'0';
        value /= 10;
        count += 1;
    }

    while count > 0 && cursor < buffer.len() {
        count -= 1;
        buffer[cursor] = digits[count];
        cursor += 1;
    }

    cursor
}

fn clamp_i32(value: i32, min: i32, max: i32) -> i32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}
