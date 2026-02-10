use core::str;

use crate::{
    allocator,
    keyboard::{self, KeyEvent},
    print,
    println,
    reboot,
    serial,
    timer,
    vga,
};

const MAX_LINE: usize = 256;
const HISTORY_SIZE: usize = 32;

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
                        vga::backspace();
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
            shell_println!("  uptime - show kernel uptime");
            shell_println!("  heap  - show heap usage");
            shell_println!("  color - set text colors");
            shell_println!("  reboot - reboot machine");
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
            shell_println!("codexOS barebones kernel");
            shell_println!("arch: x86 (32-bit)");
            shell_println!("lang: Rust + inline assembly");
            shell_println!("boot: Multiboot/GRUB");
            shell_println!("features: VGA, IDT, IRQ keyboard, PIT, shell, heap");
            shell_println!("uptime: {}.{:03}s", up.seconds, up.millis);
        }
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
        "color" => {
            handle_color_command(parts);
        }
        "reboot" => {
            shell_println!("Rebooting...");
            reboot::reboot();
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
        vga::backspace();
    }

    let copy_len = replacement.len().min(MAX_LINE);
    for index in 0..copy_len {
        let byte = replacement[index];
        line[index] = byte;
        *len += 1;
        shell_print!("{}", byte as char);
    }
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
