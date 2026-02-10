use core::sync::atomic::{AtomicU32, Ordering};

use crate::io::outb;

const PIT_BASE_FREQUENCY: u32 = 1_193_182;
const PIT_MIN_DIVISOR: u32 = 1;
const PIT_MAX_DIVISOR: u32 = 65_535;

static TICKS: AtomicU32 = AtomicU32::new(0);
static FREQUENCY_HZ: AtomicU32 = AtomicU32::new(100);

#[derive(Clone, Copy)]
pub struct Uptime {
    pub ticks: u32,
    pub hz: u32,
    pub seconds: u64,
    pub millis: u32,
}

pub fn init(hz: u32) {
    let requested_hz = hz.max(1);
    let divisor = (PIT_BASE_FREQUENCY / requested_hz).clamp(PIT_MIN_DIVISOR, PIT_MAX_DIVISOR);
    let actual_hz = PIT_BASE_FREQUENCY / divisor;

    FREQUENCY_HZ.store(actual_hz, Ordering::Relaxed);

    unsafe {
        outb(0x43, 0x36);
        outb(0x40, (divisor & 0xFF) as u8);
        outb(0x40, ((divisor >> 8) & 0xFF) as u8);
    }
}

pub fn handle_interrupt() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

pub fn ticks() -> u32 {
    TICKS.load(Ordering::Relaxed)
}

pub fn frequency_hz() -> u32 {
    FREQUENCY_HZ.load(Ordering::Relaxed)
}

pub fn uptime() -> Uptime {
    let ticks = ticks();
    let hz = frequency_hz().max(1);
    let total_millis = (ticks as u64) * 1000 / (hz as u64);

    Uptime {
        ticks,
        hz,
        seconds: total_millis / 1000,
        millis: (total_millis % 1000) as u32,
    }
}
