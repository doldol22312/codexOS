use core::str;

use crate::{
    allocator,
    ata,
    fs,
    keyboard::{self, KeyEvent},
    matrix,
    mouse,
    paging,
    print,
    println,
    reboot,
    rtc,
    serial,
    shutdown,
    timer,
    vga,
};

const MAX_LINE: usize = 256;
const HISTORY_SIZE: usize = 32;

unsafe extern "C" {
    static __heap_end: u8;
}

macro_rules! shell_print {
    ($($arg:tt)*) => {{
        print!($($arg)*);
        $crate::serial_print!($($arg)*);
    }};
}

macro_rules! shell_println {
    () => {{
        println!();
        $crate::serial_println!();
    }};
    ($($arg:tt)*) => {{
        println!($($arg)*);
        $crate::serial_println!($($arg)*);
    }};
}

struct History {
    entries: [[u8; MAX_LINE]; HISTORY_SIZE],
    lengths: [usize; HISTORY_SIZE],
    head: usize,
    count: usize,
}

impl History {
    fn new() -> Self {
        Self {
            entries: [[0; MAX_LINE]; HISTORY_SIZE],
            lengths: [0; HISTORY_SIZE],
            head: 0,
            count: 0,
        }
    }

    fn push(&mut self, line: &[u8]) {
        if line.is_empty() {
            return;
        }

        if self.latest().is_some_and(|previous| previous == line) {
            return;
        }

        let index = self.head;
        let copy_len = line.len().min(MAX_LINE);
        self.entries[index][..copy_len].copy_from_slice(&line[..copy_len]);
        self.lengths[index] = copy_len;
        self.head = (self.head + 1) % HISTORY_SIZE;

        if self.count < HISTORY_SIZE {
            self.count += 1;
        }
    }

    fn latest(&self) -> Option<&[u8]> {
        if self.count == 0 {
            return None;
        }
        Some(self.get(self.count - 1))
    }

    fn get(&self, logical_index: usize) -> &[u8] {
        let oldest_index = if self.count < HISTORY_SIZE { 0 } else { self.head };
        let physical_index = (oldest_index + logical_index) % HISTORY_SIZE;
        let len = self.lengths[physical_index];
        &self.entries[physical_index][..len]
    }
}

pub fn run() -> ! {
    let mut line = [0u8; MAX_LINE];
    let mut len = 0usize;
    let mut history = History::new();
    let mut history_cursor: Option<usize> = None;

    print_prompt();

    loop {
        if let Some(key) = read_input() {
            match key {
                KeyEvent::Char('\n') => {
                    shell_println!();
                    history.push(&line[..len]);
                    history_cursor = None;
                    execute_line(&line[..len]);
                    len = 0;
                    print_prompt();
                }
                KeyEvent::Char('\x08') => {
                    if len > 0 {
                        len -= 1;
                        erase_input_char();
                    }
                }
                KeyEvent::Char(ch) => {
                    if is_printable(ch) && len < MAX_LINE {
                        line[len] = ch as u8;
                        len += 1;
                        shell_print!("{}", ch);
                    }
                }
                KeyEvent::Up => {
                    if let Some(replacement) = navigate_history_up(&history, &mut history_cursor) {
                        set_input_line(&mut line, &mut len, replacement);
                    }
                }
                KeyEvent::Down => {
                    if let Some(replacement) =
                        navigate_history_down(&history, &mut history_cursor)
                    {
                        set_input_line(&mut line, &mut len, replacement);
                    }
                }
            }
        } else {
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
            }
        }
    }
}

#[derive(Clone, Copy)]
enum EscapeState {
    None,
    Esc,
    Csi,
}

