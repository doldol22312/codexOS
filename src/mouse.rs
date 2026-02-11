use crate::input::{self, InputEvent, MouseButton};
use crate::io::{inb, outb};
use crate::vga;

static mut PACKET: [u8; 3] = [0; 3];
static mut PACKET_INDEX: usize = 0;
static mut MOUSE_X: i32 = 0;
static mut MOUSE_Y: i32 = 0;
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

    let (screen_width, screen_height) = screen_bounds();
    unsafe {
        MOUSE_X = screen_width / 2;
        MOUSE_Y = screen_height / 2;
        BUTTONS = 0;
        PACKET_INDEX = 0;
    }

    let state = state();
    vga::set_mouse_cursor(state.x, state.y, true);
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
        let (screen_width, screen_height) = screen_bounds();

        let old_x = MOUSE_X;
        let old_y = MOUSE_Y;
        let old_buttons = BUTTONS;

        let new_x = clamp_i32(old_x + dx, 0, screen_width - 1);
        let new_y = clamp_i32(old_y - dy, 0, screen_height - 1);
        let new_buttons = flags & 0x07;

        MOUSE_X = new_x;
        MOUSE_Y = new_y;
        BUTTONS = new_buttons;

        let moved_dx = new_x - old_x;
        let moved_dy = new_y - old_y;
        if moved_dx != 0 || moved_dy != 0 {
            input::push_event(InputEvent::MouseMove {
                x: new_x,
                y: new_y,
                dx: moved_dx,
                dy: moved_dy,
            });
        }

        emit_button_events(old_buttons, new_buttons, new_x, new_y);
    }

    let state = state();
    vga::set_mouse_cursor(state.x, state.y, true);
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

fn emit_button_events(old_buttons: u8, new_buttons: u8, x: i32, y: i32) {
    emit_button_event(old_buttons, new_buttons, 0x01, MouseButton::Left, x, y);
    emit_button_event(old_buttons, new_buttons, 0x02, MouseButton::Right, x, y);
    emit_button_event(old_buttons, new_buttons, 0x04, MouseButton::Middle, x, y);
}

fn emit_button_event(
    old_buttons: u8,
    new_buttons: u8,
    mask: u8,
    button: MouseButton,
    x: i32,
    y: i32,
) {
    let was_down = (old_buttons & mask) != 0;
    let is_down = (new_buttons & mask) != 0;

    if !was_down && is_down {
        input::push_event(InputEvent::MouseDown { button, x, y });
    } else if was_down && !is_down {
        input::push_event(InputEvent::MouseUp { button, x, y });
        input::push_event(InputEvent::MouseClick { button, x, y });
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

fn clamp_i32(value: i32, min: i32, max: i32) -> i32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

fn screen_bounds() -> (i32, i32) {
    if let Some((width, height)) = vga::framebuffer_resolution() {
        let width = width.max(1).min(i32::MAX as usize) as i32;
        let height = height.max(1).min(i32::MAX as usize) as i32;
        return (width, height);
    }

    let width = vga::text_columns().max(1).min(i32::MAX as usize) as i32;
    let height = vga::status_row()
        .saturating_add(1)
        .max(1)
        .min(i32::MAX as usize) as i32;
    (width, height)
}
