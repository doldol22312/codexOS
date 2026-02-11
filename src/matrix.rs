extern crate alloc;

use alloc::vec;

use crate::{keyboard, serial, timer, vga};

const TRAIL_LEN: i32 = 8;
const HEAD_COLOR: u8 = 0x0A;
const TRAIL_COLOR: u8 = 0x02;

pub fn run() {
    let saved_color = vga::color_code();
    vga::set_color(HEAD_COLOR, 0x00);
    vga::clear_screen();

    let width = vga::text_columns().max(1);
    let height = vga::text_rows().max(1);

    let mut rng = Rng::new(timer::ticks().wrapping_add(0x2F3B_9A1D));
    let mut heads = vec![-1i32; width];
    let mut speeds = vec![1u8; width];
    let mut cooldown = vec![0u8; width];

    for col in 0..width {
        speeds[col] = (rng.next() % 3 + 1) as u8;
        cooldown[col] = (rng.next() % 8) as u8;
    }

    let mut last_tick = timer::ticks();
    let mut key_activity_marker = keyboard::key_activity();
    loop {
        if exit_requested(&mut key_activity_marker) {
            break;
        }

        let now = timer::ticks();
        if now == last_tick {
            halt();
            continue;
        }

        let elapsed = now.wrapping_sub(last_tick).min(8);
        last_tick = now;

        for _ in 0..elapsed {
            update_frame(&mut heads, &mut speeds, &mut cooldown, width, height, &mut rng);
            if exit_requested(&mut key_activity_marker) {
                break;
            }
        }
    }

    vga::set_color_code(saved_color);
    vga::clear_screen();
}

fn update_frame(
    heads: &mut [i32],
    speeds: &mut [u8],
    cooldown: &mut [u8],
    width: usize,
    height: usize,
    rng: &mut Rng,
) {
    for col in 0..width {
        if cooldown[col] > 0 {
            cooldown[col] -= 1;
            continue;
        }

        cooldown[col] = speeds[col];

        if heads[col] < 0 {
            if rng.next() % 18 == 0 {
                heads[col] = 0;
            }
            continue;
        }

        let head_row = heads[col];

        if head_row >= 0 && head_row < height as i32 {
            write_cell(head_row as usize, col, random_char(rng), HEAD_COLOR);
        }

        let trail_row = head_row - 1;
        if trail_row >= 0 && trail_row < height as i32 {
            write_cell(trail_row as usize, col, random_char(rng), TRAIL_COLOR);
        }

        let clear_row = head_row - TRAIL_LEN;
        if clear_row >= 0 && clear_row < height as i32 {
            write_cell(clear_row as usize, col, b' ', 0x00);
        }

        heads[col] = head_row + 1;
        if heads[col] > height as i32 + TRAIL_LEN {
            heads[col] = -1;
            speeds[col] = (rng.next() % 3 + 1) as u8;
            cooldown[col] = (rng.next() % 16) as u8;
        } else if rng.next() % 128 == 0 {
            speeds[col] = (rng.next() % 3 + 1) as u8;
        }
    }

    vga::present();
}

#[inline]
fn exit_requested(activity_marker: &mut u32) -> bool {
    if keyboard::read_key().is_some() || serial::read_byte().is_some() {
        return true;
    }

    let current_activity = keyboard::key_activity();
    if current_activity != *activity_marker {
        *activity_marker = current_activity;
        return true;
    }

    false
}

#[inline]
fn random_char(rng: &mut Rng) -> u8 {
    (33 + (rng.next() % 94) as u8) as u8
}

#[inline]
fn write_cell(row: usize, col: usize, ch: u8, color: u8) {
    let color_code = color & 0x0F;
    vga::write_char_at(row, col, ch, color_code);
}

#[inline]
fn halt() {
    unsafe {
        core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
    }
}

struct Rng {
    state: u32,
}

impl Rng {
    fn new(seed: u32) -> Self {
        let state = if seed == 0 { 0xA341_316C } else { seed };
        Self { state }
    }

    fn next(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }
}