fn read_input() -> Option<KeyEvent> {
    if let Some(key) = keyboard::read_key() {
        return Some(key);
    }

    static mut ESCAPE_STATE: EscapeState = EscapeState::None;
    let byte = serial::read_byte()?;
    let key = unsafe {
        match ESCAPE_STATE {
            EscapeState::None => match byte {
                0x1B => {
                    ESCAPE_STATE = EscapeState::Esc;
                    return None;
                }
                b'\r' | b'\n' => KeyEvent::Char('\n'),
                0x08 | 0x7F => KeyEvent::Char('\x08'),
                0x20..=0x7E => KeyEvent::Char(byte as char),
                _ => return None,
            },
            EscapeState::Esc => {
                if byte == b'[' {
                    ESCAPE_STATE = EscapeState::Csi;
                    return None;
                }
                ESCAPE_STATE = EscapeState::None;
                return None;
            }
            EscapeState::Csi => {
                ESCAPE_STATE = EscapeState::None;
                match byte {
                    b'A' => KeyEvent::Up,
                    b'B' => KeyEvent::Down,
                    _ => return None,
                }
            }
        }
    };

    Some(key)
}

fn print_prompt() {
    shell_print!("codexOS> ");
}

fn execute_line(bytes: &[u8]) {
    let line = str::from_utf8(bytes).unwrap_or("");
    let mut parts = line.split_whitespace();
    let Some(command) = parts.next() else {
        return;
    };

    match command {
        "help" => {
            shell_println!("Commands:");
            shell_println!("  help  - show this message");
            shell_println!("  clear - clear screen");
            shell_println!("  echo  - echo arguments");
            shell_println!("  info  - show system info");
            shell_println!("  disk  - show ATA disk info");
            shell_println!("  fsinfo - show filesystem status");
            shell_println!("  fsformat - format custom filesystem");
            shell_println!("  fsls  - list filesystem files");
            shell_println!("  fswrite <name> <text> - write a text file");
            shell_println!("  fscat <name> - read a text file");
            shell_println!("  date  - show date from RTC (fallback: uptime)");
            shell_println!("  time  - show time from RTC (fallback: uptime)");
            shell_println!("  rtc   - show RTC status and timestamp");
            shell_println!("  paging - show paging status");
            shell_println!("  uptime - show kernel uptime");
            shell_println!("  heap  - show heap usage");
            shell_println!("  memtest [bytes] - test free heap memory");
            shell_println!("  hexdump <addr> [len] - dump memory");
            shell_println!("  mouse - show mouse position/buttons");
            shell_println!("  matrix - matrix rain (press any key to exit)");
            shell_println!("  color - set text colors");
            shell_println!("  reboot - reboot machine");
            shell_println!("  shutdown - power off machine");
            shell_println!("  panic - trigger kernel panic");
            shell_println!("History: use Up/Down arrows");
        }
        "clear" => vga::clear_screen(),
        "echo" => {
            let mut first = true;
            for part in parts {
                if !first {
                    shell_print!(" ");
                }
                shell_print!("{}", part);
                first = false;
            }
            shell_println!();
        }
        "info" => {
            let up = timer::uptime();
            let paging_state = if paging::is_enabled() { "on" } else { "off" };
            let rtc_state = if rtc::is_available() {
                "present"
            } else {
                "unavailable"
            };
            let ata_state = if ata::is_present() { "present" } else { "missing" };
            let fs_state = if fs::is_mounted() {
                "mounted"
            } else {
                "unmounted"
            };
            shell_println!("codexOS barebones kernel");
            shell_println!("arch: x86 (32-bit)");
            shell_println!("lang: Rust + inline assembly");
            shell_println!("boot: custom BIOS bootloader");
            shell_println!(
                "features: VGA, IDT, IRQ keyboard, IRQ mouse, PIT, paging={}, ATA={}, FS={}, RTC={}, shell, free-list heap",
                paging_state,
                ata_state,
                fs_state,
                rtc_state
            );
            shell_println!("uptime: {}.{:03}s", up.seconds, up.millis);
        }
        "disk" => handle_disk_command(),
        "fsinfo" => handle_fsinfo_command(),
        "fsformat" => handle_fsformat_command(),
        "fsls" => handle_fsls_command(),
        "fswrite" => handle_fswrite_command(parts),
        "fscat" => handle_fscat_command(parts),
        "date" => print_date(),
        "time" => print_time(),
        "rtc" => handle_rtc_command(),
        "paging" => handle_paging_command(),
        "uptime" => {
            let up = timer::uptime();
            shell_println!(
                "uptime: {}.{:03}s (ticks={} @ {} Hz)",
                up.seconds, up.millis, up.ticks, up.hz
            );
        }
        "heap" => {
            let heap = allocator::stats();
            shell_println!("heap start: {:#010x}", heap.start);
            shell_println!("heap end:   {:#010x}", heap.end);
            shell_println!("heap total: {} bytes", heap.total);
            shell_println!("heap used:  {} bytes", heap.used);
            shell_println!("heap free:  {} bytes", heap.remaining);
        }
        "memtest" => handle_memtest_command(parts),
        "hexdump" => handle_hexdump_command(parts),
        "mouse" => {
            let state = mouse::state();
            shell_println!(
                "mouse x={} y={} left={} middle={} right={}",
                state.x,
                state.y,
                if state.left { 1 } else { 0 },
                if state.middle { 1 } else { 0 },
                if state.right { 1 } else { 0 }
            );
        }
        "matrix" => {
            shell_println!("matrix mode: press any key to return");
            matrix::run();
        }
        "color" => {
            handle_color_command(parts);
        }
        "reboot" => {
            shell_println!("Rebooting...");
            reboot::reboot();
        }
        "shutdown" => {
            shell_println!("Shutting down...");
            shutdown::shutdown();
        }
        "panic" => {
            panic!("panic command invoked from shell");
        }
        _ => {
            shell_println!("unknown command: {}", command);
        }
    }
}

