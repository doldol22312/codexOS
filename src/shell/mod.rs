use core::str;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

extern crate alloc;
use alloc::vec;

use crate::{
    allocator, ata, discord, elf, fs, net, pci,
    input::{self, InputEvent, KeyEvent, MouseButton},
    keyboard, matrix, mouse, paging, print, println, reboot, rtc, serial, shutdown, sync, task,
    timer, ui, vga,
};

const MAX_LINE: usize = 256;
const HISTORY_SIZE: usize = 32;
const MAX_FS_COMPLETION_FILES: usize = 64;

const COMMANDS: [&str; 36] = [
    "help", "clear", "echo", "info", "disk", "fsinfo", "fsformat", "fsls", "fswrite", "fsdelete",
    "fscat", "edit", "elfrun", "date", "time", "rtc", "paging", "uptime", "heap", "memtest",
    "hexdump", "mouse", "netinfo", "discordcfg", "discorddiag", "matrix", "multdemo", "gfxdemo",
    "uidemo", "uidemo2", "windemo", "desktop", "color", "reboot", "shutdown", "panic",
];

macro_rules! shell_print {
    ($($arg:tt)*) => {{
        $crate::print!($($arg)*);
        $crate::serial_print!($($arg)*);
    }};
}

macro_rules! shell_println {
    () => {{
        $crate::println!();
        $crate::serial_println!();
    }};
    ($($arg:tt)*) => {{
        $crate::println!($($arg)*);
        $crate::serial_println!($($arg)*);
    }};
}

mod commands;
mod demos;
mod editor;

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
        let oldest_index = if self.count < HISTORY_SIZE {
            0
        } else {
            self.head
        };
        let physical_index = (oldest_index + logical_index) % HISTORY_SIZE;
        let len = self.lengths[physical_index];
        &self.entries[physical_index][..len]
    }
}

pub fn run() -> ! {
    let mut line = [0u8; MAX_LINE];
    let mut len = 0usize;
    let mut cursor = 0usize;
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
                    cursor = 0;
                    print_prompt();
                }
                KeyEvent::Char('\x08') => {
                    handle_backspace(&mut line, &mut len, &mut cursor);
                }
                KeyEvent::Char('\t') => {
                    handle_tab_completion(&mut line, &mut len, &mut cursor);
                }
                KeyEvent::Char(ch) => {
                    if is_printable(ch) {
                        insert_input_char(&mut line, &mut len, &mut cursor, ch);
                    }
                }
                KeyEvent::Up => {
                    if let Some(replacement) = navigate_history_up(&history, &mut history_cursor) {
                        set_input_line(&mut line, &mut len, &mut cursor, replacement);
                    }
                }
                KeyEvent::Down => {
                    if let Some(replacement) = navigate_history_down(&history, &mut history_cursor)
                    {
                        set_input_line(&mut line, &mut len, &mut cursor, replacement);
                    }
                }
                KeyEvent::Left => move_cursor_left_in_input(&mut cursor),
                KeyEvent::Right => move_cursor_right_in_input(len, &mut cursor),
                KeyEvent::PageUp => vga::page_up(),
                KeyEvent::PageDown => vga::page_down(),
                _ => {}
            }
        } else {
            vga::tick_cursor_blink();
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
    Csi5,
    Csi6,
}

