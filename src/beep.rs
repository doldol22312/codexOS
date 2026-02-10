use crate::io::{inb, outb};
use crate::timer;

const PIT_BASE_FREQUENCY: u32 = 1_193_182;

pub fn play(frequency_hz: u32, duration_ms: u32) {
    let frequency = frequency_hz.max(1);
    let divisor = (PIT_BASE_FREQUENCY / frequency).clamp(1, 65_535);

    unsafe {
        outb(0x43, 0xB6);
        outb(0x42, (divisor & 0xFF) as u8);
        outb(0x42, ((divisor >> 8) & 0xFF) as u8);

        let speaker = inb(0x61);
        outb(0x61, speaker | 0x03);
    }

    sleep_ms(duration_ms.max(1));

    unsafe {
        let speaker = inb(0x61);
        outb(0x61, speaker & !0x03);
    }
}

pub fn startup_sound() {
    play(880, 60);
    sleep_ms(20);
    play(1320, 90);
}

fn sleep_ms(duration_ms: u32) {
    let hz = timer::frequency_hz().max(1) as u64;
    let wait_ticks = (duration_ms as u64 * hz).div_ceil(1000);
    let start = timer::ticks() as u64;

    while (timer::ticks() as u64).wrapping_sub(start) < wait_ticks {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