fn navigate_history_up<'a>(history: &'a History, cursor: &mut Option<usize>) -> Option<&'a [u8]> {
    if history.count == 0 {
        return None;
    }

    let next_index = match *cursor {
        None => history.count - 1,
        Some(0) => 0,
        Some(index) => index - 1,
    };

    *cursor = Some(next_index);
    Some(history.get(next_index))
}

fn navigate_history_down<'a>(
    history: &'a History,
    cursor: &mut Option<usize>,
) -> Option<&'a [u8]> {
    const EMPTY: &[u8] = &[];

    let current = (*cursor)?;
    if current + 1 >= history.count {
        *cursor = None;
        return Some(EMPTY);
    }

    let next_index = current + 1;
    *cursor = Some(next_index);
    Some(history.get(next_index))
}

fn set_input_line(line: &mut [u8; MAX_LINE], len: &mut usize, replacement: &[u8]) {
    while *len > 0 {
        *len -= 1;
        erase_input_char();
    }

    let copy_len = replacement.len().min(MAX_LINE);
    for index in 0..copy_len {
        let byte = replacement[index];
        line[index] = byte;
        *len += 1;
        shell_print!("{}", byte as char);
    }
}

fn erase_input_char() {
    vga::backspace();
    serial::write_str("\x08 \x08");
}

