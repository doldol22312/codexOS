use super::super::*;

unsafe extern "C" {
    static __heap_end: u8;
}

pub(super) fn print_date() {
    if let Some(now) = rtc::now() {
        shell_println!("{:04}-{:02}-{:02} (RTC)", now.year, now.month, now.day);
        return;
    }

    let up = timer::uptime();
    let days = (up.seconds / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    shell_println!(
        "{:04}-{:02}-{:02} (fallback: epoch + uptime)",
        year,
        month,
        day
    );
}

pub(super) fn print_time() {
    if let Some(now) = rtc::now() {
        shell_println!("{:02}:{:02}:{:02} (RTC)", now.hour, now.minute, now.second);
        return;
    }

    let up = timer::uptime();
    let seconds_of_day = up.seconds % 86_400;
    let hours = seconds_of_day / 3_600;
    let minutes = (seconds_of_day % 3_600) / 60;
    let seconds = seconds_of_day % 60;
    shell_println!(
        "{:02}:{:02}:{:02}.{:03} (fallback: since boot)",
        hours,
        minutes,
        seconds,
        up.millis
    );
}

pub(super) fn handle_rtc_command() {
    shell_println!(
        "rtc status: {}",
        if rtc::is_available() {
            "available"
        } else {
            "unavailable"
        }
    );

    if let Some(now) = rtc::now() {
        shell_println!(
            "rtc now: {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            now.year,
            now.month,
            now.day,
            now.hour,
            now.minute,
            now.second
        );
    } else {
        shell_println!("rtc read failed");
    }
}

pub(super) fn handle_paging_command() {
    let paging = paging::stats();
    shell_println!(
        "paging: {}",
        if paging.enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    shell_println!("page directory: {:#010x}", paging.directory_phys);
    shell_println!(
        "mapped: {} MiB ({} pages x {} KiB)",
        paging.mapped_bytes / (1024 * 1024),
        paging.mapped_regions,
        paging.page_size_bytes / 1024
    );
    shell_println!(
        "framebuffer: {}",
        if paging.framebuffer_mapped {
            "mapped"
        } else {
            "unmapped"
        }
    );
    if paging.framebuffer_mapped {
        shell_println!(
            "framebuffer virtual base: {:#010x}",
            paging.framebuffer_virtual
        );
        shell_println!("framebuffer bytes: {}", paging.framebuffer_bytes);
    }
}


pub(super) fn handle_mouse_command() {
    let state = mouse::state();

    let (width, height, status_height) =
        if let Some((fb_width, fb_height)) = vga::framebuffer_resolution() {
            (
                fb_width.min(i32::MAX as usize) as i32,
                fb_height.min(i32::MAX as usize) as i32,
                24i32,
            )
        } else {
            (
                vga::text_columns().min(i32::MAX as usize) as i32,
                vga::status_row().saturating_add(1).min(i32::MAX as usize) as i32,
                1i32,
            )
        };

    let terminal_height = height.saturating_sub(status_height).max(1);
    let status_band_height = height.saturating_sub(terminal_height).max(1);
    let regions = [
        input::HitRegion {
            id: 1,
            x: 0,
            y: 0,
            width,
            height: terminal_height,
        },
        input::HitRegion {
            id: 2,
            x: 0,
            y: terminal_height,
            width,
            height: status_band_height,
        },
    ];

    let hit_id = input::hit_test_id(&regions, state.x, state.y);
    let hit_index = input::hit_test_index(&regions, state.x, state.y);
    let hit_name = match hit_id {
        Some(1) => "terminal",
        Some(2) => "status",
        _ => "none",
    };

    shell_println!(
        "mouse x={} y={} left={} middle={} right={}",
        state.x,
        state.y,
        if state.left { 1 } else { 0 },
        if state.middle { 1 } else { 0 },
        if state.right { 1 } else { 0 }
    );
    shell_println!(
        "hit-test: {} (id={}, index={})",
        hit_name,
        hit_id.unwrap_or(0),
        hit_index.map(|value| value as i32).unwrap_or(-1)
    );
    shell_println!("input queue drops: {}", input::dropped_event_count());
}

pub(super) fn handle_memtest_command<'a, I>(mut parts: I)
where
    I: Iterator<Item = &'a str>,
{
    let requested = if let Some(token) = parts.next() {
        let Some(value) = parse_u32(token) else {
            shell_println!("invalid byte count: {}", token);
            return;
        };
        value as usize
    } else {
        4096
    };

    let result = allocator::memtest(requested);
    shell_println!("memtest start: {:#010x}", result.start);
    shell_println!("memtest bytes: {}", result.tested);

    if result.failures == 0 {
        shell_println!("memtest result: PASS");
    } else {
        shell_println!("memtest result: FAIL ({} mismatches)", result.failures);
        if let Some(address) = result.first_failure_addr {
            shell_println!("first failure: {:#010x}", address);
        }
    }
}

pub(super) fn handle_hexdump_command<'a, I>(mut parts: I)
where
    I: Iterator<Item = &'a str>,
{
    let Some(address_token) = parts.next() else {
        shell_println!("usage: hexdump <address> [length]");
        return;
    };

    let Some(address) = parse_u32(address_token) else {
        shell_println!("invalid address: {}", address_token);
        return;
    };

    let length = if let Some(length_token) = parts.next() {
        let Some(value) = parse_u32(length_token) else {
            shell_println!("invalid length: {}", length_token);
            return;
        };
        value as usize
    } else {
        128
    };

    if length == 0 {
        shell_println!("hexdump length must be > 0");
        return;
    }

    let start = address as usize;
    if !is_safe_dump_range(start, length) {
        shell_println!("hexdump range not allowed: {:#010x} len={}", start, length);
        shell_println!("allowed: kernel/heap or VGA memory");
        return;
    }

    for offset in (0..length).step_by(16) {
        let line_addr = start + offset;
        shell_print!("{:#010x}: ", line_addr);

        for column in 0..16 {
            let index = offset + column;
            if index < length {
                let byte = unsafe { ((start + index) as *const u8).read_volatile() };
                shell_print!("{:02x} ", byte);
            } else {
                shell_print!("   ");
            }
        }

        shell_print!("|");
        for column in 0..16 {
            let index = offset + column;
            if index < length {
                let byte = unsafe { ((start + index) as *const u8).read_volatile() };
                let ch = if byte.is_ascii_graphic() || byte == b' ' {
                    byte as char
                } else {
                    '.'
                };
                shell_print!("{}", ch);
            }
        }
        shell_println!("|");
    }
}

fn is_safe_dump_range(start: usize, length: usize) -> bool {
    let Some(end) = start.checked_add(length) else {
        return false;
    };

    let kernel_start = 0x0010_0000usize;
    let heap_end = core::ptr::addr_of!(__heap_end) as usize;
    let vga_start = 0x000B_8000usize;
    let vga_end = vga_start + 80 * 25 * 2;

    (start >= kernel_start && end <= heap_end) || (start >= vga_start && end <= vga_end)
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };

    (year as i32, month as u32, day as u32)
}

pub(super) fn handle_color_command<'a, I>(mut parts: I)
where
    I: Iterator<Item = &'a str>,
{
    let Some(first) = parts.next() else {
        shell_println!(
            "current color: fg={} bg={}",
            color_name(vga::foreground_color()),
            color_name(vga::background_color())
        );
        shell_println!("usage: color <fg> [bg]");
        shell_println!("       color list");
        return;
    };

    if first.eq_ignore_ascii_case("list") {
        shell_println!("0:black 1:blue 2:green 3:cyan 4:red 5:magenta 6:brown 7:light-gray");
        shell_println!("8:dark-gray 9:light-blue a:light-green b:light-cyan");
        shell_println!("c:light-red d:light-magenta e:yellow f:white");
        return;
    }

    let Some(foreground) = parse_color(first) else {
        shell_println!("invalid color: {}", first);
        return;
    };

    let background = if let Some(token) = parts.next() {
        let Some(parsed) = parse_color(token) else {
            shell_println!("invalid color: {}", token);
            return;
        };
        parsed
    } else {
        vga::background_color()
    };

    vga::set_color(foreground, background);
    shell_println!(
        "color set: fg={} bg={}",
        color_name(foreground),
        color_name(background)
    );
}

fn parse_color(token: &str) -> Option<u8> {
    if token.len() == 1 {
        return hex_value(token.as_bytes()[0]);
    }

    if token.len() == 3 {
        let bytes = token.as_bytes();
        if bytes[0] == b'0' && (bytes[1] == b'x' || bytes[1] == b'X') {
            return hex_value(bytes[2]);
        }
    }

    if eq_any(token, &["black"]) {
        Some(0x0)
    } else if eq_any(token, &["blue"]) {
        Some(0x1)
    } else if eq_any(token, &["green"]) {
        Some(0x2)
    } else if eq_any(token, &["cyan"]) {
        Some(0x3)
    } else if eq_any(token, &["red"]) {
        Some(0x4)
    } else if eq_any(token, &["magenta", "purple"]) {
        Some(0x5)
    } else if eq_any(token, &["brown"]) {
        Some(0x6)
    } else if eq_any(token, &["lightgray", "light-gray", "grey", "gray"]) {
        Some(0x7)
    } else if eq_any(token, &["darkgray", "dark-gray"]) {
        Some(0x8)
    } else if eq_any(token, &["lightblue", "light-blue"]) {
        Some(0x9)
    } else if eq_any(token, &["lightgreen", "light-green"]) {
        Some(0xA)
    } else if eq_any(token, &["lightcyan", "light-cyan"]) {
        Some(0xB)
    } else if eq_any(token, &["lightred", "light-red"]) {
        Some(0xC)
    } else if eq_any(token, &["lightmagenta", "light-magenta"]) {
        Some(0xD)
    } else if eq_any(token, &["yellow"]) {
        Some(0xE)
    } else if eq_any(token, &["white"]) {
        Some(0xF)
    } else {
        None
    }
}

fn eq_any(token: &str, names: &[&str]) -> bool {
    for name in names {
        if token.eq_ignore_ascii_case(name) {
            return true;
        }
    }
    false
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn color_name(color: u8) -> &'static str {
    match color & 0x0F {
        0x0 => "black",
        0x1 => "blue",
        0x2 => "green",
        0x3 => "cyan",
        0x4 => "red",
        0x5 => "magenta",
        0x6 => "brown",
        0x7 => "light-gray",
        0x8 => "dark-gray",
        0x9 => "light-blue",
        0xA => "light-green",
        0xB => "light-cyan",
        0xC => "light-red",
        0xD => "light-magenta",
        0xE => "yellow",
        _ => "white",
    }
}