fn read_input() -> Option<KeyEvent> {
    if let Some(key) = input::read_key_press() {
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
                b'\t' => KeyEvent::Char('\t'),
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
            EscapeState::Csi => match byte {
                b'A' => {
                    ESCAPE_STATE = EscapeState::None;
                    KeyEvent::Up
                }
                b'B' => {
                    ESCAPE_STATE = EscapeState::None;
                    KeyEvent::Down
                }
                b'C' => {
                    ESCAPE_STATE = EscapeState::None;
                    KeyEvent::Right
                }
                b'D' => {
                    ESCAPE_STATE = EscapeState::None;
                    KeyEvent::Left
                }
                b'5' => {
                    ESCAPE_STATE = EscapeState::Csi5;
                    return None;
                }
                b'6' => {
                    ESCAPE_STATE = EscapeState::Csi6;
                    return None;
                }
                _ => {
                    ESCAPE_STATE = EscapeState::None;
                    return None;
                }
            },
            EscapeState::Csi5 => {
                ESCAPE_STATE = EscapeState::None;
                match byte {
                    b'~' => KeyEvent::PageUp,
                    _ => return None,
                }
            }
            EscapeState::Csi6 => {
                ESCAPE_STATE = EscapeState::None;
                match byte {
                    b'~' => KeyEvent::PageDown,
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
            shell_println!("  fsdelete <name> - delete a file");
            shell_println!("  fscat <name> - read a text file");
            shell_println!("  edit <name> - simple line editor for a file");
            shell_println!("  elfrun <name> - load and run an ELF32 userspace process (foreground)");
            shell_println!("  date  - show date from RTC (fallback: uptime)");
            shell_println!("  time  - show time from RTC (fallback: uptime)");
            shell_println!("  rtc   - show RTC status and timestamp");
            shell_println!("  paging - show paging status");
            shell_println!("  uptime - show kernel uptime");
            shell_println!("  heap  - show heap usage");
            shell_println!("  memtest [bytes] - test free heap memory");
            shell_println!("  hexdump <addr> [len] - dump memory");
            shell_println!("  mouse - show mouse position/buttons");
            shell_println!("  netinfo - show PCI/network status");
            shell_println!("  discordcfg - inspect discord.cfg bot settings");
            shell_println!("  discorddiag - run Discord client diagnostics");
            shell_println!("  matrix - matrix rain (press any key to exit)");
            shell_println!("  multdemo [bench [iters]] - graphical multitasking windows demo");
            shell_println!("  gfxdemo - draw framebuffer primitives demo");
            shell_println!("  uidemo - UI dispatcher/widgets demo");
            shell_println!("  uidemo2 - advanced widget showcase");
            shell_println!("  windemo - multi-window compositor demo");
            shell_println!("  desktop - desktop environment shell demo");
            shell_println!("  color - set text colors");
            shell_println!("  reboot - reboot machine");
            shell_println!("  shutdown - power off machine");
            shell_println!("  panic - trigger kernel panic");
            shell_println!("Editing: Up/Down history, Left/Right move cursor, Tab complete");
            shell_println!("View: PageUp/PageDown scroll output");
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
            let ata_state = if ata::is_present() {
                "present"
            } else {
                "missing"
            };
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
                "features: VBE+framebuffer text, IDT, IRQ keyboard, IRQ mouse, PIT, paging={}, ATA={}, FS={}, RTC={}, shell, free-list heap",
                paging_state,
                ata_state,
                fs_state,
                rtc_state
            );
            shell_println!(
                "text grid: {}x{} (+1 status row)",
                vga::text_columns(),
                vga::text_rows()
            );
            shell_println!("uptime: {}.{:03}s", up.seconds, up.millis);
        }
        "disk" => commands::handle_disk_command(),
        "fsinfo" => commands::handle_fsinfo_command(),
        "fsformat" => commands::handle_fsformat_command(),
        "fsls" => commands::handle_fsls_command(),
        "fswrite" => commands::handle_fswrite_command(parts),
        "fsdelete" => commands::handle_fsdelete_command(parts),
        "fscat" => commands::handle_fscat_command(parts),
        "edit" => editor::handle_edit_command(parts),
        "elfrun" => commands::handle_elfrun_command(parts),
        "date" => commands::print_date(),
        "time" => commands::print_time(),
        "rtc" => commands::handle_rtc_command(),
        "paging" => commands::handle_paging_command(),
        "uptime" => {
            let up = timer::uptime();
            shell_println!(
                "uptime: {}.{:03}s (ticks={} @ {} Hz)",
                up.seconds,
                up.millis,
                up.ticks,
                up.hz
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
        "memtest" => commands::handle_memtest_command(parts),
        "hexdump" => commands::handle_hexdump_command(parts),
        "mouse" => commands::handle_mouse_command(),
        "netinfo" => commands::handle_netinfo_command(),
        "discordcfg" => commands::handle_discordcfg_command(),
        "discorddiag" => commands::handle_discorddiag_command(),
        "matrix" => {
            shell_println!("matrix mode: press any key to return");
            matrix::run();
        }
        "multdemo" => {
            demos::handle_multdemo_command(parts);
        }
        "gfxdemo" => {
            demos::handle_gfxdemo_command();
        }
        "uidemo" => {
            demos::handle_uidemo_command();
        }
        "uidemo2" => {
            demos::handle_uidemo2_command();
        }
        "windemo" => {
            demos::handle_windemo_command();
        }
        "desktop" => {
            demos::handle_desktop_command();
        }
        "color" => {
            commands::handle_color_command(parts);
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

fn navigate_history_down<'a>(history: &'a History, cursor: &mut Option<usize>) -> Option<&'a [u8]> {
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

fn set_input_line(
    line: &mut [u8; MAX_LINE],
    len: &mut usize,
    cursor: &mut usize,
    replacement: &[u8],
) {
    clear_input_line(*len, *cursor);

    let copy_len = replacement.len().min(MAX_LINE);
    line[..copy_len].copy_from_slice(&replacement[..copy_len]);
    *len = copy_len;
    *cursor = copy_len;

    for byte in line.iter().take(copy_len) {
        shell_print!("{}", *byte as char);
    }
}

fn insert_input_char(line: &mut [u8; MAX_LINE], len: &mut usize, cursor: &mut usize, ch: char) {
    if *len >= MAX_LINE {
        return;
    }

    let byte = ch as u8;

    if *cursor == *len {
        line[*cursor] = byte;
        *len += 1;
        *cursor += 1;
        shell_print!("{}", ch);
        return;
    }

    for index in (*cursor..*len).rev() {
        line[index + 1] = line[index];
    }

    let redraw_start = *cursor;
    line[redraw_start] = byte;
    *len += 1;
    *cursor += 1;

    for index in redraw_start..*len {
        shell_print!("{}", line[index] as char);
    }

    move_cursor_left_visual(*len - *cursor);
}

fn handle_backspace(line: &mut [u8; MAX_LINE], len: &mut usize, cursor: &mut usize) {
    if *cursor == 0 || *len == 0 {
        return;
    }

    let delete_index = *cursor - 1;
    for index in delete_index..(*len - 1) {
        line[index] = line[index + 1];
    }

    *len -= 1;
    *cursor -= 1;

    move_cursor_left_visual(1);

    for index in *cursor..*len {
        shell_print!("{}", line[index] as char);
    }
    shell_print!(" ");

    move_cursor_left_visual((*len - *cursor) + 1);
}

fn move_cursor_left_in_input(cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }

    *cursor -= 1;
    move_cursor_left_visual(1);
}

fn move_cursor_right_in_input(len: usize, cursor: &mut usize) {
    if *cursor >= len {
        return;
    }

    *cursor += 1;
    move_cursor_right_visual(1);
}

fn clear_input_line(len: usize, cursor: usize) {
    if cursor > 0 {
        move_cursor_left_visual(cursor);
    }

    for _ in 0..len {
        shell_print!(" ");
    }

    if len > 0 {
        move_cursor_left_visual(len);
    }
}

fn move_cursor_left_visual(steps: usize) {
    for _ in 0..steps {
        vga::move_cursor_left(1);
        serial::write_str("\x1b[D");
    }
}

fn move_cursor_right_visual(steps: usize) {
    for _ in 0..steps {
        vga::move_cursor_right(1);
        serial::write_str("\x1b[C");
    }
}

fn handle_tab_completion(line: &mut [u8; MAX_LINE], len: &mut usize, cursor: &mut usize) {
    if *cursor > *len {
        return;
    }

    let token_start = find_token_start(line, *cursor);
    let token_end = find_token_end(line, *len, token_start);
    let prefix = &line[token_start..*cursor];
    let token_index = word_index_at(line, token_start);
    let complete_command = token_index == 0;

    let old_len = *len;
    let old_cursor = *cursor;

    let mut matches = 0usize;
    let mut first = [0u8; MAX_LINE];
    let mut first_len = 0usize;
    let mut common = [0u8; MAX_LINE];
    let mut common_len = 0usize;

    if complete_command {
        for command in COMMANDS {
            update_completion_match(
                command.as_bytes(),
                prefix,
                &mut matches,
                &mut first,
                &mut first_len,
                &mut common,
                &mut common_len,
            );
        }
    } else {
        let mut files = [fs::FileInfo::empty(); MAX_FS_COMPLETION_FILES];
        if let Ok(count) = fs::list(&mut files) {
            for file in files.iter().take(count) {
                update_completion_match(
                    file.name_str().as_bytes(),
                    prefix,
                    &mut matches,
                    &mut first,
                    &mut first_len,
                    &mut common,
                    &mut common_len,
                );
            }
        }
    }

    if matches == 0 {
        return;
    }

    let (completion, completion_len) = if matches == 1 {
        (&first, first_len)
    } else {
        (&common, common_len)
    };

    let remove_len = token_end - token_start;
    let tail_len = old_len - token_end;
    let Some(new_len) = token_start
        .checked_add(completion_len)
        .and_then(|value| value.checked_add(tail_len))
    else {
        return;
    };

    if new_len > MAX_LINE {
        return;
    }

    if completion_len != remove_len {
        line.copy_within(token_end..old_len, token_start + completion_len);
    }
    line[token_start..token_start + completion_len].copy_from_slice(&completion[..completion_len]);

    *len = new_len;
    *cursor = token_start + completion_len;

    let can_append_space = matches == 1
        && old_cursor == token_end
        && token_end == old_len
        && new_len < MAX_LINE
        && (*len == 0 || line[*len - 1] != b' ');

    if can_append_space {
        line[*len] = b' ';
        *len += 1;
        *cursor += 1;
    }
    redraw_input_line(line, old_len, old_cursor, *len, *cursor);
}

fn update_completion_match(
    candidate: &[u8],
    prefix: &[u8],
    matches: &mut usize,
    first: &mut [u8; MAX_LINE],
    first_len: &mut usize,
    common: &mut [u8; MAX_LINE],
    common_len: &mut usize,
) {
    if candidate.len() > MAX_LINE || !candidate.starts_with(prefix) {
        return;
    }

    if *matches == 0 {
        *first_len = candidate.len();
        first[..*first_len].copy_from_slice(candidate);
        *common_len = *first_len;
        common[..*common_len].copy_from_slice(candidate);
    } else {
        *common_len = common_prefix_len(&common[..*common_len], candidate);
    }

    *matches += 1;
}

fn common_prefix_len(left: &[u8], right: &[u8]) -> usize {
    let limit = left.len().min(right.len());
    for index in 0..limit {
        if left[index] != right[index] {
            return index;
        }
    }
    limit
}

fn find_token_start(line: &[u8; MAX_LINE], cursor: usize) -> usize {
    let mut index = cursor;
    while index > 0 && !is_whitespace_byte(line[index - 1]) {
        index -= 1;
    }
    index
}

fn find_token_end(line: &[u8; MAX_LINE], len: usize, start: usize) -> usize {
    let mut index = start;
    while index < len && !is_whitespace_byte(line[index]) {
        index += 1;
    }
    index
}

fn word_index_at(line: &[u8; MAX_LINE], token_start: usize) -> usize {
    let mut index = 0usize;
    let mut cursor = 0usize;

    while cursor < token_start {
        while cursor < token_start && is_whitespace_byte(line[cursor]) {
            cursor += 1;
        }

        if cursor >= token_start {
            break;
        }

        while cursor < token_start && !is_whitespace_byte(line[cursor]) {
            cursor += 1;
        }
        index += 1;
    }

    index
}

fn is_whitespace_byte(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

fn redraw_input_line(
    line: &[u8; MAX_LINE],
    old_len: usize,
    old_cursor: usize,
    new_len: usize,
    new_cursor: usize,
) {
    clear_input_line(old_len, old_cursor);

    for byte in line.iter().take(new_len) {
        shell_print!("{}", *byte as char);
    }

    move_cursor_left_visual(new_len.saturating_sub(new_cursor));
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
fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
fn sanitize_editor_byte(byte: u8) -> u8 {
    match byte {
        b'\t' => b' ',
        0x20..=0x7E => byte,
        _ => b'?',
    }
}

fn is_printable(ch: char) -> bool {
    ch >= ' ' && ch <= '~'
}
