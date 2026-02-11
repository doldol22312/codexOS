use core::sync::atomic::{AtomicBool, Ordering};

use crate::io::{inb, io_wait, outb};

const CMOS_ADDRESS_PORT: u16 = 0x70;
const CMOS_DATA_PORT: u16 = 0x71;
const CMOS_NMI_DISABLE: u8 = 0x80;

const CMOS_SECONDS: u8 = 0x00;
const CMOS_MINUTES: u8 = 0x02;
const CMOS_HOURS: u8 = 0x04;
const CMOS_DAY: u8 = 0x07;
const CMOS_MONTH: u8 = 0x08;
const CMOS_YEAR: u8 = 0x09;
const CMOS_STATUS_A: u8 = 0x0A;
const CMOS_STATUS_B: u8 = 0x0B;
const CMOS_CENTURY: u8 = 0x32;

static AVAILABLE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RawRtc {
    second: u8,
    minute: u8,
    hour: u8,
    day: u8,
    month: u8,
    year: u8,
    century: u8,
    status_b: u8,
}

pub fn init() -> bool {
    let available = now_internal().is_some();
    AVAILABLE.store(available, Ordering::Relaxed);
    available
}

pub fn is_available() -> bool {
    AVAILABLE.load(Ordering::Relaxed)
}

pub fn now() -> Option<DateTime> {
    let result = now_internal();
    AVAILABLE.store(result.is_some(), Ordering::Relaxed);
    result
}

fn now_internal() -> Option<DateTime> {
    let raw = read_stable_rtc()?;
    convert_raw(raw)
}

fn read_stable_rtc() -> Option<RawRtc> {
    for _ in 0..5 {
        wait_for_update_to_finish()?;
        let first = unsafe { read_raw_rtc() };

        wait_for_update_to_finish()?;
        let second = unsafe { read_raw_rtc() };

        if first == second {
            return Some(second);
        }
    }

    None
}

fn wait_for_update_to_finish() -> Option<()> {
    for _ in 0..100_000 {
        if unsafe { !is_update_in_progress() } {
            return Some(());
        }
        core::hint::spin_loop();
    }

    None
}

unsafe fn read_raw_rtc() -> RawRtc {
    RawRtc {
        second: read_cmos(CMOS_SECONDS),
        minute: read_cmos(CMOS_MINUTES),
        hour: read_cmos(CMOS_HOURS),
        day: read_cmos(CMOS_DAY),
        month: read_cmos(CMOS_MONTH),
        year: read_cmos(CMOS_YEAR),
        century: read_cmos(CMOS_CENTURY),
        status_b: read_cmos(CMOS_STATUS_B),
    }
}

unsafe fn is_update_in_progress() -> bool {
    (read_cmos(CMOS_STATUS_A) & 0x80) != 0
}

unsafe fn read_cmos(register: u8) -> u8 {
    outb(CMOS_ADDRESS_PORT, CMOS_NMI_DISABLE | register);
    io_wait();
    inb(CMOS_DATA_PORT)
}

fn convert_raw(raw: RawRtc) -> Option<DateTime> {
    let mut second = raw.second;
    let mut minute = raw.minute;
    let mut hour = raw.hour;
    let mut day = raw.day;
    let mut month = raw.month;
    let mut year = raw.year;
    let mut century = raw.century;

    let binary_mode = (raw.status_b & 0x04) != 0;
    let is_24_hour = (raw.status_b & 0x02) != 0;
    let hour_is_pm = (hour & 0x80) != 0;

    hour &= 0x7F;

    if !binary_mode {
        second = bcd_to_binary(second);
        minute = bcd_to_binary(minute);
        hour = bcd_to_binary(hour);
        day = bcd_to_binary(day);
        month = bcd_to_binary(month);
        year = bcd_to_binary(year);
        if century != 0 {
            century = bcd_to_binary(century);
        }
    }

    if !is_24_hour {
        if hour_is_pm {
            if hour < 12 {
                hour = hour.saturating_add(12);
            }
        } else if hour == 12 {
            hour = 0;
        }
    }

    let full_year = if century != 0 {
        (century as u16) * 100 + (year as u16)
    } else {
        2000 + (year as u16)
    };

    if !is_valid_datetime(full_year, month, day, hour, minute, second) {
        return None;
    }

    Some(DateTime {
        year: full_year,
        month,
        day,
        hour,
        minute,
        second,
    })
}

fn bcd_to_binary(value: u8) -> u8 {
    ((value >> 4) * 10) + (value & 0x0F)
}

fn is_valid_datetime(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> bool {
    if !(1..=12).contains(&month) {
        return false;
    }

    if hour > 23 || minute > 59 || second > 59 {
        return false;
    }

    let max_day = days_in_month(year, month);
    day >= 1 && day <= max_day
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}