fn print_date() {
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

fn print_time() {
    if let Some(now) = rtc::now() {
        shell_println!(
            "{:02}:{:02}:{:02} (RTC)",
            now.hour,
            now.minute,
            now.second
        );
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

fn handle_rtc_command() {
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

fn handle_paging_command() {
    let paging = paging::stats();
    shell_println!(
        "paging: {}",
        if paging.enabled { "enabled" } else { "disabled" }
    );
    shell_println!("page directory: {:#010x}", paging.directory_phys);
    shell_println!(
        "identity map: {} MiB ({} entries x {} MiB pages)",
        paging.mapped_bytes / (1024 * 1024),
        paging.mapped_regions,
        paging.page_size_bytes / (1024 * 1024)
    );
}

fn handle_disk_command() {
    let Some(info) = ata::info() else {
        shell_println!("ata disk: unavailable");
        return;
    };

    let model_end = info
        .model
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(info.model.len());
    let model = core::str::from_utf8(&info.model[..model_end]).unwrap_or("unknown");
    let mib = (info.sectors as u64 * info.sector_size as u64) / (1024 * 1024);

    shell_println!("ata disk: {}", if info.present { "present" } else { "missing" });
    shell_println!("model: {}", model);
    shell_println!(
        "capacity: {} sectors ({} bytes, {} MiB)",
        info.sectors,
        info.sectors as u64 * info.sector_size as u64,
        mib
    );
}

fn handle_fsinfo_command() {
    let info = fs::info();
    shell_println!(
        "filesystem: {}",
        if info.mounted { "mounted" } else { "unmounted" }
    );
    shell_println!(
        "disk: {}",
        if info.disk_present {
            "present"
        } else {
            "missing"
        }
    );
    if !info.mounted {
        shell_println!("hint: run `fsformat` once to initialize");
        return;
    }

    shell_println!("total sectors: {}", info.total_sectors);
    shell_println!("directory sectors: {}", info.directory_sectors);
    shell_println!("next free lba: {}", info.next_free_lba);
    shell_println!("file count: {}", info.file_count);
    shell_println!("free sectors: {}", info.free_sectors);
}

fn handle_fsformat_command() {
    match fs::format() {
        Ok(()) => {
            shell_println!("filesystem formatted and mounted");
            handle_fsinfo_command();
        }
        Err(error) => shell_println!("fsformat failed: {}", error.as_str()),
    }
}

fn handle_fsls_command() {
    let mut files = [fs::FileInfo::empty(); 64];
    let listed = match fs::list(&mut files) {
        Ok(value) => value,
        Err(error) => {
            shell_println!("fsls failed: {}", error.as_str());
            return;
        }
    };

    if listed == 0 {
        shell_println!("filesystem is empty");
        return;
    }

    shell_println!("files ({}):", listed);
    for file in files.iter().take(listed) {
        shell_println!("  {} ({} bytes)", file.name_str(), file.size_bytes);
    }
}

fn handle_fswrite_command<'a, I>(mut parts: I)
where
    I: Iterator<Item = &'a str>,
{
    let Some(name) = parts.next() else {
        shell_println!("usage: fswrite <name> <text>");
        return;
    };

    let mut data = [0u8; 4096];
    let mut length = 0usize;
    let mut first = true;

    for part in parts {
        if !first {
            if length >= data.len() {
                shell_println!("fswrite failed: text too long (max {} bytes)", data.len());
                return;
            }
            data[length] = b' ';
            length += 1;
        }

        for byte in part.bytes() {
            if length >= data.len() {
                shell_println!("fswrite failed: text too long (max {} bytes)", data.len());
                return;
            }
            data[length] = byte;
            length += 1;
        }
        first = false;
    }

    match fs::write_file(name, &data[..length]) {
        Ok(()) => shell_println!("wrote {} bytes to {}", length, name),
        Err(error) => shell_println!("fswrite failed: {}", error.as_str()),
    }
}

fn handle_fscat_command<'a, I>(mut parts: I)
where
    I: Iterator<Item = &'a str>,
{
    let Some(name) = parts.next() else {
        shell_println!("usage: fscat <name>");
        return;
    };

    let mut buffer = [0u8; 4096];
    match fs::read_file(name, &mut buffer) {
        Ok(result) => {
            if result.total_size == 0 {
                shell_println!("{} is empty", name);
                return;
            }

            let text = core::str::from_utf8(&buffer[..result.copied_size]).unwrap_or("<binary>");
            shell_println!("{} ({} bytes):", name, result.total_size);
            shell_println!("{}", text);
        }
        Err(error) => shell_println!("fscat failed: {}", error.as_str()),
    }
}

fn handle_memtest_command<'a, I>(mut parts: I)
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

fn handle_hexdump_command<'a, I>(mut parts: I)
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

fn parse_u32(token: &str) -> Option<u32> {
    if token.is_empty() {
        return None;
    }

    let (digits, base) = if let Some(hex) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        (hex.as_bytes(), 16u32)
    } else {
        (token.as_bytes(), 10u32)
    };

    if digits.is_empty() {
        return None;
    }

    let mut value = 0u32;
    for digit in digits {
        let numeric = if base == 16 {
            hex_value(*digit)? as u32
        } else {
            match *digit {
                b'0'..=b'9' => (digit - b'0') as u32,
                _ => return None,
            }
        };

        value = value.checked_mul(base)?;
        value = value.checked_add(numeric)?;
    }

    Some(value)
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

fn handle_color_command<'a, I>(mut parts: I)
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

fn is_printable(ch: char) -> bool {
    ch >= ' ' && ch <= '~'
}
