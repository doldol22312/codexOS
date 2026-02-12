use core::str;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

extern crate alloc;
use alloc::vec;

use crate::{
    allocator, ata, fs,
    input::{self, InputEvent, KeyEvent, MouseButton},
    keyboard, matrix, mouse, paging, print, println, reboot, rtc, serial, shutdown, sync, task,
    timer, ui, vga,
};

const MAX_LINE: usize = 256;
const HISTORY_SIZE: usize = 32;
const MAX_FS_COMPLETION_FILES: usize = 64;
const EDITOR_MAX_LINES: usize = 128;
const EDITOR_MAX_LINE_LEN: usize = 200;
const EDITOR_MAX_BYTES: usize = 4096;
const UIDEMO_BUTTON_PING_ID: u16 = 10;
const UIDEMO_BUTTON_EXIT_ID: u16 = 11;
const UIDEMO2_TEXTBOX_ID: u16 = 200;
const UIDEMO2_TEXTAREA_ID: u16 = 201;
const UIDEMO2_CHECKBOX_ID: u16 = 202;
const UIDEMO2_RADIO_A_ID: u16 = 203;
const UIDEMO2_RADIO_B_ID: u16 = 204;
const UIDEMO2_DROPDOWN_ID: u16 = 205;
const UIDEMO2_COMBO_ID: u16 = 206;
const UIDEMO2_SCROLL_H_ID: u16 = 207;
const UIDEMO2_SCROLL_V_ID: u16 = 208;
const UIDEMO2_LIST_ID: u16 = 209;
const UIDEMO2_TREE_ID: u16 = 210;
const UIDEMO2_PROGRESS_ID: u16 = 211;
const UIDEMO2_POPUP_ID: u16 = 212;
const MULTDEMO_WORKERS: usize = 3;
const MULTDEMO_DEFAULT_ITERATIONS: u32 = 140;
const MULTDEMO_MAX_ITERATIONS: u32 = 20_000;
const MULTDEMO_PROGRESS_TICKS: u32 = 20;
const MULTDEMO_FRAME_TICKS: u32 = 2;

const COMMANDS: [&str; 32] = [
    "help", "clear", "echo", "info", "disk", "fsinfo", "fsformat", "fsls", "fswrite", "fsdelete",
    "fscat", "edit", "date", "time", "rtc", "paging", "uptime", "heap", "memtest", "hexdump",
    "mouse", "matrix", "multdemo", "gfxdemo", "uidemo", "uidemo2", "windemo", "desktop", "color",
    "reboot", "shutdown", "panic",
];

struct TextDocument {
    lines: [[u8; EDITOR_MAX_LINE_LEN]; EDITOR_MAX_LINES],
    lengths: [usize; EDITOR_MAX_LINES],
    count: usize,
}

impl TextDocument {
    fn new() -> Self {
        Self {
            lines: [[0; EDITOR_MAX_LINE_LEN]; EDITOR_MAX_LINES],
            lengths: [0; EDITOR_MAX_LINES],
            count: 0,
        }
    }

    fn load_from_bytes(&mut self, bytes: &[u8]) -> bool {
        self.count = 0;
        let mut current = [0u8; EDITOR_MAX_LINE_LEN];
        let mut current_len = 0usize;
        let mut truncated = false;

        for byte in bytes {
            match *byte {
                b'\r' => {}
                b'\n' => {
                    if !self.push_line_bytes(&current[..current_len]) {
                        truncated = true;
                        return truncated;
                    }
                    current_len = 0;
                }
                value => {
                    if current_len >= EDITOR_MAX_LINE_LEN {
                        truncated = true;
                        continue;
                    }
                    current[current_len] = sanitize_editor_byte(value);
                    current_len += 1;
                }
            }
        }

        if current_len > 0 {
            if !self.push_line_bytes(&current[..current_len]) {
                truncated = true;
            }
        }

        truncated
    }

    fn push_line_bytes(&mut self, text: &[u8]) -> bool {
        if self.count >= EDITOR_MAX_LINES {
            return false;
        }

        let index = self.count;
        let copy_len = text.len().min(EDITOR_MAX_LINE_LEN);
        self.lines[index][..copy_len].copy_from_slice(&text[..copy_len]);
        self.lengths[index] = copy_len;
        self.count += 1;
        true
    }

    fn append_line(&mut self, text: &[u8]) -> Result<(), &'static str> {
        if self.count >= EDITOR_MAX_LINES {
            return Err("line limit reached");
        }

        let copy_len = text.len().min(EDITOR_MAX_LINE_LEN);
        if text.len() > EDITOR_MAX_LINE_LEN {
            return Err("line too long");
        }

        self.lines[self.count][..copy_len].copy_from_slice(&text[..copy_len]);
        self.lengths[self.count] = copy_len;
        self.count += 1;
        Ok(())
    }

    fn insert_line(&mut self, line_no: usize, text: &[u8]) -> Result<(), &'static str> {
        if line_no == 0 || line_no > self.count + 1 {
            return Err("line number out of range");
        }

        if self.count >= EDITOR_MAX_LINES {
            return Err("line limit reached");
        }

        if text.len() > EDITOR_MAX_LINE_LEN {
            return Err("line too long");
        }

        let index = line_no - 1;
        for slot in (index..self.count).rev() {
            let src_len = self.lengths[slot];
            let mut temp = [0u8; EDITOR_MAX_LINE_LEN];
            temp[..src_len].copy_from_slice(&self.lines[slot][..src_len]);
            self.lines[slot + 1][..src_len].copy_from_slice(&temp[..src_len]);
            self.lengths[slot + 1] = src_len;
        }

        self.lines[index][..text.len()].copy_from_slice(text);
        self.lengths[index] = text.len();
        self.count += 1;
        Ok(())
    }

    fn set_line(&mut self, line_no: usize, text: &[u8]) -> Result<(), &'static str> {
        if line_no == 0 || line_no > self.count {
            return Err("line number out of range");
        }

        if text.len() > EDITOR_MAX_LINE_LEN {
            return Err("line too long");
        }

        let index = line_no - 1;
        self.lines[index][..text.len()].copy_from_slice(text);
        self.lengths[index] = text.len();
        Ok(())
    }

    fn delete_line(&mut self, line_no: usize) -> Result<(), &'static str> {
        if line_no == 0 || line_no > self.count {
            return Err("line number out of range");
        }

        let index = line_no - 1;
        for slot in index..self.count - 1 {
            let src_len = self.lengths[slot + 1];
            let mut temp = [0u8; EDITOR_MAX_LINE_LEN];
            temp[..src_len].copy_from_slice(&self.lines[slot + 1][..src_len]);
            self.lines[slot][..src_len].copy_from_slice(&temp[..src_len]);
            self.lengths[slot] = src_len;
        }

        self.count -= 1;
        Ok(())
    }

    fn write_to_buffer(&self, output: &mut [u8; EDITOR_MAX_BYTES]) -> Result<usize, &'static str> {
        let mut cursor = 0usize;

        for index in 0..self.count {
            let line_len = self.lengths[index];
            if cursor
                .checked_add(line_len)
                .is_none_or(|value| value > output.len())
            {
                return Err("document exceeds max size");
            }

            output[cursor..cursor + line_len].copy_from_slice(&self.lines[index][..line_len]);
            cursor += line_len;

            if index + 1 < self.count {
                if cursor >= output.len() {
                    return Err("document exceeds max size");
                }
                output[cursor] = b'\n';
                cursor += 1;
            }
        }

        Ok(cursor)
    }
}

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
        "disk" => handle_disk_command(),
        "fsinfo" => handle_fsinfo_command(),
        "fsformat" => handle_fsformat_command(),
        "fsls" => handle_fsls_command(),
        "fswrite" => handle_fswrite_command(parts),
        "fsdelete" => handle_fsdelete_command(parts),
        "fscat" => handle_fscat_command(parts),
        "edit" => handle_edit_command(parts),
        "date" => print_date(),
        "time" => print_time(),
        "rtc" => handle_rtc_command(),
        "paging" => handle_paging_command(),
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
        "memtest" => handle_memtest_command(parts),
        "hexdump" => handle_hexdump_command(parts),
        "mouse" => handle_mouse_command(),
        "matrix" => {
            shell_println!("matrix mode: press any key to return");
            matrix::run();
        }
        "multdemo" => {
            handle_multdemo_command(parts);
        }
        "gfxdemo" => {
            handle_gfxdemo_command();
        }
        "uidemo" => {
            handle_uidemo_command();
        }
        "uidemo2" => {
            handle_uidemo2_command();
        }
        "windemo" => {
            handle_windemo_command();
        }
        "desktop" => {
            handle_desktop_command();
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

fn handle_gfxdemo_command() {
    let Some((fb_width, fb_height)) = vga::framebuffer_resolution() else {
        shell_println!("gfxdemo requires VBE/framebuffer mode");
        return;
    };

    let width = fb_width.min(i32::MAX as usize) as i32;
    let height = fb_height.min(i32::MAX as usize) as i32;
    if width <= 0 || height <= 0 {
        shell_println!("gfxdemo: invalid framebuffer size");
        return;
    }

    shell_println!("gfxdemo: press any key to return");
    for _ in 0..256 {
        if input::pop_event().is_none() {
            break;
        }
    }
    let key_activity_marker = keyboard::key_activity();

    let _ = vga::draw_filled_rect(0, 0, width, height, 0x111520);
    let _ = vga::draw_filled_rect(24, 24, width - 48, height - 48, 0x1B2232);
    let _ = vga::draw_filled_rect(40, 40, width - 80, 56, 0x27344D);

    let _ = vga::draw_horizontal_line(40, 116, width - 80, 0x6FA8FF);
    let _ = vga::draw_vertical_line(40, 116, height - 156, 0x6FA8FF);
    let _ = vga::draw_horizontal_line(40, height - 40, width - 80, 0x6FA8FF);
    let _ = vga::draw_vertical_line(width - 40, 116, height - 156, 0x6FA8FF);

    let _ = vga::draw_line(56, 132, width - 56, height - 56, 0xFF8A65);
    let _ = vga::draw_line(width - 56, 132, 56, height - 56, 0x7CFFCB);
    let _ = vga::draw_line(56, height / 2, width - 56, height / 2, 0xFFE082);

    let circle_r = (width.min(height) / 7).max(18);
    let _ = vga::draw_circle(width / 3, height / 2 + 24, circle_r, 0xFFD166);
    let _ = vga::draw_ellipse(
        (width * 2) / 3,
        height / 2 + 24,
        circle_r + 26,
        (circle_r * 2) / 3,
        0x66D9EF,
    );

    const ICON_W: usize = 28;
    const ICON_H: usize = 28;
    let mut icon = [0u32; ICON_W * ICON_H];
    for y in 0..ICON_H {
        for x in 0..ICON_W {
            let idx = y * ICON_W + x;
            let border = x == 0 || y == 0 || x + 1 == ICON_W || y + 1 == ICON_H;
            let checker = ((x / 4) + (y / 4)) & 1 == 0;
            icon[idx] = if border {
                0xFFFFFF
            } else if checker {
                0xF07178
            } else {
                0x82AAFF
            };
        }
    }

    let bottom = (height - ICON_H as i32 - 52).max(0);
    let _ = vga::blit_bitmap(56, bottom, &icon, ICON_W, ICON_H, ICON_W);
    let _ = vga::blit_bitmap(
        width - ICON_W as i32 - 56,
        bottom,
        &icon,
        ICON_W,
        ICON_H,
        ICON_W,
    );

    loop {
        if serial::read_byte().is_some() {
            break;
        }

        let mut should_exit = false;
        for _ in 0..128 {
            let Some(event) = input::pop_event() else {
                break;
            };
            if let InputEvent::KeyPress { .. } = event {
                should_exit = true;
                break;
            }
        }
        if should_exit {
            break;
        }

        let current_activity = keyboard::key_activity();
        if current_activity != key_activity_marker {
            break;
        }

        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }

    vga::clear_screen();
}

fn handle_uidemo_command() {
    let Some((fb_width, fb_height)) = vga::framebuffer_resolution() else {
        shell_println!("uidemo requires VBE/framebuffer mode");
        return;
    };

    let width = fb_width.min(i32::MAX as usize) as i32;
    let height = fb_height.min(i32::MAX as usize) as i32;
    if width <= 0 || height <= 0 {
        shell_println!("uidemo: invalid framebuffer size");
        return;
    }

    let Some((_, font_h)) = vga::font_metrics() else {
        shell_println!("uidemo: font metrics unavailable");
        return;
    };

    shell_println!("uidemo: Tab to focus, Enter/Space to activate, q to exit");
    for _ in 0..512 {
        if input::pop_event().is_none() {
            break;
        }
    }

    let panel_margin = 24;
    let panel = ui::Rect::new(
        panel_margin,
        panel_margin,
        width - panel_margin * 2,
        height - panel_margin * 2,
    );
    if panel.width <= 200 || panel.height <= 140 {
        shell_println!("uidemo: framebuffer is too small");
        return;
    }

    let title_height = (font_h as i32 + 10).max(20);
    let title_rect = ui::Rect::new(panel.x + 16, panel.y + 16, panel.width - 32, title_height);
    let hint_rect = ui::Rect::new(
        panel.x + 16,
        title_rect.y + title_rect.height + 8,
        panel.width - 32,
        title_height,
    );

    let button_height = (font_h as i32 + 18).max(30);
    let button_width = ((panel.width - 56) / 2).max(96);
    let button_y = panel.y + panel.height - button_height - 24;
    let ping_rect = ui::Rect::new(panel.x + 20, button_y, button_width, button_height);
    let exit_rect = ui::Rect::new(
        panel.x + panel.width - button_width - 20,
        button_y,
        button_width,
        button_height,
    );

    let status_height = (font_h as i32 + 10).max(18);
    let status_y = (button_y - status_height - 10).max(hint_rect.y + hint_rect.height + 8);
    let status_rect = ui::Rect::new(panel.x + 16, status_y, panel.width - 32, status_height);

    let mut dispatcher = ui::EventDispatcher::new();
    if dispatcher
        .add_panel(ui::Panel::new(1, panel, 0x0D1424, 0x00E5FF))
        .is_err()
    {
        shell_println!("uidemo: failed to allocate panel widget");
        return;
    }

    if dispatcher
        .add_label(ui::Label::new(
            2,
            title_rect,
            "Event Dispatcher + Widgets",
            0xE8F1FF,
            0x111A2E,
        ))
        .is_err()
    {
        shell_println!("uidemo: failed to allocate title widget");
        return;
    }

    if dispatcher
        .add_label(ui::Label::new(
            3,
            hint_rect,
            "Mouse click routes by hit-region. Tab changes keyboard focus.",
            0xB7C7E4,
            0x111A2E,
        ))
        .is_err()
    {
        shell_println!("uidemo: failed to allocate hint widget");
        return;
    }

    let mut ping_button = ui::Button::new(UIDEMO_BUTTON_PING_ID, ping_rect, "PING");
    ping_button.fill_normal = 0x1A2B45;
    ping_button.fill_hover = 0x224171;
    ping_button.fill_pressed = 0x112238;
    ping_button.border = 0x00E5FF;
    ping_button.border_focused = 0x39FF14;
    ping_button.text_color = 0xF3F7FF;
    if dispatcher.add_button(ping_button).is_err() {
        shell_println!("uidemo: failed to allocate ping button");
        return;
    }

    let mut exit_button = ui::Button::new(UIDEMO_BUTTON_EXIT_ID, exit_rect, "EXIT");
    exit_button.fill_normal = 0x3A1538;
    exit_button.fill_hover = 0x552059;
    exit_button.fill_pressed = 0x2A102A;
    exit_button.border = 0xFF00FF;
    exit_button.border_focused = 0x39FF14;
    exit_button.text_color = 0xFFE8FF;
    if dispatcher.add_button(exit_button).is_err() {
        shell_println!("uidemo: failed to allocate exit button");
        return;
    }

    let _ = dispatcher.focus_first();

    let mut status_kind = UidemoStatus::Ready;
    draw_uidemo_background(width, height);
    draw_uidemo_scene(&dispatcher, status_rect, status_kind);
    let arm_input_after = timer::ticks().wrapping_add(25);

    loop {
        let batch = dispatcher.poll_and_dispatch(128);
        let mut redraw = batch.redraw;

        if let Some(key) = batch.key_press {
            if matches!(key, KeyEvent::Char('q') | KeyEvent::Char('Q')) {
                break;
            }
        }

        if let Some(clicked) = batch.clicked {
            let armed = timer::ticks().wrapping_sub(arm_input_after) < (u32::MAX / 2);
            if armed {
                if clicked == UIDEMO_BUTTON_PING_ID {
                    status_kind = UidemoStatus::PingClicked;
                    redraw = true;
                } else if clicked == UIDEMO_BUTTON_EXIT_ID {
                    status_kind = UidemoStatus::ExitClicked;
                    redraw = true;
                }
            }
        }

        if redraw {
            draw_uidemo_scene(&dispatcher, status_rect, status_kind);
            if matches!(status_kind, UidemoStatus::ExitClicked) {
                break;
            }
        }

        if batch.processed == 0 {
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
            }
        }
    }

    vga::clear_screen();
}

#[derive(Clone, Copy)]
enum UidemoStatus {
    Ready,
    PingClicked,
    ExitClicked,
}

fn draw_uidemo_scene(
    dispatcher: &ui::EventDispatcher,
    status_rect: ui::Rect,
    status_kind: UidemoStatus,
) {
    vga::begin_draw_batch();
    dispatcher.draw();
    draw_uidemo_status_bar(status_rect, dispatcher.focused_widget(), status_kind);
    vga::end_draw_batch();
}

fn draw_uidemo_background(width: i32, height: i32) {
    if width <= 0 || height <= 0 {
        return;
    }
    let _ = vga::draw_filled_rect(0, 0, width, height, 0x070B14);
}

fn draw_uidemo_status_bar(status_rect: ui::Rect, focus: Option<u16>, status_kind: UidemoStatus) {
    let message = match (status_kind, focus) {
        (UidemoStatus::ExitClicked, _) => "Exit clicked. Leaving demo...",
        (UidemoStatus::PingClicked, Some(UIDEMO_BUTTON_EXIT_ID)) => {
            "PING clicked. Focus on EXIT (Enter/Space or click)."
        }
        (UidemoStatus::PingClicked, Some(UIDEMO_BUTTON_PING_ID)) => {
            "PING clicked. Focus on PING (Tab to move focus)."
        }
        (UidemoStatus::PingClicked, _) => "PING clicked. Focus none (Tab or click a button).",
        (UidemoStatus::Ready, Some(UIDEMO_BUTTON_EXIT_ID)) => {
            "Focus: EXIT. Enter/Space activates focused button."
        }
        (UidemoStatus::Ready, Some(UIDEMO_BUTTON_PING_ID)) => {
            "Focus: PING. Enter/Space activates focused button."
        }
        (UidemoStatus::Ready, _) => "Focus: none. Tab/click to focus, q to exit.",
    };

    let _ = vga::draw_filled_rect(
        status_rect.x,
        status_rect.y,
        status_rect.width,
        status_rect.height,
        0x0D1B2E,
    );
    let _ = vga::draw_horizontal_line(
        status_rect.x,
        status_rect.y,
        status_rect.width,
        0x4A6FA8,
    );
    let _ = vga::draw_horizontal_line(
        status_rect.x,
        status_rect
            .y
            .saturating_add(status_rect.height)
            .saturating_sub(1),
        status_rect.width,
        0x4A6FA8,
    );

    if let Some((_, font_h)) = vga::font_metrics() {
        let text_y = status_rect
            .y
            .saturating_add(((status_rect.height - font_h as i32) / 2).max(1));
        let _ = vga::draw_text(status_rect.x + 8, text_y, message, 0xDFEAFF, 0x0D1B2E);
    }
}

fn handle_uidemo2_command() {
    let Some((fb_width, fb_height)) = vga::framebuffer_resolution() else {
        shell_println!("uidemo2 requires VBE/framebuffer mode");
        return;
    };

    let width = fb_width.min(i32::MAX as usize) as i32;
    let height = fb_height.min(i32::MAX as usize) as i32;
    if width <= 0 || height <= 0 {
        shell_println!("uidemo2: invalid framebuffer size");
        return;
    }

    let Some((_, font_h)) = vga::font_metrics() else {
        shell_println!("uidemo2: font metrics unavailable");
        return;
    };

    shell_println!("uidemo2: Tab focus, right-click for context menu, q to exit");
    for _ in 0..512 {
        if input::pop_event().is_none() {
            break;
        }
    }

    let margin = 14;
    let panel = ui::Rect::new(margin, margin, width - margin * 2, height - margin * 2);
    if panel.width <= 520 || panel.height <= 380 {
        shell_println!("uidemo2: framebuffer is too small");
        return;
    }

    let gutter = 10;
    let row_h = (font_h as i32 + 10).max(22);
    let title_rect = ui::Rect::new(panel.x + gutter, panel.y + gutter, panel.width - 2 * gutter, row_h);
    let subtitle_rect = ui::Rect::new(
        panel.x + gutter,
        title_rect.y + title_rect.height + 4,
        panel.width - 2 * gutter,
        row_h,
    );
    let status_rect = ui::Rect::new(
        panel.x + gutter,
        panel.y + panel.height - row_h - gutter,
        panel.width - 2 * gutter,
        row_h,
    );

    let content_top = subtitle_rect.y + subtitle_rect.height + 8;
    let content_bottom = status_rect.y - 8;
    let content_height = content_bottom - content_top;
    if content_height <= row_h * 8 {
        shell_println!("uidemo2: framebuffer is too small for layout");
        return;
    }

    let col_w = ((panel.width - gutter * 3) / 2).max(240);
    let left_x = panel.x + gutter;
    let right_x = left_x + col_w + gutter;

    let mut left_y = content_top;
    let textbox_rect = ui::Rect::new(left_x, left_y, col_w - 2, row_h);
    left_y += row_h + 6;
    let dropdown_rect = ui::Rect::new(left_x, left_y, col_w - 2, row_h);
    left_y += row_h + 6;
    let combo_rect = ui::Rect::new(left_x, left_y, col_w - 2, row_h);
    left_y += row_h + 6;
    let checkbox_rect = ui::Rect::new(left_x, left_y, col_w - 2, row_h);
    left_y += row_h + 4;
    let radio_a_rect = ui::Rect::new(left_x, left_y, col_w - 2, row_h);
    left_y += row_h + 4;
    let radio_b_rect = ui::Rect::new(left_x, left_y, col_w - 2, row_h);
    left_y += row_h + 8;
    let hscroll_rect = ui::Rect::new(left_x, left_y, col_w - 2, 14);
    left_y += 14 + 8;
    let progress_rect = ui::Rect::new(left_x, left_y, col_w - 2, row_h);

    let textarea_h = (content_height / 3).max(row_h * 4);
    let textarea_rect = ui::Rect::new(right_x, content_top, col_w - 20, textarea_h);
    let vscroll_rect = ui::Rect::new(right_x + col_w - 14, content_top, 12, textarea_h);

    let list_y = textarea_rect.y + textarea_rect.height + 8;
    let list_h = (content_height / 3).max(row_h * 4);
    let list_rect = ui::Rect::new(right_x, list_y, col_w - 2, list_h);

    let tree_y = list_rect.y + list_rect.height + 8;
    let tree_h = (content_bottom - tree_y).max(row_h * 3);
    let tree_rect = ui::Rect::new(right_x, tree_y, col_w - 2, tree_h);

    let mut dispatcher = ui::EventDispatcher::new();
    if dispatcher
        .add_panel(ui::Panel::new(100, panel, 0x0C1323, 0x00E5FF))
        .is_err()
    {
        shell_println!("uidemo2: failed to allocate panel");
        return;
    }
    if dispatcher
        .add_label(ui::Label::new(
            101,
            title_rect,
            "UI Demo 2: Advanced Widget Set",
            0xE8F1FF,
            0x131C2F,
        ))
        .is_err()
    {
        shell_println!("uidemo2: failed to allocate title");
        return;
    }
    if dispatcher
        .add_label(ui::Label::new(
            102,
            subtitle_rect,
            "Text, selection, list/tree, combo/dropdown, scrollbars, progress, popup menu",
            0xB7C7E4,
            0x131C2F,
        ))
        .is_err()
    {
        shell_println!("uidemo2: failed to allocate subtitle");
        return;
    }

    let mut text_box = ui::TextBox::new(UIDEMO2_TEXTBOX_ID, textbox_rect);
    text_box.placeholder = "single-line input";
    text_box.set_text("edit me");
    if dispatcher.add_text_box(text_box).is_err() {
        shell_println!("uidemo2: failed to allocate textbox");
        return;
    }

    let mut dropdown = ui::Dropdown::new(
        UIDEMO2_DROPDOWN_ID,
        dropdown_rect,
        vec!["Debug", "Release", "Safe", "Turbo"],
    );
    dropdown.selected = 1;
    if dispatcher.add_dropdown(dropdown).is_err() {
        shell_println!("uidemo2: failed to allocate dropdown");
        return;
    }

    let combo = ui::ComboBox::new(
        UIDEMO2_COMBO_ID,
        combo_rect,
        vec!["alpha", "beta", "gamma", "delta", "omega"],
    );
    if dispatcher.add_combo_box(combo).is_err() {
        shell_println!("uidemo2: failed to allocate combobox");
        return;
    }

    let mut checkbox = ui::Checkbox::new(UIDEMO2_CHECKBOX_ID, checkbox_rect, "Enable telemetry");
    checkbox.checked = true;
    if dispatcher.add_checkbox(checkbox).is_err() {
        shell_println!("uidemo2: failed to allocate checkbox");
        return;
    }

    let mut radio_a = ui::RadioButton::new(UIDEMO2_RADIO_A_ID, 1, radio_a_rect, "Renderer: Raster");
    radio_a.selected = true;
    if dispatcher.add_radio_button(radio_a).is_err() {
        shell_println!("uidemo2: failed to allocate radio A");
        return;
    }

    let radio_b = ui::RadioButton::new(UIDEMO2_RADIO_B_ID, 1, radio_b_rect, "Renderer: Vector");
    if dispatcher.add_radio_button(radio_b).is_err() {
        shell_println!("uidemo2: failed to allocate radio B");
        return;
    }

    let mut h_scroll = ui::Scrollbar::new(UIDEMO2_SCROLL_H_ID, hscroll_rect, ui::Orientation::Horizontal);
    h_scroll.max = 100;
    h_scroll.page = 20;
    h_scroll.value = 35;
    if dispatcher.add_scrollbar(h_scroll).is_err() {
        shell_println!("uidemo2: failed to allocate horizontal scrollbar");
        return;
    }

    let mut progress = ui::ProgressBar::new(UIDEMO2_PROGRESS_ID, progress_rect);
    progress.max = 100;
    progress.value = 35;
    progress.foreground = 0x39FF14;
    if dispatcher.add_progress_bar(progress).is_err() {
        shell_println!("uidemo2: failed to allocate progress bar");
        return;
    }

    let mut text_area = ui::TextArea::new(UIDEMO2_TEXTAREA_ID, textarea_rect);
    text_area.placeholder = "multi-line input";
    text_area.set_text("This is TextArea.\nType here.\nUse arrows and PageUp/PageDown.");
    if dispatcher.add_text_area(text_area).is_err() {
        shell_println!("uidemo2: failed to allocate textarea");
        return;
    }

    let mut v_scroll = ui::Scrollbar::new(UIDEMO2_SCROLL_V_ID, vscroll_rect, ui::Orientation::Vertical);
    v_scroll.max = 100;
    v_scroll.page = 25;
    v_scroll.value = 40;
    if dispatcher.add_scrollbar(v_scroll).is_err() {
        shell_println!("uidemo2: failed to allocate vertical scrollbar");
        return;
    }

    let mut list_view = ui::ListView::new(
        UIDEMO2_LIST_ID,
        list_rect,
        vec![
            "kernel.log",
            "drivers/",
            "boot/",
            "README.md",
            "Cargo.toml",
            "src/",
            "build/",
            "target/",
            "notes.txt",
            "assets/",
        ],
    );
    list_view.selected = Some(0);
    if dispatcher.add_list_view(list_view).is_err() {
        shell_println!("uidemo2: failed to allocate list view");
        return;
    }

    let nodes = vec![
        ui::TreeNode::new("root", 0, true, true),
        ui::TreeNode::new("boot", 1, true, false),
        ui::TreeNode::new("stage1", 2, false, true),
        ui::TreeNode::new("stage2", 2, false, true),
        ui::TreeNode::new("src", 1, true, true),
        ui::TreeNode::new("ui.rs", 2, false, true),
        ui::TreeNode::new("shell.rs", 2, false, true),
        ui::TreeNode::new("vga.rs", 2, false, true),
        ui::TreeNode::new("tests", 1, true, false),
        ui::TreeNode::new("integration", 2, false, true),
    ];
    let mut tree_view = ui::TreeView::new(UIDEMO2_TREE_ID, tree_rect, nodes);
    tree_view.selected = Some(0);
    if dispatcher.add_tree_view(tree_view).is_err() {
        shell_println!("uidemo2: failed to allocate tree view");
        return;
    }

    let popup = ui::PopupMenu::new(
        UIDEMO2_POPUP_ID,
        ui::Rect::new(panel.x + 40, panel.y + 40, 180, 24),
        vec!["Copy", "Paste", "Rename", "Delete", "Properties"],
    );
    if dispatcher.add_popup_menu(popup).is_err() {
        shell_println!("uidemo2: failed to allocate popup menu");
        return;
    }

    let _ = dispatcher.focus_first();

    let mut status_message = "Ready: Tab focus, type in TextBox/TextArea, right-click for menu.";
    draw_uidemo2_background(width, height);
    draw_uidemo2_scene(&dispatcher, status_rect, status_message);

    'demo: loop {
        let mut redraw = false;
        let mut processed = 0usize;

        for _ in 0..128 {
            let Some(event) = input::pop_event() else {
                break;
            };
            processed += 1;

            match event {
                InputEvent::KeyPress {
                    key: KeyEvent::Char('q'),
                }
                | InputEvent::KeyPress {
                    key: KeyEvent::Char('Q'),
                } => break 'demo,
                InputEvent::MouseDown {
                    button: MouseButton::Right,
                    x,
                    y,
                } => {
                    if dispatcher.show_popup_menu(UIDEMO2_POPUP_ID, x, y) {
                        status_message = "Context menu opened.";
                        redraw = true;
                    }
                    continue;
                }
                _ => {}
            }

            let batch = dispatcher.dispatch_input_event(event);
            if batch.redraw {
                redraw = true;
            }

            if let Some(clicked) = batch.clicked {
                status_message = uidemo2_status_message(clicked, &dispatcher);
                redraw = true;
            }
        }

        if let Some(value) = dispatcher.scrollbar_value(UIDEMO2_SCROLL_H_ID) {
            let clamped = value.clamp(0, 100) as u32;
            if dispatcher.set_progress_value(UIDEMO2_PROGRESS_ID, clamped) {
                redraw = true;
            }
        }

        if redraw {
            draw_uidemo2_scene(&dispatcher, status_rect, status_message);
        }

        if processed == 0 {
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
            }
        }
    }

    vga::clear_screen();
}

fn uidemo2_status_message(clicked_id: u16, dispatcher: &ui::EventDispatcher) -> &'static str {
    match clicked_id {
        UIDEMO2_TEXTBOX_ID => "TextBox updated.",
        UIDEMO2_TEXTAREA_ID => "TextArea updated.",
        UIDEMO2_CHECKBOX_ID => "Checkbox toggled.",
        UIDEMO2_RADIO_A_ID => "Radio selected: Raster.",
        UIDEMO2_RADIO_B_ID => "Radio selected: Vector.",
        UIDEMO2_DROPDOWN_ID => "Dropdown changed.",
        UIDEMO2_COMBO_ID => "ComboBox changed.",
        UIDEMO2_SCROLL_H_ID => "Horizontal scrollbar moved.",
        UIDEMO2_SCROLL_V_ID => "Vertical scrollbar moved.",
        UIDEMO2_LIST_ID => "ListView selection changed.",
        UIDEMO2_TREE_ID => "TreeView selection/toggle changed.",
        UIDEMO2_POPUP_ID => match dispatcher.popup_menu_selected(UIDEMO2_POPUP_ID) {
            Some(0) => "Popup: Copy",
            Some(1) => "Popup: Paste",
            Some(2) => "Popup: Rename",
            Some(3) => "Popup: Delete",
            Some(4) => "Popup: Properties",
            _ => "Popup action",
        },
        _ => "Widget interaction.",
    }
}

fn draw_uidemo2_scene(
    dispatcher: &ui::EventDispatcher,
    status_rect: ui::Rect,
    status_message: &'static str,
) {
    vga::begin_draw_batch();
    dispatcher.draw();
    draw_uidemo2_status_bar(status_rect, status_message);
    vga::end_draw_batch();
}

fn draw_uidemo2_background(width: i32, height: i32) {
    if width <= 0 || height <= 0 {
        return;
    }
    let _ = vga::draw_filled_rect(0, 0, width, height, 0x050914);
}

fn draw_uidemo2_status_bar(status_rect: ui::Rect, message: &'static str) {
    let _ = vga::draw_filled_rect(
        status_rect.x,
        status_rect.y,
        status_rect.width,
        status_rect.height,
        0x11243A,
    );
    let _ = vga::draw_horizontal_line(status_rect.x, status_rect.y, status_rect.width, 0x4A6FA8);
    let _ = vga::draw_horizontal_line(
        status_rect.x,
        status_rect
            .y
            .saturating_add(status_rect.height)
            .saturating_sub(1),
        status_rect.width,
        0x4A6FA8,
    );

    if let Some((_, font_h)) = vga::font_metrics() {
        let text_y = status_rect
            .y
            .saturating_add(((status_rect.height - font_h as i32) / 2).max(1));
        let _ = vga::draw_text(status_rect.x + 8, text_y, message, 0xDFEAFF, 0x11243A);
    }
}

fn handle_windemo_command() {
    let Some((fb_width, fb_height)) = vga::framebuffer_resolution() else {
        shell_println!("windemo requires VBE/framebuffer mode");
        return;
    };

    let width = fb_width.min(i32::MAX as usize) as i32;
    let height = fb_height.min(i32::MAX as usize) as i32;
    if width <= 0 || height <= 0 {
        shell_println!("windemo: invalid framebuffer size");
        return;
    }

    if width < 720 || height < 420 {
        shell_println!("windemo: framebuffer too small (need at least 720x420)");
        return;
    }

    shell_println!(
        "windemo: drag title bars, resize borders/corners, use window buttons, q exits, d toggles debug"
    );
    for _ in 0..512 {
        if input::pop_event().is_none() {
            break;
        }
    }

    let desktop = ui::Rect::new(0, 0, width, height);
    let mut manager = ui::WindowManager::new(0x081224);

    let shell_window = manager.add_window(ui::WindowSpec {
        title: "Shell",
        rect: ui::Rect::new(52, 54, 420, 260),
        min_width: 220,
        min_height: 150,
        background: 0x0A1A2C,
        accent: 0x00E5FF,
    });
    let monitor_window = manager.add_window(ui::WindowSpec {
        title: "Monitor",
        rect: ui::Rect::new(316, 112, 390, 242),
        min_width: 220,
        min_height: 150,
        background: 0x101A11,
        accent: 0x39FF14,
    });
    let tools_window = manager.add_window(ui::WindowSpec {
        title: "Tools",
        rect: ui::Rect::new(186, 238, 320, 196),
        min_width: 180,
        min_height: 130,
        background: 0x25190B,
        accent: 0xFF9E3D,
    });

    let Ok(shell_id) = shell_window else {
        shell_println!("windemo: failed to create Shell window");
        return;
    };
    let Ok(monitor_id) = monitor_window else {
        shell_println!("windemo: failed to create Monitor window");
        return;
    };
    let Ok(tools_id) = tools_window else {
        shell_println!("windemo: failed to create Tools window");
        return;
    };

    paint_windemo_window(&mut manager, shell_id, 0x112A44, 0x0D1E35, 0x21557D);
    paint_windemo_window(&mut manager, monitor_id, 0x182B18, 0x132013, 0x286C36);
    paint_windemo_window(&mut manager, tools_id, 0x3A250E, 0x291A0B, 0x8C5C21);

    let mut debug_enabled = false;
    draw_windemo_scene(&manager, desktop, width, height, debug_enabled);

    'demo: loop {
        let mut redraw = false;
        let mut processed = 0usize;

        for _ in 0..128 {
            let Some(event) = input::pop_event() else {
                break;
            };
            processed += 1;

            match event {
                InputEvent::KeyPress {
                    key: KeyEvent::Char('q'),
                }
                | InputEvent::KeyPress {
                    key: KeyEvent::Char('Q'),
                } => break 'demo,
                InputEvent::KeyPress {
                    key: KeyEvent::Char('d'),
                }
                | InputEvent::KeyPress {
                    key: KeyEvent::Char('D'),
                } => {
                    debug_enabled = !debug_enabled;
                    redraw = true;
                    continue;
                }
                _ => {}
            }

            let response = manager.handle_event(event, desktop);
            if response.redraw || response.closed.is_some() {
                redraw = true;
            }
            if debug_enabled && matches!(event, InputEvent::MouseMove { .. }) {
                redraw = true;
            }
        }

        if manager.window_count() == 0 {
            break;
        }

        if redraw {
            paint_windemo_window(&mut manager, shell_id, 0x112A44, 0x0D1E35, 0x21557D);
            paint_windemo_window(&mut manager, monitor_id, 0x182B18, 0x132013, 0x286C36);
            paint_windemo_window(&mut manager, tools_id, 0x3A250E, 0x291A0B, 0x8C5C21);
            draw_windemo_scene(&manager, desktop, width, height, debug_enabled);
        }

        if processed == 0 {
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
            }
        }
    }

    vga::clear_screen();
}

fn paint_windemo_window(
    manager: &mut ui::WindowManager,
    id: ui::WindowId,
    base_a: u32,
    base_b: u32,
    stripe: u32,
) {
    let _ = manager.with_window_buffer_mut(id, |pixels, width, height| {
        if width == 0 || height == 0 {
            return;
        }

        let cell = 18usize;
        for y in 0..height {
            let row = y * width;
            for x in 0..width {
                let checker = ((x / cell) + (y / cell)) & 1 == 0;
                pixels[row + x] = if checker { base_a } else { base_b };
            }
        }

        for y in (6..height).step_by(24) {
            let row = y * width;
            for x in 6..width.saturating_sub(6) {
                pixels[row + x] = stripe;
            }
        }

        if width >= 3 && height >= 3 {
            let top = 0usize;
            let bottom = height - 1;
            for x in 0..width {
                pixels[top * width + x] = 0xE8F1FF;
                pixels[bottom * width + x] = 0xE8F1FF;
            }
            for y in 0..height {
                let row = y * width;
                pixels[row] = 0xE8F1FF;
                pixels[row + width - 1] = 0xE8F1FF;
            }
        }
    });
}

fn draw_windemo_scene(
    manager: &ui::WindowManager,
    desktop: ui::Rect,
    width: i32,
    height: i32,
    debug_enabled: bool,
) {
    vga::begin_draw_batch();
    manager.compose(desktop);
    draw_windemo_overlay_bottom(width, height);
    if debug_enabled {
        let pointer = mouse::state();
        let snapshot = manager.debug_snapshot(pointer.x, pointer.y);
        draw_windemo_debug_overlay(pointer.x, pointer.y, snapshot);
    }
    vga::end_draw_batch();
}

fn draw_windemo_overlay_bottom(width: i32, height: i32) {
    let overlay_height = 24;
    let overlay_width = (width - 24).max(240);
    let overlay_x = 12;
    let overlay_y = height.saturating_sub(overlay_height + 10);
    let _ = vga::draw_filled_rect(overlay_x, overlay_y, overlay_width, overlay_height, 0x0E1B33);
    let _ = vga::draw_horizontal_line(overlay_x, overlay_y, overlay_width, 0x41658E);
    let _ = vga::draw_horizontal_line(
        overlay_x,
        overlay_y + overlay_height - 1,
        overlay_width,
        0x41658E,
    );
    let _ = vga::draw_text(
        overlay_x + 8,
        overlay_y + 4,
        "windemo: click+drag/resize, q exits, d debug",
        0xE1ECFF,
        0x0E1B33,
    );
}

fn draw_windemo_debug_overlay(cursor_x: i32, cursor_y: i32, snapshot: ui::WindowDebugSnapshot) {
    let panel_x = 12;
    let panel_y = 12;
    let panel_w = 420;
    let panel_h = 74;
    let bg = 0x13232D;
    let border = 0x4A7D91;
    let fg = 0xE4F2F9;

    let _ = vga::draw_filled_rect(panel_x, panel_y, panel_w, panel_h, bg);
    let _ = vga::draw_horizontal_line(panel_x, panel_y, panel_w, border);
    let _ = vga::draw_horizontal_line(panel_x, panel_y + panel_h - 1, panel_w, border);
    let _ = vga::draw_vertical_line(panel_x, panel_y, panel_h, border);
    let _ = vga::draw_vertical_line(panel_x + panel_w - 1, panel_y, panel_h, border);

    let mut line0 = [0u8; 160];
    let mut line0_len = 0usize;
    windemo_push_bytes(&mut line0, &mut line0_len, b"dbg cursor=(");
    windemo_push_i32(&mut line0, &mut line0_len, cursor_x);
    windemo_push_bytes(&mut line0, &mut line0_len, b",");
    windemo_push_i32(&mut line0, &mut line0_len, cursor_y);
    windemo_push_bytes(&mut line0, &mut line0_len, b")");
    draw_windemo_debug_line(panel_x + 8, panel_y + 5, &line0[..line0_len], fg, bg);

    let mut line1 = [0u8; 160];
    let mut line1_len = 0usize;
    windemo_push_bytes(&mut line1, &mut line1_len, b"hover id=");
    match snapshot.cursor_window_id {
        Some(id) => windemo_push_u16(&mut line1, &mut line1_len, id),
        None => windemo_push_bytes(&mut line1, &mut line1_len, b"none"),
    }
    windemo_push_bytes(&mut line1, &mut line1_len, b" frame=");
    match snapshot.cursor_window_frame {
        Some(rect) => {
            windemo_push_i32(&mut line1, &mut line1_len, rect.width);
            windemo_push_bytes(&mut line1, &mut line1_len, b"x");
            windemo_push_i32(&mut line1, &mut line1_len, rect.height);
        }
        None => windemo_push_bytes(&mut line1, &mut line1_len, b"-"),
    }
    draw_windemo_debug_line(panel_x + 8, panel_y + 21, &line1[..line1_len], fg, bg);

    let mut line2 = [0u8; 160];
    let mut line2_len = 0usize;
    windemo_push_bytes(&mut line2, &mut line2_len, b"hover client=");
    match snapshot.cursor_window_client {
        Some(rect) => {
            windemo_push_i32(&mut line2, &mut line2_len, rect.width);
            windemo_push_bytes(&mut line2, &mut line2_len, b"x");
            windemo_push_i32(&mut line2, &mut line2_len, rect.height);
        }
        None => windemo_push_bytes(&mut line2, &mut line2_len, b"-"),
    }
    windemo_push_bytes(&mut line2, &mut line2_len, b" focus=");
    match snapshot.focused_window_id {
        Some(id) => windemo_push_u16(&mut line2, &mut line2_len, id),
        None => windemo_push_bytes(&mut line2, &mut line2_len, b"none"),
    }
    windemo_push_bytes(&mut line2, &mut line2_len, b" frame=");
    match snapshot.focused_window_frame {
        Some(rect) => {
            windemo_push_i32(&mut line2, &mut line2_len, rect.width);
            windemo_push_bytes(&mut line2, &mut line2_len, b"x");
            windemo_push_i32(&mut line2, &mut line2_len, rect.height);
        }
        None => windemo_push_bytes(&mut line2, &mut line2_len, b"-"),
    }
    draw_windemo_debug_line(panel_x + 8, panel_y + 37, &line2[..line2_len], fg, bg);

    let mut line3 = [0u8; 160];
    let mut line3_len = 0usize;
    windemo_push_bytes(&mut line3, &mut line3_len, b"resizing id=");
    match snapshot.resizing_window_id {
        Some(id) => windemo_push_u16(&mut line3, &mut line3_len, id),
        None => windemo_push_bytes(&mut line3, &mut line3_len, b"none"),
    }
    windemo_push_bytes(&mut line3, &mut line3_len, b" frame=");
    match snapshot.resizing_window_frame {
        Some(rect) => {
            windemo_push_i32(&mut line3, &mut line3_len, rect.width);
            windemo_push_bytes(&mut line3, &mut line3_len, b"x");
            windemo_push_i32(&mut line3, &mut line3_len, rect.height);
        }
        None => windemo_push_bytes(&mut line3, &mut line3_len, b"-"),
    }
    draw_windemo_debug_line(panel_x + 8, panel_y + 53, &line3[..line3_len], fg, bg);
}

fn draw_windemo_debug_line(x: i32, y: i32, line: &[u8], fg: u32, bg: u32) {
    if line.is_empty() {
        return;
    }
    if let Ok(text) = core::str::from_utf8(line) {
        let _ = vga::draw_text(x, y, text, fg, bg);
    }
}

fn windemo_push_bytes<const N: usize>(buffer: &mut [u8; N], len: &mut usize, bytes: &[u8]) {
    let remain = N.saturating_sub(*len);
    if remain == 0 {
        return;
    }
    let copy_len = bytes.len().min(remain);
    buffer[*len..*len + copy_len].copy_from_slice(&bytes[..copy_len]);
    *len += copy_len;
}

fn windemo_push_u16<const N: usize>(buffer: &mut [u8; N], len: &mut usize, value: u16) {
    windemo_push_u32(buffer, len, value as u32);
}

fn windemo_push_i32<const N: usize>(buffer: &mut [u8; N], len: &mut usize, value: i32) {
    if value < 0 {
        windemo_push_bytes(buffer, len, b"-");
        windemo_push_u32(buffer, len, value.wrapping_neg() as u32);
    } else {
        windemo_push_u32(buffer, len, value as u32);
    }
}

fn windemo_push_u32<const N: usize>(buffer: &mut [u8; N], len: &mut usize, mut value: u32) {
    let mut digits = [0u8; 10];
    let mut count = 0usize;
    if value == 0 {
        digits[count] = b'0';
        count = 1;
    } else {
        while value > 0 && count < digits.len() {
            digits[count] = b'0' + (value % 10) as u8;
            value /= 10;
            count += 1;
        }
    }
    for index in (0..count).rev() {
        windemo_push_bytes(buffer, len, &digits[index..index + 1]);
    }
}

const DESKTOP_APP_COUNT: usize = 5;
const DESKTOP_APP_TERMINAL: usize = 0;
const DESKTOP_APP_FILES: usize = 1;
const DESKTOP_APP_MONITOR: usize = 2;
const DESKTOP_APP_NOTES: usize = 3;
const DESKTOP_APP_PAINT: usize = 4;
const DESKTOP_MAX_TASK_BUTTONS: usize = ui::MAX_WINDOWS;
const DESKTOP_PANEL_HEIGHT: i32 = 38;
const DESKTOP_START_BUTTON_WIDTH: i32 = 88;
const DESKTOP_CLOCK_WIDTH: i32 = 96;
const DESKTOP_TASK_BUTTON_GAP: i32 = 6;
const DESKTOP_TASK_BUTTON_MIN_WIDTH: i32 = 92;
const DESKTOP_TASK_BUTTON_MAX_WIDTH: i32 = 180;
const DESKTOP_MENU_WIDTH: i32 = 272;
const DESKTOP_MENU_HEADER_HEIGHT: i32 = 30;
const DESKTOP_MENU_ITEM_HEIGHT: i32 = 29;
const DESKTOP_FRAME_TICKS: u32 = 3;
const DESKTOP_TERMINAL_MAX_LINES: usize = 128;
const DESKTOP_TERMINAL_LINE_LEN: usize = 112;
const DESKTOP_FILES_MAX: usize = 64;
const DESKTOP_FILES_PREVIEW_MAX: usize = 2048;
const DESKTOP_STATUS_TEXT_MAX: usize = 96;
const DESKTOP_NOTES_MAX_LINES: usize = 128;
const DESKTOP_NOTES_LINE_LEN: usize = 120;
const DESKTOP_NOTES_FILE: &str = "notes.txt";
const DESKTOP_NOTES_SAVE_MAX: usize = 8192;
const DESKTOP_MONITOR_HISTORY: usize = 72;
const DESKTOP_PAINT_CANVAS_W: usize = 128;
const DESKTOP_PAINT_CANVAS_H: usize = 80;
const DESKTOP_PAINT_CANVAS_PIXELS: usize = DESKTOP_PAINT_CANVAS_W * DESKTOP_PAINT_CANVAS_H;

#[derive(Clone, Copy)]
struct DesktopAppSpec {
    key: &'static str,
    name: &'static str,
    description: &'static str,
    title: &'static str,
    rect: ui::Rect,
    min_width: i32,
    min_height: i32,
    background: u32,
    accent: u32,
    fill_a: u32,
    fill_b: u32,
    stripe: u32,
}

const DESKTOP_APP_REGISTRY: [DesktopAppSpec; DESKTOP_APP_COUNT] = [
    DesktopAppSpec {
        key: "terminal",
        name: "Terminal",
        description: "shell session",
        title: "Terminal",
        rect: ui::Rect::new(48, 44, 428, 266),
        min_width: 220,
        min_height: 150,
        background: 0x091424,
        accent: 0x46D2FF,
        fill_a: 0x0F2A44,
        fill_b: 0x071325,
        stripe: 0x204D73,
    },
    DesktopAppSpec {
        key: "files",
        name: "Files",
        description: "project browser",
        title: "File Browser",
        rect: ui::Rect::new(152, 74, 416, 252),
        min_width: 220,
        min_height: 150,
        background: 0x111E12,
        accent: 0x57D76B,
        fill_a: 0x1A331D,
        fill_b: 0x0D1C0F,
        stripe: 0x2D6A36,
    },
    DesktopAppSpec {
        key: "monitor",
        name: "Monitor",
        description: "system metrics",
        title: "System Monitor",
        rect: ui::Rect::new(236, 92, 392, 246),
        min_width: 220,
        min_height: 150,
        background: 0x1A140C,
        accent: 0xFFBF55,
        fill_a: 0x382610,
        fill_b: 0x1A1208,
        stripe: 0x8D5E20,
    },
    DesktopAppSpec {
        key: "notes",
        name: "Notes",
        description: "quick notes",
        title: "Notes",
        rect: ui::Rect::new(92, 152, 360, 236),
        min_width: 220,
        min_height: 150,
        background: 0x1F101F,
        accent: 0xE58BFF,
        fill_a: 0x331A35,
        fill_b: 0x1A0D1B,
        stripe: 0x7E4390,
    },
    DesktopAppSpec {
        key: "paint",
        name: "Paint",
        description: "pixel board",
        title: "Pixel Paint",
        rect: ui::Rect::new(202, 142, 382, 236),
        min_width: 220,
        min_height: 150,
        background: 0x100F26,
        accent: 0x8E9BFF,
        fill_a: 0x1C2253,
        fill_b: 0x0C1030,
        stripe: 0x384FA6,
    },
];

#[derive(Clone, Copy)]
struct DesktopTaskButton {
    window_id: ui::WindowId,
    title: &'static str,
    rect: ui::Rect,
    focused: bool,
    minimized: bool,
}

const EMPTY_DESKTOP_TASK_BUTTON: DesktopTaskButton = DesktopTaskButton {
    window_id: 0,
    title: "",
    rect: ui::Rect::new(0, 0, 0, 0),
    focused: false,
    minimized: false,
};

#[derive(Clone, Copy)]
struct DesktopLauncherItem {
    app_index: usize,
    rect: ui::Rect,
    running: bool,
}

const EMPTY_DESKTOP_LAUNCHER_ITEM: DesktopLauncherItem = DesktopLauncherItem {
    app_index: 0,
    rect: ui::Rect::new(0, 0, 0, 0),
    running: false,
};

struct DesktopLayout {
    desktop_bounds: ui::Rect,
    panel_rect: ui::Rect,
    start_button: ui::Rect,
    clock_rect: ui::Rect,
    task_buttons: [DesktopTaskButton; DESKTOP_MAX_TASK_BUTTONS],
    task_button_count: usize,
    launcher_rect: ui::Rect,
    launcher_items: [DesktopLauncherItem; DESKTOP_APP_COUNT],
    launcher_item_count: usize,
}

#[derive(Clone, Copy)]
struct DesktopClock {
    hour: u8,
    minute: u8,
    second: u8,
}

fn handle_desktop_command() {
    let Some((fb_width, fb_height)) = vga::framebuffer_resolution() else {
        shell_println!("desktop requires VBE/framebuffer mode");
        return;
    };

    let width = fb_width.min(i32::MAX as usize) as i32;
    let height = fb_height.min(i32::MAX as usize) as i32;
    if width <= 0 || height <= 0 {
        shell_println!("desktop: invalid framebuffer size");
        return;
    }

    if width < 760 || height < 460 {
        shell_println!("desktop: framebuffer too small (need at least 760x460)");
        return;
    }

    shell_println!("desktop: taskbar + launcher shell (Shift+Q exits, Shift+S toggles start menu)");
    for _ in 0..512 {
        if input::pop_event().is_none() {
            break;
        }
    }

    let panel_height = DESKTOP_PANEL_HEIGHT.min((height / 2).max(30));
    let desktop_bounds = ui::Rect::new(0, 0, width, height.saturating_sub(panel_height).max(1));
    let mut manager = ui::WindowManager::new(0x081423);
    let mut running_windows = [None; DESKTOP_APP_COUNT];
    let mut launch_serial = 0u32;

    let _ = desktop_launch_app(
        &mut manager,
        &mut running_windows,
        DESKTOP_APP_TERMINAL,
        &mut launch_serial,
        desktop_bounds,
    );
    let _ = desktop_launch_app(
        &mut manager,
        &mut running_windows,
        DESKTOP_APP_MONITOR,
        &mut launch_serial,
        desktop_bounds,
    );

    let mut launcher_open = false;
    let mut layout = desktop_compute_layout(&manager, width, height, launcher_open, &running_windows);
    let mut last_clock_second = desktop_clock_second_key();
    let mut last_frame_tick = timer::ticks().wrapping_sub(DESKTOP_FRAME_TICKS);

    let start_tick = timer::ticks();
    let mut apps = DesktopApps::new(start_tick);
    if running_windows[DESKTOP_APP_TERMINAL].is_some() {
        apps.on_app_launched(DESKTOP_APP_TERMINAL);
    }
    if running_windows[DESKTOP_APP_MONITOR].is_some() {
        apps.on_app_launched(DESKTOP_APP_MONITOR);
    }

    desktop_paint_running_windows(&mut manager, &running_windows, &mut apps, start_tick);
    desktop_draw_scene(&manager, &layout, launcher_open, start_tick);

    'desktop: loop {
        let mut redraw = false;
        let mut processed = 0usize;

        for _ in 0..128 {
            let Some(event) = input::pop_event() else {
                break;
            };
            processed += 1;

            let mut consumed = false;

            if let InputEvent::MouseDown {
                button: MouseButton::Left,
                x,
                y,
            } = event
            {
                layout = desktop_compute_layout(&manager, width, height, launcher_open, &running_windows);

                if layout.start_button.contains(x, y) {
                    launcher_open = !launcher_open;
                    redraw = true;
                    consumed = true;
                } else if let Some(window_id) = desktop_hit_task_button(&layout, x, y) {
                    let _ = manager.activate_window(window_id);
                    launcher_open = false;
                    redraw = true;
                    consumed = true;
                } else if launcher_open {
                    if let Some(app_index) = desktop_hit_launcher_item(&layout, x, y) {
                        let launched = desktop_launch_app(
                            &mut manager,
                            &mut running_windows,
                            app_index,
                            &mut launch_serial,
                            desktop_bounds,
                        );
                        launcher_open = false;
                        if launched {
                            apps.on_app_launched(app_index);
                            redraw = true;
                        } else {
                            redraw = true;
                            shell_println!(
                                "desktop: failed to launch {}",
                                DESKTOP_APP_REGISTRY[app_index].name
                            );
                        }
                        consumed = true;
                    } else if layout.launcher_rect.contains(x, y) {
                        consumed = true;
                    } else {
                        launcher_open = false;
                        redraw = true;
                    }
                }
            }

            if !consumed {
                let response = manager.handle_event(event, desktop_bounds);
                if response.redraw {
                    redraw = true;
                }
                if let Some(closed_id) = response.closed {
                    desktop_unregister_closed_window(&mut running_windows, closed_id);
                    apps.on_window_closed(closed_id);
                    redraw = true;
                }

                let app_redraw = desktop_handle_app_event(
                    &mut apps,
                    &manager,
                    &running_windows,
                    event,
                    timer::ticks(),
                );
                if app_redraw {
                    redraw = true;
                }

                if !app_redraw {
                    match event {
                        InputEvent::KeyPress {
                            key: KeyEvent::Char('Q'),
                        } => break 'desktop,
                        InputEvent::KeyPress {
                            key: KeyEvent::Char('S'),
                        } => {
                            launcher_open = !launcher_open;
                            redraw = true;
                        }
                        _ => {}
                    }
                }
            }
        }

        let clock_second = desktop_clock_second_key();
        if clock_second != last_clock_second {
            last_clock_second = clock_second;
            redraw = true;
        }

        let now = timer::ticks();
        let frame_due = now.wrapping_sub(last_frame_tick) >= DESKTOP_FRAME_TICKS;
        if frame_due {
            last_frame_tick = now;
            redraw = true;
        }

        if redraw {
            let tick = if frame_due { now } else { timer::ticks() };
            desktop_paint_running_windows(&mut manager, &running_windows, &mut apps, tick);
            layout = desktop_compute_layout(&manager, width, height, launcher_open, &running_windows);
            desktop_draw_scene(&manager, &layout, launcher_open, tick);
        }

        if processed == 0 {
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
            }
        }
    }

    vga::clear_screen();
}

fn desktop_launch_app(
    manager: &mut ui::WindowManager,
    running_windows: &mut [Option<ui::WindowId>; DESKTOP_APP_COUNT],
    app_index: usize,
    launch_serial: &mut u32,
    desktop_bounds: ui::Rect,
) -> bool {
    if app_index >= DESKTOP_APP_REGISTRY.len() {
        return false;
    }

    if let Some(existing_id) = running_windows[app_index] {
        if manager.focus_window(existing_id) {
            return true;
        }
        running_windows[app_index] = None;
    }

    let app = DESKTOP_APP_REGISTRY[app_index];
    let mut rect = app.rect;
    let offset = ((*launch_serial % 6) as i32).saturating_mul(18);
    rect.x = rect.x.saturating_add(offset);
    rect.y = rect.y.saturating_add(offset / 2);
    rect = desktop_clamp_window_rect(rect, desktop_bounds, app.min_width, app.min_height);

    let spec = ui::WindowSpec {
        title: app.title,
        rect,
        min_width: app.min_width,
        min_height: app.min_height,
        background: app.background,
        accent: app.accent,
    };

    let Ok(id) = manager.add_window(spec) else {
        return false;
    };

    running_windows[app_index] = Some(id);
    *launch_serial = launch_serial.wrapping_add(1);
    true
}

fn desktop_clamp_window_rect(
    mut rect: ui::Rect,
    desktop_bounds: ui::Rect,
    min_width: i32,
    min_height: i32,
) -> ui::Rect {
    let hard_min_w = min_width.max(120);
    let hard_min_h = min_height.max(96);
    let max_w = desktop_bounds.width.saturating_sub(12).max(hard_min_w);
    let max_h = desktop_bounds.height.saturating_sub(12).max(hard_min_h);

    rect.width = rect.width.max(hard_min_w).min(max_w);
    rect.height = rect.height.max(hard_min_h).min(max_h);

    let min_x = desktop_bounds.x.saturating_add(6);
    let max_x = desktop_bounds
        .x
        .saturating_add(desktop_bounds.width)
        .saturating_sub(rect.width)
        .saturating_sub(6);
    if max_x >= min_x {
        rect.x = rect.x.clamp(min_x, max_x);
    } else {
        rect.x = desktop_bounds.x;
    }

    let min_y = desktop_bounds.y.saturating_add(6);
    let max_y = desktop_bounds
        .y
        .saturating_add(desktop_bounds.height)
        .saturating_sub(rect.height)
        .saturating_sub(6);
    if max_y >= min_y {
        rect.y = rect.y.clamp(min_y, max_y);
    } else {
        rect.y = desktop_bounds.y;
    }

    rect
}

fn desktop_unregister_closed_window(
    running_windows: &mut [Option<ui::WindowId>; DESKTOP_APP_COUNT],
    closed: ui::WindowId,
) {
    for window in running_windows.iter_mut() {
        if *window == Some(closed) {
            *window = None;
        }
    }
}

fn desktop_compute_layout(
    manager: &ui::WindowManager,
    width: i32,
    height: i32,
    launcher_open: bool,
    running_windows: &[Option<ui::WindowId>; DESKTOP_APP_COUNT],
) -> DesktopLayout {
    let panel_height = DESKTOP_PANEL_HEIGHT.min((height / 2).max(30));
    let panel_y = height.saturating_sub(panel_height);
    let desktop_bounds = ui::Rect::new(0, 0, width, panel_y.max(1));
    let panel_rect = ui::Rect::new(0, panel_y, width, panel_height);
    let button_height = panel_height.saturating_sub(12).max(1);
    let start_button = ui::Rect::new(8, panel_y + 6, DESKTOP_START_BUTTON_WIDTH, button_height);
    let clock_x = width.saturating_sub(DESKTOP_CLOCK_WIDTH + 8).max(8);
    let clock_rect = ui::Rect::new(clock_x, panel_y + 6, DESKTOP_CLOCK_WIDTH, button_height);

    let mut layout = DesktopLayout {
        desktop_bounds,
        panel_rect,
        start_button,
        clock_rect,
        task_buttons: [EMPTY_DESKTOP_TASK_BUTTON; DESKTOP_MAX_TASK_BUTTONS],
        task_button_count: 0,
        launcher_rect: ui::Rect::new(0, 0, 0, 0),
        launcher_items: [EMPTY_DESKTOP_LAUNCHER_ITEM; DESKTOP_APP_COUNT],
        launcher_item_count: 0,
    };

    let mut summaries = [ui::WindowSummary::default(); DESKTOP_MAX_TASK_BUTTONS];
    let summary_count = manager.window_summaries(&mut summaries);

    let task_x = start_button
        .x
        .saturating_add(start_button.width)
        .saturating_add(8);
    let task_end = clock_rect.x.saturating_sub(8);
    let task_available = task_end.saturating_sub(task_x);
    if task_available > DESKTOP_TASK_BUTTON_MIN_WIDTH && summary_count > 0 {
        let mut visible = summary_count.min(DESKTOP_MAX_TASK_BUTTONS);
        while visible > 0 {
            let needed = (visible as i32).saturating_mul(DESKTOP_TASK_BUTTON_MIN_WIDTH)
                + (visible.saturating_sub(1) as i32).saturating_mul(DESKTOP_TASK_BUTTON_GAP);
            if needed <= task_available {
                break;
            }
            visible -= 1;
        }

        if visible > 0 {
            let start_index = summary_count.saturating_sub(visible);
            let gap_total = (visible.saturating_sub(1) as i32).saturating_mul(DESKTOP_TASK_BUTTON_GAP);
            let button_width = ((task_available.saturating_sub(gap_total)) / (visible as i32))
                .clamp(DESKTOP_TASK_BUTTON_MIN_WIDTH, DESKTOP_TASK_BUTTON_MAX_WIDTH);
            let mut x = task_x;

            for offset in 0..visible {
                if x.saturating_add(button_width) > task_end {
                    break;
                }
                let summary = summaries[start_index + offset];
                layout.task_buttons[layout.task_button_count] = DesktopTaskButton {
                    window_id: summary.id,
                    title: summary.title,
                    rect: ui::Rect::new(x, panel_y + 6, button_width, button_height),
                    focused: summary.focused,
                    minimized: summary.minimized,
                };
                layout.task_button_count += 1;
                x = x.saturating_add(button_width + DESKTOP_TASK_BUTTON_GAP);
            }
        }
    }

    if launcher_open {
        let menu_width = DESKTOP_MENU_WIDTH.min(width.saturating_sub(16)).max(180);
        let menu_height = DESKTOP_MENU_HEADER_HEIGHT
            .saturating_add((DESKTOP_APP_COUNT as i32).saturating_mul(DESKTOP_MENU_ITEM_HEIGHT))
            .saturating_add(8);
        let menu_x = start_button
            .x
            .min(width.saturating_sub(menu_width + 8))
            .max(8);
        let menu_y = panel_y.saturating_sub(menu_height + 8).max(8);
        layout.launcher_rect = ui::Rect::new(menu_x, menu_y, menu_width, menu_height);

        let mut item_y = menu_y.saturating_add(DESKTOP_MENU_HEADER_HEIGHT);
        let item_x = menu_x.saturating_add(6);
        let item_w = menu_width.saturating_sub(12).max(1);
        for index in 0..DESKTOP_APP_COUNT {
            layout.launcher_items[index] = DesktopLauncherItem {
                app_index: index,
                rect: ui::Rect::new(
                    item_x,
                    item_y,
                    item_w,
                    DESKTOP_MENU_ITEM_HEIGHT.saturating_sub(2).max(1),
                ),
                running: running_windows[index].is_some(),
            };
            layout.launcher_item_count += 1;
            item_y = item_y.saturating_add(DESKTOP_MENU_ITEM_HEIGHT);
        }
    }

    layout
}

fn desktop_hit_task_button(layout: &DesktopLayout, x: i32, y: i32) -> Option<ui::WindowId> {
    for button in layout.task_buttons[..layout.task_button_count].iter().rev() {
        if button.rect.contains(x, y) {
            return Some(button.window_id);
        }
    }
    None
}

fn desktop_hit_launcher_item(layout: &DesktopLayout, x: i32, y: i32) -> Option<usize> {
    for item in layout.launcher_items[..layout.launcher_item_count].iter() {
        if item.rect.contains(x, y) {
            return Some(item.app_index);
        }
    }
    None
}

const DESKTOP_TEXT_SCRATCH: usize = 192;
const DESKTOP_PAINT_PALETTE_COUNT: usize = 10;
const DESKTOP_PAINT_PALETTE: [u32; DESKTOP_PAINT_PALETTE_COUNT] = [
    0x111111, 0xF8F8F8, 0xE43F5A, 0xF9A03F, 0xFFE66D, 0x2EC4B6, 0x3A86FF, 0x6A4C93, 0x8AC926,
    0xFF8FAB,
];
const EMPTY_DESKTOP_RECT: ui::Rect = ui::Rect::new(0, 0, 0, 0);

#[derive(Clone, Copy)]
struct DesktopFilesLayout {
    refresh_button: ui::Rect,
    delete_button: ui::Rect,
    list_rect: ui::Rect,
    preview_rect: ui::Rect,
    status_rect: ui::Rect,
}

#[derive(Clone, Copy)]
struct DesktopNotesLayout {
    save_button: ui::Rect,
    load_button: ui::Rect,
    clear_button: ui::Rect,
    editor_rect: ui::Rect,
    status_rect: ui::Rect,
}

#[derive(Clone, Copy)]
struct DesktopPaintLayout {
    palette: [ui::Rect; DESKTOP_PAINT_PALETTE_COUNT],
    clear_button: ui::Rect,
    canvas_rect: ui::Rect,
    scale: i32,
}

struct DesktopApps {
    terminal: DesktopTerminalState,
    files: DesktopFilesState,
    monitor: DesktopMonitorState,
    notes: DesktopNotesState,
    paint: DesktopPaintState,
}

impl DesktopApps {
    fn new(start_tick: u32) -> Self {
        Self {
            terminal: DesktopTerminalState::new(),
            files: DesktopFilesState::new(),
            monitor: DesktopMonitorState::new(start_tick),
            notes: DesktopNotesState::new(),
            paint: DesktopPaintState::new(),
        }
    }

    fn on_app_launched(&mut self, app_index: usize) {
        match app_index {
            DESKTOP_APP_TERMINAL => self.terminal.push_line(b"session attached"),
            DESKTOP_APP_FILES => self.files.refresh(),
            DESKTOP_APP_MONITOR => self.monitor.reset(),
            DESKTOP_APP_NOTES => {}
            DESKTOP_APP_PAINT => {
                let _ = self.paint.end_stroke();
            }
            _ => {}
        }
    }

    fn on_window_closed(&mut self, id: ui::WindowId) {
        self.paint.on_window_closed(id);
    }
}

fn desktop_window_app_index(
    running_windows: &[Option<ui::WindowId>; DESKTOP_APP_COUNT],
    id: ui::WindowId,
) -> Option<usize> {
    for (index, window_id) in running_windows.iter().enumerate() {
        if *window_id == Some(id) {
            return Some(index);
        }
    }
    None
}

fn desktop_handle_app_event(
    apps: &mut DesktopApps,
    manager: &ui::WindowManager,
    running_windows: &[Option<ui::WindowId>; DESKTOP_APP_COUNT],
    event: InputEvent,
    _tick: u32,
) -> bool {
    let mut redraw = false;

    if matches!(
        event,
        InputEvent::MouseUp {
            button: MouseButton::Left,
            ..
        }
    ) {
        redraw |= apps.paint.end_stroke();
    }

    let Some(focused_id) = manager.focused_window() else {
        return redraw;
    };
    let Some(app_index) = desktop_window_app_index(running_windows, focused_id) else {
        return redraw;
    };
    let Some(client_rect) = manager.window_client_rect(focused_id) else {
        return redraw;
    };

    match app_index {
        DESKTOP_APP_TERMINAL => {
            if let InputEvent::KeyPress { key } = event {
                redraw |= apps.terminal.handle_key(key);
            }
        }
        DESKTOP_APP_FILES => {
            redraw |= apps.files.handle_event(event, client_rect);
        }
        DESKTOP_APP_MONITOR => {
            if let InputEvent::KeyPress { key } = event {
                redraw |= apps.monitor.handle_key(key);
            }
        }
        DESKTOP_APP_NOTES => {
            redraw |= apps.notes.handle_event(event, client_rect);
        }
        DESKTOP_APP_PAINT => {
            redraw |= apps.paint.handle_event(event, focused_id, client_rect);
        }
        _ => {}
    }

    redraw
}

fn desktop_files_layout(width: usize, height: usize) -> DesktopFilesLayout {
    let w = width as i32;
    let h = height as i32;
    let button_h = 18;
    let font_h = desktop_font_height().max(8);
    let status_h = (font_h + 8).max(18);
    let refresh_button = ui::Rect::new(8, 6, 72, button_h);
    let delete_button = ui::Rect::new(86, 6, 72, button_h);

    let list_y = 30;
    let status_y = h.saturating_sub(status_h + 6).max(list_y + 2);
    let status_rect = ui::Rect::new(8, status_y, w.saturating_sub(16).max(1), status_h);
    let list_h = status_y.saturating_sub(list_y + 6).max(1);
    let list_w = ((w / 2).saturating_sub(12)).max(90);
    let list_rect = ui::Rect::new(8, list_y, list_w, list_h);

    let preview_x = list_rect.x.saturating_add(list_rect.width).saturating_add(8);
    let preview_w = w.saturating_sub(preview_x + 8).max(1);
    let preview_rect = ui::Rect::new(preview_x, list_y, preview_w, list_h);

    DesktopFilesLayout {
        refresh_button,
        delete_button,
        list_rect,
        preview_rect,
        status_rect,
    }
}

fn desktop_notes_layout(width: usize, height: usize) -> DesktopNotesLayout {
    let w = width as i32;
    let h = height as i32;
    let button_h = 18;
    let font_h = desktop_font_height().max(8);
    let status_h = (font_h + 8).max(18);
    let save_button = ui::Rect::new(8, 6, 56, button_h);
    let load_button = ui::Rect::new(70, 6, 56, button_h);
    let clear_button = ui::Rect::new(132, 6, 56, button_h);
    let editor_y = 34;
    let status_y = h.saturating_sub(status_h + 6).max(editor_y + 2);
    let status_rect = ui::Rect::new(8, status_y, w.saturating_sub(16).max(1), status_h);
    let editor_rect = ui::Rect::new(
        8,
        editor_y,
        w.saturating_sub(16).max(1),
        status_y.saturating_sub(editor_y + 6).max(1),
    );

    DesktopNotesLayout {
        save_button,
        load_button,
        clear_button,
        editor_rect,
        status_rect,
    }
}

fn desktop_paint_layout(width: usize, height: usize) -> DesktopPaintLayout {
    let mut palette = [EMPTY_DESKTOP_RECT; DESKTOP_PAINT_PALETTE_COUNT];
    let swatch_size = 16;
    let swatch_gap = 4;
    let mut swatch_x = 8;
    for slot in 0..DESKTOP_PAINT_PALETTE_COUNT {
        palette[slot] = ui::Rect::new(swatch_x, 6, swatch_size, swatch_size);
        swatch_x = swatch_x.saturating_add(swatch_size + swatch_gap);
    }

    let clear_button = ui::Rect::new((width as i32).saturating_sub(68).max(8), 6, 60, 18);
    let available_w = (width as i32).saturating_sub(16).max(1);
    let available_h = (height as i32).saturating_sub(36).max(1);
    let scale_x = (available_w / DESKTOP_PAINT_CANVAS_W as i32).max(1);
    let scale_y = (available_h / DESKTOP_PAINT_CANVAS_H as i32).max(1);
    let scale = scale_x.min(scale_y).max(1);

    let canvas_w = (DESKTOP_PAINT_CANVAS_W as i32).saturating_mul(scale);
    let canvas_h = (DESKTOP_PAINT_CANVAS_H as i32).saturating_mul(scale);
    let canvas_x = ((width as i32).saturating_sub(canvas_w) / 2).max(8);
    let canvas_y = 30
        + ((height as i32)
            .saturating_sub(30)
            .saturating_sub(canvas_h)
            .max(0)
            / 2);

    DesktopPaintLayout {
        palette,
        clear_button,
        canvas_rect: ui::Rect::new(canvas_x, canvas_y, canvas_w, canvas_h),
        scale,
    }
}

fn desktop_mouse_local(event: InputEvent, rect: ui::Rect) -> Option<(i32, i32)> {
    match event {
        InputEvent::MouseDown { x, y, .. }
        | InputEvent::MouseUp { x, y, .. }
        | InputEvent::MouseClick { x, y, .. }
        | InputEvent::MouseMove { x, y, .. } => {
            if rect.contains(x, y) {
                Some((x.saturating_sub(rect.x), y.saturating_sub(rect.y)))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn desktop_font_height() -> i32 {
    vga::font_metrics().map_or(16, |(_, h)| h.max(1)) as i32
}

fn desktop_draw_text(pixels: &mut [u32], width: usize, height: usize, x: i32, y: i32, text: &str, fg: u32, bg: u32) {
    let _ = vga::draw_text_bitmap(pixels, width, height, width, x, y, text, fg, bg);
}

fn desktop_draw_text_bytes(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    bytes: &[u8],
    fg: u32,
    bg: u32,
) {
    if let Ok(text) = str::from_utf8(bytes) {
        desktop_draw_text(pixels, width, height, x, y, text, fg, bg);
        return;
    }

    let mut sanitized = [0u8; DESKTOP_TEXT_SCRATCH];
    let copy_len = bytes.len().min(sanitized.len());
    for (index, byte) in bytes.iter().copied().take(copy_len).enumerate() {
        sanitized[index] = sanitize_editor_byte(byte);
    }
    if let Ok(text) = str::from_utf8(&sanitized[..copy_len]) {
        desktop_draw_text(pixels, width, height, x, y, text, fg, bg);
    }
}

fn desktop_draw_border(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    rect: ui::Rect,
    color: u32,
) {
    if rect.width <= 0 || rect.height <= 0 {
        return;
    }
    multdemo_fill_rect(pixels, width, height, rect.x, rect.y, rect.width, 1, color);
    multdemo_fill_rect(
        pixels,
        width,
        height,
        rect.x,
        rect.y.saturating_add(rect.height).saturating_sub(1),
        rect.width,
        1,
        color,
    );
    multdemo_fill_rect(pixels, width, height, rect.x, rect.y, 1, rect.height, color);
    multdemo_fill_rect(
        pixels,
        width,
        height,
        rect.x.saturating_add(rect.width).saturating_sub(1),
        rect.y,
        1,
        rect.height,
        color,
    );
}

fn desktop_set_message<const N: usize>(buffer: &mut [u8; N], len: &mut usize, text: &str) {
    *len = 0;
    for byte in text.bytes() {
        if *len >= N {
            break;
        }
        buffer[*len] = sanitize_editor_byte(byte);
        *len += 1;
    }
}

fn desktop_copy_sanitized_ascii(dst: &mut [u8], src: &[u8]) -> usize {
    let mut written = 0usize;
    for byte in src.iter().copied() {
        if written >= dst.len() {
            break;
        }
        dst[written] = match byte {
            b'\n' | b'\r' | b'\t' => b' ',
            0x20..=0x7E => byte,
            _ => b'.',
        };
        written += 1;
    }
    written
}

struct DesktopTerminalState {
    lines: [[u8; DESKTOP_TERMINAL_LINE_LEN]; DESKTOP_TERMINAL_MAX_LINES],
    line_lens: [usize; DESKTOP_TERMINAL_MAX_LINES],
    head: usize,
    count: usize,
    input: [u8; DESKTOP_TERMINAL_LINE_LEN],
    input_len: usize,
}

impl DesktopTerminalState {
    fn new() -> Self {
        let mut state = Self {
            lines: [[0; DESKTOP_TERMINAL_LINE_LEN]; DESKTOP_TERMINAL_MAX_LINES],
            line_lens: [0; DESKTOP_TERMINAL_MAX_LINES],
            head: 0,
            count: 0,
            input: [0; DESKTOP_TERMINAL_LINE_LEN],
            input_len: 0,
        };
        state.push_line(b"codexOS desktop terminal");
        state.push_line(b"commands: help clear echo time uptime heap fsls fscat");
        state
    }

    fn oldest_index(&self) -> usize {
        if self.count < DESKTOP_TERMINAL_MAX_LINES {
            0
        } else {
            self.head
        }
    }

    fn line_at(&self, logical_index: usize) -> Option<&[u8]> {
        if logical_index >= self.count {
            return None;
        }
        let index = (self.oldest_index() + logical_index) % DESKTOP_TERMINAL_MAX_LINES;
        Some(&self.lines[index][..self.line_lens[index]])
    }

    fn clear_history(&mut self) {
        self.count = 0;
        self.head = 0;
    }

    fn push_line(&mut self, bytes: &[u8]) {
        let slot = self.head;
        let mut len = 0usize;
        for byte in bytes.iter().copied() {
            if len >= DESKTOP_TERMINAL_LINE_LEN {
                break;
            }
            self.lines[slot][len] = sanitize_editor_byte(byte);
            len += 1;
        }
        self.line_lens[slot] = len;
        self.head = (self.head + 1) % DESKTOP_TERMINAL_MAX_LINES;
        if self.count < DESKTOP_TERMINAL_MAX_LINES {
            self.count += 1;
        }
    }

    fn push_prompt_line(&mut self) {
        let mut line = [0u8; DESKTOP_TERMINAL_LINE_LEN];
        let mut len = 0usize;
        windemo_push_bytes(&mut line, &mut len, b"> ");
        windemo_push_bytes(&mut line, &mut len, &self.input[..self.input_len]);
        self.push_line(&line[..len]);
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key {
            KeyEvent::Char('\n') => {
                self.submit();
                true
            }
            KeyEvent::Char('\x08') => {
                if self.input_len == 0 {
                    return false;
                }
                self.input_len -= 1;
                true
            }
            KeyEvent::Char(ch) if is_printable(ch) => {
                if self.input_len >= self.input.len() {
                    return false;
                }
                self.input[self.input_len] = ch as u8;
                self.input_len += 1;
                true
            }
            _ => false,
        }
    }

    fn submit(&mut self) {
        self.push_prompt_line();

        let mut command = [0u8; DESKTOP_TERMINAL_LINE_LEN];
        let command_len = self.input_len;
        command[..command_len].copy_from_slice(&self.input[..command_len]);
        self.input_len = 0;

        if command_len == 0 {
            return;
        }

        let command_text = str::from_utf8(&command[..command_len]).unwrap_or("");
        let mut parts = command_text.split_whitespace();
        let Some(token) = parts.next() else {
            return;
        };

        match token {
            "help" => {
                self.push_line(b"help clear echo time uptime heap fsls fscat");
            }
            "clear" => {
                self.clear_history();
                self.push_line(b"terminal cleared");
            }
            "echo" => {
                let rest = command_text
                    .split_once(' ')
                    .map(|(_, tail)| tail)
                    .unwrap_or("");
                if rest.is_empty() {
                    self.push_line(b"(empty)");
                } else {
                    self.push_line(rest.as_bytes());
                }
            }
            "time" => {
                let clock = desktop_clock_now();
                let mut line = [0u8; DESKTOP_TERMINAL_LINE_LEN];
                let mut len = 0usize;
                windemo_push_bytes(&mut line, &mut len, b"time ");
                desktop_push_two_digits(&mut line, &mut len, clock.hour);
                windemo_push_bytes(&mut line, &mut len, b":");
                desktop_push_two_digits(&mut line, &mut len, clock.minute);
                windemo_push_bytes(&mut line, &mut len, b":");
                desktop_push_two_digits(&mut line, &mut len, clock.second);
                self.push_line(&line[..len]);
            }
            "uptime" => {
                let up = timer::uptime();
                let mut line = [0u8; DESKTOP_TERMINAL_LINE_LEN];
                let mut len = 0usize;
                windemo_push_bytes(&mut line, &mut len, b"uptime ");
                windemo_push_u32(&mut line, &mut len, up.seconds.min(u32::MAX as u64) as u32);
                windemo_push_bytes(&mut line, &mut len, b".");
                desktop_push_two_digits(&mut line, &mut len, (up.millis / 10) as u8);
                windemo_push_bytes(&mut line, &mut len, b"s");
                self.push_line(&line[..len]);
            }
            "heap" => {
                let heap = allocator::stats();
                let mut line = [0u8; DESKTOP_TERMINAL_LINE_LEN];
                let mut len = 0usize;
                windemo_push_bytes(&mut line, &mut len, b"heap used ");
                windemo_push_u32(&mut line, &mut len, heap.used.min(u32::MAX as usize) as u32);
                windemo_push_bytes(&mut line, &mut len, b" / ");
                windemo_push_u32(&mut line, &mut len, heap.total.min(u32::MAX as usize) as u32);
                windemo_push_bytes(&mut line, &mut len, b" bytes");
                self.push_line(&line[..len]);
            }
            "fsls" => {
                let mut files = [fs::FileInfo::empty(); 16];
                match fs::list(&mut files) {
                    Ok(count) => {
                        if count == 0 {
                            self.push_line(b"no files");
                        } else {
                            for file in files.iter().take(count) {
                                let mut line = [0u8; DESKTOP_TERMINAL_LINE_LEN];
                                let mut len = 0usize;
                                windemo_push_bytes(&mut line, &mut len, file.name_str().as_bytes());
                                windemo_push_bytes(&mut line, &mut len, b" (");
                                windemo_push_u32(&mut line, &mut len, file.size_bytes);
                                windemo_push_bytes(&mut line, &mut len, b"b)");
                                self.push_line(&line[..len]);
                            }
                        }
                    }
                    Err(error) => {
                        let mut line = [0u8; DESKTOP_TERMINAL_LINE_LEN];
                        let mut len = 0usize;
                        windemo_push_bytes(&mut line, &mut len, b"fsls: ");
                        windemo_push_bytes(&mut line, &mut len, error.as_str().as_bytes());
                        self.push_line(&line[..len]);
                    }
                }
            }
            "fscat" => {
                let Some(name) = parts.next() else {
                    self.push_line(b"usage: fscat <name>");
                    return;
                };

                let mut buffer = [0u8; 256];
                match fs::read_file(name, &mut buffer) {
                    Ok(result) => {
                        if result.copied_size == 0 {
                            self.push_line(b"<empty>");
                            return;
                        }

                        let mut offset = 0usize;
                        for _ in 0..3 {
                            if offset >= result.copied_size {
                                break;
                            }
                            let mut line = [0u8; DESKTOP_TERMINAL_LINE_LEN];
                            let mut len = 0usize;
                            while offset < result.copied_size && len < line.len() {
                                let byte = buffer[offset];
                                offset += 1;
                                if byte == b'\n' || byte == b'\r' {
                                    break;
                                }
                                line[len] = sanitize_editor_byte(byte);
                                len += 1;
                            }
                            self.push_line(&line[..len]);
                        }

                        if result.total_size > result.copied_size || offset < result.copied_size {
                            self.push_line(b"...");
                        }
                    }
                    Err(error) => {
                        let mut line = [0u8; DESKTOP_TERMINAL_LINE_LEN];
                        let mut len = 0usize;
                        windemo_push_bytes(&mut line, &mut len, b"fscat: ");
                        windemo_push_bytes(&mut line, &mut len, error.as_str().as_bytes());
                        self.push_line(&line[..len]);
                    }
                }
            }
            _ => {
                let mut line = [0u8; DESKTOP_TERMINAL_LINE_LEN];
                let mut len = 0usize;
                windemo_push_bytes(&mut line, &mut len, b"unknown command: ");
                windemo_push_bytes(&mut line, &mut len, token.as_bytes());
                self.push_line(&line[..len]);
            }
        }
    }

    fn draw(
        &mut self,
        pixels: &mut [u32],
        width: usize,
        height: usize,
        app: DesktopAppSpec,
        tick: u32,
        focused: bool,
    ) {
        if width == 0 || height == 0 {
            return;
        }

        multdemo_fill_gradient_vertical(pixels, width, height, app.fill_a, app.fill_b);
        desktop_draw_border(
            pixels,
            width,
            height,
            ui::Rect::new(0, 0, width as i32, height as i32),
            app.stripe,
        );

        multdemo_fill_rect(pixels, width, height, 0, 0, width as i32, 22, app.background);
        desktop_draw_text(
            pixels,
            width,
            height,
            8,
            6,
            "Terminal",
            0xEAF3FF,
            app.background,
        );

        let font_h = desktop_font_height().max(8);
        let line_start_y = 28;
        let input_y = (height as i32).saturating_sub(font_h + 6).max(line_start_y);
        let usable_h = input_y.saturating_sub(line_start_y);
        let visible_lines = (usable_h / font_h).max(0) as usize;
        let start = self.count.saturating_sub(visible_lines);

        for row in 0..visible_lines {
            let logical = start + row;
            let Some(line) = self.line_at(logical) else {
                continue;
            };
            let y = line_start_y + row as i32 * font_h;
            desktop_draw_text_bytes(pixels, width, height, 8, y, line, 0xD7E9FF, app.fill_b);
        }

        multdemo_fill_rect(pixels, width, height, 0, input_y - 2, width as i32, font_h + 6, 0x0A1A2E);
        let mut input_line = [0u8; DESKTOP_TERMINAL_LINE_LEN + 2];
        let mut input_len = 0usize;
        windemo_push_bytes(&mut input_line, &mut input_len, b"> ");
        windemo_push_bytes(&mut input_line, &mut input_len, &self.input[..self.input_len]);
        desktop_draw_text_bytes(
            pixels,
            width,
            height,
            8,
            input_y + 1,
            &input_line[..input_len],
            0xF4FAFF,
            0x0A1A2E,
        );

        let show_cursor = focused && ((tick / 20) & 1) == 0;
        if show_cursor {
            let cursor_x = 8 + (input_len as i32 * 8);
            multdemo_fill_rect(
                pixels,
                width,
                height,
                cursor_x,
                input_y + 1,
                1,
                font_h,
                0x8BD8FF,
            );
        }
    }
}

struct DesktopFilesState {
    entries: [fs::FileInfo; DESKTOP_FILES_MAX],
    entry_count: usize,
    selected: Option<usize>,
    scroll: usize,
    preview: [u8; DESKTOP_FILES_PREVIEW_MAX],
    preview_len: usize,
    status: [u8; DESKTOP_STATUS_TEXT_MAX],
    status_len: usize,
}

impl DesktopFilesState {
    fn new() -> Self {
        let mut state = Self {
            entries: [fs::FileInfo::empty(); DESKTOP_FILES_MAX],
            entry_count: 0,
            selected: None,
            scroll: 0,
            preview: [0; DESKTOP_FILES_PREVIEW_MAX],
            preview_len: 0,
            status: [0; DESKTOP_STATUS_TEXT_MAX],
            status_len: 0,
        };
        state.refresh();
        state
    }

    fn set_status(&mut self, text: &str) {
        desktop_set_message(&mut self.status, &mut self.status_len, text);
    }

    fn refresh(&mut self) {
        match fs::list(&mut self.entries) {
            Ok(count) => {
                self.entry_count = count.min(self.entries.len());
                if self.entry_count == 0 {
                    self.selected = None;
                    self.preview_len = 0;
                    self.scroll = 0;
                    self.set_status("no files");
                    return;
                }

                if self.selected.is_none_or(|index| index >= self.entry_count) {
                    self.selected = Some(0);
                }
                let mut line = [0u8; DESKTOP_STATUS_TEXT_MAX];
                let mut len = 0usize;
                windemo_push_bytes(&mut line, &mut len, b"files: ");
                windemo_push_u32(&mut line, &mut len, self.entry_count as u32);
                if let Ok(text) = str::from_utf8(&line[..len]) {
                    self.set_status(text);
                }
                if let Some(index) = self.selected {
                    self.load_preview(index);
                }
            }
            Err(error) => {
                self.entry_count = 0;
                self.selected = None;
                self.preview_len = 0;
                self.scroll = 0;
                let mut line = [0u8; DESKTOP_STATUS_TEXT_MAX];
                let mut len = 0usize;
                windemo_push_bytes(&mut line, &mut len, b"fs error: ");
                windemo_push_bytes(&mut line, &mut len, error.as_str().as_bytes());
                if let Ok(text) = str::from_utf8(&line[..len]) {
                    self.set_status(text);
                }
            }
        }
    }

    fn clamp_scroll(&mut self, visible_rows: usize) {
        let visible = visible_rows.max(1);
        if self.selected.is_some_and(|selected| selected < self.scroll) {
            self.scroll = self.selected.unwrap_or(0);
        }
        if let Some(selected) = self.selected {
            if selected >= self.scroll.saturating_add(visible) {
                self.scroll = selected.saturating_add(1).saturating_sub(visible);
            }
        }

        if self.scroll >= self.entry_count {
            self.scroll = self.entry_count.saturating_sub(1);
        }
    }

    fn select(&mut self, index: usize) {
        if index >= self.entry_count {
            return;
        }
        self.selected = Some(index);
        self.load_preview(index);
    }

    fn load_preview(&mut self, index: usize) {
        self.preview_len = 0;
        if index >= self.entry_count {
            return;
        }

        let mut buffer = [0u8; DESKTOP_FILES_PREVIEW_MAX];
        match fs::read_file(self.entries[index].name_str(), &mut buffer) {
            Ok(result) => {
                self.preview_len = desktop_copy_sanitized_ascii(
                    &mut self.preview,
                    &buffer[..result.copied_size.min(buffer.len())],
                );
                let mut line = [0u8; DESKTOP_STATUS_TEXT_MAX];
                let mut len = 0usize;
                windemo_push_bytes(&mut line, &mut len, b"preview ");
                windemo_push_u32(&mut line, &mut len, result.total_size.min(u32::MAX as usize) as u32);
                windemo_push_bytes(&mut line, &mut len, b" bytes");
                if let Ok(text) = str::from_utf8(&line[..len]) {
                    self.set_status(text);
                }
            }
            Err(error) => {
                let mut line = [0u8; DESKTOP_STATUS_TEXT_MAX];
                let mut len = 0usize;
                windemo_push_bytes(&mut line, &mut len, b"read failed: ");
                windemo_push_bytes(&mut line, &mut len, error.as_str().as_bytes());
                if let Ok(text) = str::from_utf8(&line[..len]) {
                    self.set_status(text);
                }
            }
        }
    }

    fn delete_selected(&mut self) {
        let Some(index) = self.selected else {
            self.set_status("select a file first");
            return;
        };
        if index >= self.entry_count {
            self.selected = None;
            self.set_status("select a file first");
            return;
        }

        let delete_result = fs::delete_file(self.entries[index].name_str());
        match delete_result {
            Ok(()) => {
                self.refresh();
                self.set_status("file deleted");
            }
            Err(error) => {
                let mut line = [0u8; DESKTOP_STATUS_TEXT_MAX];
                let mut len = 0usize;
                windemo_push_bytes(&mut line, &mut len, b"delete failed: ");
                windemo_push_bytes(&mut line, &mut len, error.as_str().as_bytes());
                if let Ok(text) = str::from_utf8(&line[..len]) {
                    self.set_status(text);
                }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key {
            KeyEvent::Up => {
                if self.entry_count == 0 {
                    return false;
                }
                let current = self.selected.unwrap_or(0);
                let next = current.saturating_sub(1);
                self.select(next);
                true
            }
            KeyEvent::Down => {
                if self.entry_count == 0 {
                    return false;
                }
                let current = self.selected.unwrap_or(0);
                let next = (current + 1).min(self.entry_count.saturating_sub(1));
                self.select(next);
                true
            }
            KeyEvent::PageUp => {
                self.scroll = self.scroll.saturating_sub(6);
                true
            }
            KeyEvent::PageDown => {
                self.scroll = self.scroll.saturating_add(6);
                true
            }
            KeyEvent::Char('r') | KeyEvent::Char('R') => {
                self.refresh();
                true
            }
            KeyEvent::Char('d') | KeyEvent::Char('D') => {
                self.delete_selected();
                true
            }
            _ => false,
        }
    }

    fn handle_mouse_down(&mut self, local_x: i32, local_y: i32, width: usize, height: usize) -> bool {
        let layout = desktop_files_layout(width, height);
        if layout.refresh_button.contains(local_x, local_y) {
            self.refresh();
            return true;
        }
        if layout.delete_button.contains(local_x, local_y) {
            self.delete_selected();
            return true;
        }

        if !layout.list_rect.contains(local_x, local_y) {
            return false;
        }

        let font_h = desktop_font_height().max(8);
        let row_h = font_h + 2;
        let row = ((local_y.saturating_sub(layout.list_rect.y)) / row_h).max(0) as usize;
        let index = self.scroll.saturating_add(row);
        if index < self.entry_count {
            self.select(index);
            return true;
        }
        false
    }

    fn handle_event(&mut self, event: InputEvent, client_rect: ui::Rect) -> bool {
        match event {
            InputEvent::KeyPress { key } => self.handle_key(key),
            InputEvent::MouseDown {
                button: MouseButton::Left,
                ..
            } => {
                let Some((local_x, local_y)) = desktop_mouse_local(event, client_rect) else {
                    return false;
                };
                self.handle_mouse_down(
                    local_x,
                    local_y,
                    client_rect.width.max(0) as usize,
                    client_rect.height.max(0) as usize,
                )
            }
            _ => false,
        }
    }

    fn draw(
        &mut self,
        pixels: &mut [u32],
        width: usize,
        height: usize,
        app: DesktopAppSpec,
        _tick: u32,
        _focused: bool,
    ) {
        if width == 0 || height == 0 {
            return;
        }

        multdemo_fill_gradient_vertical(pixels, width, height, app.fill_a, app.fill_b);
        desktop_draw_border(
            pixels,
            width,
            height,
            ui::Rect::new(0, 0, width as i32, height as i32),
            app.stripe,
        );

        let layout = desktop_files_layout(width, height);
        multdemo_fill_rect(
            pixels,
            width,
            height,
            0,
            0,
            width as i32,
            26,
            app.background,
        );
        desktop_draw_border(pixels, width, height, layout.refresh_button, 0x6FA57A);
        desktop_draw_border(pixels, width, height, layout.delete_button, 0xB67777);
        desktop_draw_text(
            pixels,
            width,
            height,
            layout.refresh_button.x + 8,
            layout.refresh_button.y + 4,
            "Refresh",
            0xE7F4EA,
            app.background,
        );
        desktop_draw_text(
            pixels,
            width,
            height,
            layout.delete_button.x + 10,
            layout.delete_button.y + 4,
            "Delete",
            0xFFE4E4,
            app.background,
        );

        desktop_draw_border(pixels, width, height, layout.list_rect, 0x5B7F61);
        desktop_draw_border(pixels, width, height, layout.preview_rect, 0x5B7F61);
        multdemo_fill_rect(
            pixels,
            width,
            height,
            layout.list_rect.x + 1,
            layout.list_rect.y + 1,
            layout.list_rect.width.saturating_sub(2),
            layout.list_rect.height.saturating_sub(2),
            0x132617,
        );
        multdemo_fill_rect(
            pixels,
            width,
            height,
            layout.preview_rect.x + 1,
            layout.preview_rect.y + 1,
            layout.preview_rect.width.saturating_sub(2),
            layout.preview_rect.height.saturating_sub(2),
            0x122012,
        );

        let font_h = desktop_font_height().max(8);
        let row_h = font_h + 2;
        let visible_rows = (layout.list_rect.height / row_h).max(1) as usize;
        self.clamp_scroll(visible_rows);

        for row in 0..visible_rows {
            let index = self.scroll.saturating_add(row);
            if index >= self.entry_count {
                break;
            }
            let y = layout.list_rect.y + row as i32 * row_h;
            let selected = self.selected == Some(index);
            if selected {
                multdemo_fill_rect(
                    pixels,
                    width,
                    height,
                    layout.list_rect.x + 1,
                    y,
                    layout.list_rect.width.saturating_sub(2),
                    row_h,
                    0x254E2D,
                );
            }

            let file = self.entries[index];
            desktop_draw_text(
                pixels,
                width,
                height,
                layout.list_rect.x + 6,
                y + 1,
                file.name_str(),
                if selected { 0xF3FFF6 } else { 0xC5E4CB },
                if selected { 0x254E2D } else { 0x132617 },
            );
            let mut size = [0u8; 16];
            let mut size_len = 0usize;
            windemo_push_u32(&mut size, &mut size_len, file.size_bytes);
            windemo_push_bytes(&mut size, &mut size_len, b"b");
            let size_x = layout
                .list_rect
                .x
                .saturating_add(layout.list_rect.width)
                .saturating_sub(52);
            desktop_draw_text_bytes(
                pixels,
                width,
                height,
                size_x,
                y + 1,
                &size[..size_len],
                0x95BDA0,
                if selected { 0x254E2D } else { 0x132617 },
            );
        }

        desktop_draw_text(
            pixels,
            width,
            height,
            layout.preview_rect.x + 6,
            layout.preview_rect.y + 3,
            "Preview",
            0xE2F2E5,
            0x122012,
        );
        let preview_y = layout.preview_rect.y + font_h + 6;
        let preview_cols = ((layout.preview_rect.width.saturating_sub(10)) / 8).max(1) as usize;
        let preview_rows = ((layout.preview_rect.height.saturating_sub(font_h + 10)) / font_h).max(1) as usize;
        let mut offset = 0usize;
        for row in 0..preview_rows {
            if offset >= self.preview_len {
                break;
            }
            let chunk = (self.preview_len - offset).min(preview_cols);
            let y = preview_y + row as i32 * font_h;
            desktop_draw_text_bytes(
                pixels,
                width,
                height,
                layout.preview_rect.x + 6,
                y,
                &self.preview[offset..offset + chunk],
                0xB9D3BE,
                0x122012,
            );
            offset += chunk;
        }

        multdemo_fill_rect(
            pixels,
            width,
            height,
            layout.status_rect.x,
            layout.status_rect.y,
            layout.status_rect.width,
            layout.status_rect.height,
            0x142117,
        );
        desktop_draw_border(pixels, width, height, layout.status_rect, 0x5B7F61);

        if let Ok(text) = str::from_utf8(&self.status[..self.status_len]) {
            let max_chars = ((layout.status_rect.width.saturating_sub(12)) / 8).max(0) as usize;
            let clipped = desktop_trim_text(text, max_chars);
            desktop_draw_text(
                pixels,
                width,
                height,
                layout.status_rect.x + 6,
                layout.status_rect.y + 4,
                clipped,
                0xBEDBC5,
                0x142117,
            );
        }
    }
}

struct DesktopMonitorState {
    frame_ticks: [u32; DESKTOP_MONITOR_HISTORY],
    frame_head: usize,
    frame_count: usize,
    last_tick: u32,
}

impl DesktopMonitorState {
    fn new(start_tick: u32) -> Self {
        Self {
            frame_ticks: [1; DESKTOP_MONITOR_HISTORY],
            frame_head: 0,
            frame_count: 0,
            last_tick: start_tick,
        }
    }

    fn reset(&mut self) {
        self.frame_head = 0;
        self.frame_count = 0;
    }

    fn push_frame_delta(&mut self, delta: u32) {
        self.frame_ticks[self.frame_head] = delta.max(1);
        self.frame_head = (self.frame_head + 1) % DESKTOP_MONITOR_HISTORY;
        if self.frame_count < DESKTOP_MONITOR_HISTORY {
            self.frame_count += 1;
        }
    }

    fn frame_delta_at(&self, logical_index: usize) -> u32 {
        if logical_index >= self.frame_count {
            return 1;
        }
        let oldest = if self.frame_count < DESKTOP_MONITOR_HISTORY {
            0
        } else {
            self.frame_head
        };
        let index = (oldest + logical_index) % DESKTOP_MONITOR_HISTORY;
        self.frame_ticks[index].max(1)
    }

    fn sample(&mut self, tick: u32) {
        let delta = tick.wrapping_sub(self.last_tick).max(1);
        self.last_tick = tick;
        self.push_frame_delta(delta);
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if matches!(key, KeyEvent::Char('R')) {
            self.reset();
            return true;
        }
        false
    }

    fn draw(
        &mut self,
        pixels: &mut [u32],
        width: usize,
        height: usize,
        app: DesktopAppSpec,
        tick: u32,
        _focused: bool,
    ) {
        if width == 0 || height == 0 {
            return;
        }

        self.sample(tick);
        multdemo_fill_gradient_vertical(pixels, width, height, app.fill_a, app.fill_b);
        desktop_draw_border(
            pixels,
            width,
            height,
            ui::Rect::new(0, 0, width as i32, height as i32),
            app.stripe,
        );

        let panel = ui::Rect::new(8, 8, (width as i32).saturating_sub(16).max(1), 92);
        multdemo_fill_rect(
            pixels,
            width,
            height,
            panel.x,
            panel.y,
            panel.width,
            panel.height,
            0x21160D,
        );
        desktop_draw_border(pixels, width, height, panel, 0x8A6A42);

        let up = timer::uptime();
        let hz = timer::frequency_hz().max(1);
        let heap = allocator::stats();
        let fs_info = fs::info();
        let mouse_state = mouse::state();
        let latest_delta = self.frame_delta_at(self.frame_count.saturating_sub(1)).max(1);
        let fps = hz.saturating_div(latest_delta).max(1);
        let frame_ms = (latest_delta as u64).saturating_mul(1000) / hz as u64;

        let mut line = [0u8; 96];
        let mut len = 0usize;
        windemo_push_bytes(&mut line, &mut len, b"uptime ");
        windemo_push_u32(&mut line, &mut len, up.seconds.min(u32::MAX as u64) as u32);
        windemo_push_bytes(&mut line, &mut len, b"s  fps ");
        windemo_push_u32(&mut line, &mut len, fps);
        windemo_push_bytes(&mut line, &mut len, b"  frame ");
        windemo_push_u32(&mut line, &mut len, frame_ms.min(u32::MAX as u64) as u32);
        windemo_push_bytes(&mut line, &mut len, b"ms");
        desktop_draw_text_bytes(
            pixels,
            width,
            height,
            panel.x + 8,
            panel.y + 8,
            &line[..len],
            0xFFE6BD,
            0x21160D,
        );

        len = 0;
        windemo_push_bytes(&mut line, &mut len, b"heap ");
        windemo_push_u32(&mut line, &mut len, heap.used.min(u32::MAX as usize) as u32);
        windemo_push_bytes(&mut line, &mut len, b"/");
        windemo_push_u32(&mut line, &mut len, heap.total.min(u32::MAX as usize) as u32);
        windemo_push_bytes(&mut line, &mut len, b"  input drops ");
        windemo_push_u32(&mut line, &mut len, input::dropped_event_count());
        desktop_draw_text_bytes(
            pixels,
            width,
            height,
            panel.x + 8,
            panel.y + 24,
            &line[..len],
            0xE8D2AD,
            0x21160D,
        );

        len = 0;
        windemo_push_bytes(&mut line, &mut len, b"mouse ");
        windemo_push_i32(&mut line, &mut len, mouse_state.x);
        windemo_push_bytes(&mut line, &mut len, b",");
        windemo_push_i32(&mut line, &mut len, mouse_state.y);
        windemo_push_bytes(&mut line, &mut len, b"  fs ");
        windemo_push_bytes(
            &mut line,
            &mut len,
            if fs_info.mounted { b"mounted" } else { b"offline" },
        );
        windemo_push_bytes(&mut line, &mut len, b" files=");
        windemo_push_u32(&mut line, &mut len, fs_info.file_count);
        desktop_draw_text_bytes(
            pixels,
            width,
            height,
            panel.x + 8,
            panel.y + 40,
            &line[..len],
            0xDDBF93,
            0x21160D,
        );

        let graph = ui::Rect::new(
            8,
            panel.y.saturating_add(panel.height).saturating_add(8),
            (width as i32).saturating_sub(16).max(1),
            (height as i32)
                .saturating_sub(panel.y.saturating_add(panel.height).saturating_add(16))
                .max(1),
        );
        multdemo_fill_rect(pixels, width, height, graph.x, graph.y, graph.width, graph.height, 0x160E08);
        desktop_draw_border(pixels, width, height, graph, 0x74593A);

        let bars = (graph.width / 3).max(1) as usize;
        let samples = self.frame_count.min(bars);
        if samples == 0 {
            return;
        }

        let start = self.frame_count.saturating_sub(samples);
        for offset in 0..samples {
            let sample = self.frame_delta_at(start + offset).max(1);
            let fps = hz.saturating_div(sample).max(1);
            let bar_h = ((fps.min(hz) as u64 * graph.height.max(1) as u64) / hz.max(1) as u64) as i32;
            let x = graph.x + 2 + offset as i32 * 3;
            let y = graph.y.saturating_add(graph.height).saturating_sub(bar_h).saturating_sub(1);
            multdemo_fill_rect(
                pixels,
                width,
                height,
                x,
                y,
                2,
                bar_h.max(1),
                if fps > hz / 2 { 0xFFCD72 } else { 0xA46A2E },
            );
        }
    }
}

struct DesktopNotesState {
    lines: [[u8; DESKTOP_NOTES_LINE_LEN]; DESKTOP_NOTES_MAX_LINES],
    line_lens: [usize; DESKTOP_NOTES_MAX_LINES],
    line_count: usize,
    cursor_line: usize,
    cursor_col: usize,
    scroll: usize,
    status: [u8; DESKTOP_STATUS_TEXT_MAX],
    status_len: usize,
    dirty: bool,
}

impl DesktopNotesState {
    fn new() -> Self {
        let mut state = Self {
            lines: [[0; DESKTOP_NOTES_LINE_LEN]; DESKTOP_NOTES_MAX_LINES],
            line_lens: [0; DESKTOP_NOTES_MAX_LINES],
            line_count: 1,
            cursor_line: 0,
            cursor_col: 0,
            scroll: 0,
            status: [0; DESKTOP_STATUS_TEXT_MAX],
            status_len: 0,
            dirty: false,
        };
        state.load_from_disk();
        state
    }

    fn set_status(&mut self, text: &str) {
        desktop_set_message(&mut self.status, &mut self.status_len, text);
    }

    fn reset_document(&mut self) {
        self.lines = [[0; DESKTOP_NOTES_LINE_LEN]; DESKTOP_NOTES_MAX_LINES];
        self.line_lens = [0; DESKTOP_NOTES_MAX_LINES];
        self.line_count = 1;
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.scroll = 0;
        self.dirty = false;
    }

    fn ensure_cursor_bounds(&mut self) {
        if self.line_count == 0 {
            self.line_count = 1;
        }
        if self.cursor_line >= self.line_count {
            self.cursor_line = self.line_count - 1;
        }
        self.cursor_col = self.cursor_col.min(self.line_lens[self.cursor_line]);
    }

    fn load_from_disk(&mut self) {
        let mut buffer = [0u8; DESKTOP_NOTES_SAVE_MAX];
        match fs::read_file(DESKTOP_NOTES_FILE, &mut buffer) {
            Ok(result) => {
                self.reset_document();
                self.line_count = 0;
                let mut line = 0usize;
                let mut col = 0usize;
                for byte in buffer[..result.copied_size].iter().copied() {
                    if line >= DESKTOP_NOTES_MAX_LINES {
                        break;
                    }
                    match byte {
                        b'\r' => {}
                        b'\n' => {
                            self.line_lens[line] = col;
                            line += 1;
                            col = 0;
                        }
                        value => {
                            if col < DESKTOP_NOTES_LINE_LEN {
                                self.lines[line][col] = sanitize_editor_byte(value);
                                col += 1;
                            }
                        }
                    }
                }
                if line < DESKTOP_NOTES_MAX_LINES {
                    self.line_lens[line] = col;
                    line += 1;
                }
                self.line_count = line.max(1);
                self.cursor_line = 0;
                self.cursor_col = 0;
                self.scroll = 0;
                self.dirty = false;
                self.set_status("notes loaded");
            }
            Err(fs::FsError::NotFound) => {
                self.reset_document();
                self.set_status("new note");
            }
            Err(error) => {
                self.reset_document();
                let mut line = [0u8; DESKTOP_STATUS_TEXT_MAX];
                let mut len = 0usize;
                windemo_push_bytes(&mut line, &mut len, b"load failed: ");
                windemo_push_bytes(&mut line, &mut len, error.as_str().as_bytes());
                if let Ok(text) = str::from_utf8(&line[..len]) {
                    self.set_status(text);
                }
            }
        }
    }

    fn save_to_disk(&mut self) {
        let mut output = [0u8; DESKTOP_NOTES_SAVE_MAX];
        let mut cursor = 0usize;
        for line in 0..self.line_count {
            let len = self.line_lens[line];
            let Some(end) = cursor.checked_add(len) else {
                self.set_status("save failed: note too large");
                return;
            };
            if end > output.len() {
                self.set_status("save failed: note too large");
                return;
            }
            output[cursor..end].copy_from_slice(&self.lines[line][..len]);
            cursor = end;
            if line + 1 < self.line_count {
                if cursor >= output.len() {
                    self.set_status("save failed: note too large");
                    return;
                }
                output[cursor] = b'\n';
                cursor += 1;
            }
        }

        match fs::write_file(DESKTOP_NOTES_FILE, &output[..cursor]) {
            Ok(()) => {
                self.dirty = false;
                self.set_status("saved notes.txt");
            }
            Err(error) => {
                let mut line = [0u8; DESKTOP_STATUS_TEXT_MAX];
                let mut len = 0usize;
                windemo_push_bytes(&mut line, &mut len, b"save failed: ");
                windemo_push_bytes(&mut line, &mut len, error.as_str().as_bytes());
                if let Ok(text) = str::from_utf8(&line[..len]) {
                    self.set_status(text);
                }
            }
        }
    }

    fn insert_char(&mut self, byte: u8) -> bool {
        self.ensure_cursor_bounds();
        let line = self.cursor_line;
        let len = self.line_lens[line];
        if len >= DESKTOP_NOTES_LINE_LEN {
            return false;
        }

        if self.cursor_col < len {
            for idx in (self.cursor_col..len).rev() {
                self.lines[line][idx + 1] = self.lines[line][idx];
            }
        }

        self.lines[line][self.cursor_col] = sanitize_editor_byte(byte);
        self.line_lens[line] += 1;
        self.cursor_col += 1;
        self.dirty = true;
        true
    }

    fn insert_newline(&mut self) -> bool {
        self.ensure_cursor_bounds();
        if self.line_count >= DESKTOP_NOTES_MAX_LINES {
            self.set_status("line limit reached");
            return false;
        }

        for idx in (self.cursor_line + 1..=self.line_count).rev() {
            self.lines[idx] = self.lines[idx - 1];
            self.line_lens[idx] = self.line_lens[idx - 1];
        }

        let old_line = self.cursor_line;
        let old_len = self.line_lens[old_line];
        let split = self.cursor_col.min(old_len);
        let tail_len = old_len.saturating_sub(split);
        let mut tail = [0u8; DESKTOP_NOTES_LINE_LEN];
        tail[..tail_len].copy_from_slice(&self.lines[old_line][split..split + tail_len]);

        self.line_lens[old_line] = split;
        self.lines[old_line][split..].fill(0);

        let new_line = old_line + 1;
        self.lines[new_line].fill(0);
        self.lines[new_line][..tail_len].copy_from_slice(&tail[..tail_len]);
        self.line_lens[new_line] = tail_len;

        self.line_count += 1;
        self.cursor_line = new_line;
        self.cursor_col = 0;
        self.dirty = true;
        true
    }

    fn backspace(&mut self) -> bool {
        self.ensure_cursor_bounds();
        if self.cursor_col > 0 {
            let line = self.cursor_line;
            let len = self.line_lens[line];
            let remove = self.cursor_col - 1;
            for idx in remove..len.saturating_sub(1) {
                self.lines[line][idx] = self.lines[line][idx + 1];
            }
            self.line_lens[line] = len.saturating_sub(1);
            self.cursor_col -= 1;
            self.dirty = true;
            return true;
        }

        if self.cursor_line == 0 {
            return false;
        }

        let current = self.cursor_line;
        let previous = current - 1;
        let prev_len = self.line_lens[previous];
        let cur_len = self.line_lens[current];
        if prev_len + cur_len > DESKTOP_NOTES_LINE_LEN {
            self.set_status("line too long to merge");
            return false;
        }

        let mut tail = [0u8; DESKTOP_NOTES_LINE_LEN];
        tail[..cur_len].copy_from_slice(&self.lines[current][..cur_len]);
        self.lines[previous][prev_len..prev_len + cur_len].copy_from_slice(&tail[..cur_len]);
        self.line_lens[previous] = prev_len + cur_len;

        for idx in current..self.line_count.saturating_sub(1) {
            self.lines[idx] = self.lines[idx + 1];
            self.line_lens[idx] = self.line_lens[idx + 1];
        }
        self.line_count = self.line_count.saturating_sub(1).max(1);
        self.cursor_line = previous;
        self.cursor_col = prev_len;
        self.dirty = true;
        true
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key {
            KeyEvent::Char('\n') => self.insert_newline(),
            KeyEvent::Char('\x08') => self.backspace(),
            KeyEvent::Char(ch) if is_printable(ch) => self.insert_char(ch as u8),
            KeyEvent::Left => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                } else if self.cursor_line > 0 {
                    self.cursor_line -= 1;
                    self.cursor_col = self.line_lens[self.cursor_line];
                }
                true
            }
            KeyEvent::Right => {
                let len = self.line_lens[self.cursor_line];
                if self.cursor_col < len {
                    self.cursor_col += 1;
                } else if self.cursor_line + 1 < self.line_count {
                    self.cursor_line += 1;
                    self.cursor_col = 0;
                }
                true
            }
            KeyEvent::Up => {
                if self.cursor_line > 0 {
                    self.cursor_line -= 1;
                    self.cursor_col = self.cursor_col.min(self.line_lens[self.cursor_line]);
                }
                true
            }
            KeyEvent::Down => {
                if self.cursor_line + 1 < self.line_count {
                    self.cursor_line += 1;
                    self.cursor_col = self.cursor_col.min(self.line_lens[self.cursor_line]);
                }
                true
            }
            KeyEvent::PageUp => {
                self.scroll = self.scroll.saturating_sub(8);
                true
            }
            KeyEvent::PageDown => {
                self.scroll = self.scroll.saturating_add(8);
                true
            }
            _ => false,
        }
    }

    fn handle_mouse_down(&mut self, local_x: i32, local_y: i32, width: usize, height: usize) -> bool {
        let layout = desktop_notes_layout(width, height);
        if layout.save_button.contains(local_x, local_y) {
            self.save_to_disk();
            return true;
        }
        if layout.load_button.contains(local_x, local_y) {
            self.load_from_disk();
            return true;
        }
        if layout.clear_button.contains(local_x, local_y) {
            self.reset_document();
            self.set_status("cleared");
            return true;
        }

        if !layout.editor_rect.contains(local_x, local_y) {
            return false;
        }

        let font_h = desktop_font_height().max(8);
        let row = ((local_y.saturating_sub(layout.editor_rect.y)) / font_h).max(0) as usize;
        let col = ((local_x.saturating_sub(layout.editor_rect.x).saturating_sub(4)) / 8).max(0) as usize;
        let line = (self.scroll + row).min(self.line_count.saturating_sub(1));
        self.cursor_line = line;
        self.cursor_col = col.min(self.line_lens[line]);
        true
    }

    fn handle_event(&mut self, event: InputEvent, client_rect: ui::Rect) -> bool {
        match event {
            InputEvent::KeyPress { key } => self.handle_key(key),
            InputEvent::MouseDown {
                button: MouseButton::Left,
                ..
            } => {
                let Some((local_x, local_y)) = desktop_mouse_local(event, client_rect) else {
                    return false;
                };
                self.handle_mouse_down(
                    local_x,
                    local_y,
                    client_rect.width.max(0) as usize,
                    client_rect.height.max(0) as usize,
                )
            }
            _ => false,
        }
    }

    fn draw(
        &mut self,
        pixels: &mut [u32],
        width: usize,
        height: usize,
        app: DesktopAppSpec,
        tick: u32,
        focused: bool,
    ) {
        if width == 0 || height == 0 {
            return;
        }

        multdemo_fill_gradient_vertical(pixels, width, height, app.fill_a, app.fill_b);
        desktop_draw_border(
            pixels,
            width,
            height,
            ui::Rect::new(0, 0, width as i32, height as i32),
            app.stripe,
        );

        let layout = desktop_notes_layout(width, height);
        multdemo_fill_rect(
            pixels,
            width,
            height,
            0,
            0,
            width as i32,
            26,
            app.background,
        );
        desktop_draw_border(pixels, width, height, layout.save_button, 0xB285C2);
        desktop_draw_border(pixels, width, height, layout.load_button, 0xB285C2);
        desktop_draw_border(pixels, width, height, layout.clear_button, 0xB285C2);
        desktop_draw_text(
            pixels,
            width,
            height,
            layout.save_button.x + 11,
            layout.save_button.y + 4,
            "Save",
            0xFFE9FF,
            app.background,
        );
        desktop_draw_text(
            pixels,
            width,
            height,
            layout.load_button.x + 11,
            layout.load_button.y + 4,
            "Load",
            0xFFE9FF,
            app.background,
        );
        desktop_draw_text(
            pixels,
            width,
            height,
            layout.clear_button.x + 8,
            layout.clear_button.y + 4,
            "Clear",
            0xFFE9FF,
            app.background,
        );

        multdemo_fill_rect(
            pixels,
            width,
            height,
            layout.editor_rect.x,
            layout.editor_rect.y,
            layout.editor_rect.width,
            layout.editor_rect.height,
            0x2B1D2E,
        );
        desktop_draw_border(pixels, width, height, layout.editor_rect, 0xA16CB4);

        let font_h = desktop_font_height().max(8);
        let visible_rows = (layout.editor_rect.height / font_h).max(1) as usize;
        if self.cursor_line < self.scroll {
            self.scroll = self.cursor_line;
        }
        if self.cursor_line >= self.scroll.saturating_add(visible_rows) {
            self.scroll = self.cursor_line.saturating_add(1).saturating_sub(visible_rows);
        }
        if self.scroll >= self.line_count {
            self.scroll = self.line_count.saturating_sub(1);
        }

        for row in 0..visible_rows {
            let line_index = self.scroll.saturating_add(row);
            if line_index >= self.line_count {
                break;
            }
            let y = layout.editor_rect.y + row as i32 * font_h;
            let bg = if line_index % 2 == 0 { 0x2B1D2E } else { 0x311F35 };
            multdemo_fill_rect(
                pixels,
                width,
                height,
                layout.editor_rect.x + 1,
                y,
                layout.editor_rect.width.saturating_sub(2),
                font_h,
                bg,
            );
            desktop_draw_text_bytes(
                pixels,
                width,
                height,
                layout.editor_rect.x + 4,
                y,
                &self.lines[line_index][..self.line_lens[line_index]],
                0xF8E7FF,
                bg,
            );
        }

        let show_cursor = focused && ((tick / 20) & 1) == 0;
        if show_cursor && self.cursor_line >= self.scroll && self.cursor_line < self.scroll + visible_rows {
            let cursor_x = layout
                .editor_rect
                .x
                .saturating_add(4)
                .saturating_add(self.cursor_col as i32 * 8);
            let cursor_y = layout
                .editor_rect
                .y
                .saturating_add((self.cursor_line - self.scroll) as i32 * font_h);
            multdemo_fill_rect(
                pixels,
                width,
                height,
                cursor_x,
                cursor_y,
                1,
                font_h,
                0xFFE3FF,
            );
        }

        let dirty_suffix = if self.dirty { " *" } else { "" };
        let mut status = [0u8; DESKTOP_STATUS_TEXT_MAX];
        let mut len = 0usize;
        windemo_push_bytes(&mut status, &mut len, &self.status[..self.status_len]);
        windemo_push_bytes(&mut status, &mut len, dirty_suffix.as_bytes());
        multdemo_fill_rect(
            pixels,
            width,
            height,
            layout.status_rect.x,
            layout.status_rect.y,
            layout.status_rect.width,
            layout.status_rect.height,
            0x2A1B2D,
        );
        desktop_draw_border(pixels, width, height, layout.status_rect, 0xA16CB4);
        if let Ok(text) = str::from_utf8(&status[..len]) {
            let max_chars = ((layout.status_rect.width.saturating_sub(12)) / 8).max(0) as usize;
            let clipped = desktop_trim_text(text, max_chars);
            desktop_draw_text(
                pixels,
                width,
                height,
                layout.status_rect.x + 6,
                layout.status_rect.y + 4,
                clipped,
                0xE2C9EA,
                0x2A1B2D,
            );
        }
    }
}

struct DesktopPaintState {
    canvas: [u32; DESKTOP_PAINT_CANVAS_PIXELS],
    color_index: usize,
    drawing: bool,
    drawing_window: Option<ui::WindowId>,
    last_point: Option<(usize, usize)>,
}

impl DesktopPaintState {
    fn new() -> Self {
        Self {
            canvas: [0xF2F2F2; DESKTOP_PAINT_CANVAS_PIXELS],
            color_index: 0,
            drawing: false,
            drawing_window: None,
            last_point: None,
        }
    }

    fn on_window_closed(&mut self, id: ui::WindowId) {
        if self.drawing_window == Some(id) {
            self.end_stroke();
        }
    }

    fn end_stroke(&mut self) -> bool {
        if !self.drawing && self.last_point.is_none() && self.drawing_window.is_none() {
            return false;
        }
        self.drawing = false;
        self.drawing_window = None;
        self.last_point = None;
        true
    }

    fn clear_canvas(&mut self) {
        self.canvas.fill(0xF2F2F2);
    }

    fn paint_color(&self) -> u32 {
        DESKTOP_PAINT_PALETTE[self
            .color_index
            .min(DESKTOP_PAINT_PALETTE.len().saturating_sub(1))]
    }

    fn set_canvas_pixel(&mut self, x: i32, y: i32, color: u32) {
        if x < 0 || y < 0 || x >= DESKTOP_PAINT_CANVAS_W as i32 || y >= DESKTOP_PAINT_CANVAS_H as i32 {
            return;
        }
        let index = y as usize * DESKTOP_PAINT_CANVAS_W + x as usize;
        if index < self.canvas.len() {
            self.canvas[index] = color;
        }
    }

    fn draw_canvas_line(&mut self, start: (usize, usize), end: (usize, usize), color: u32) {
        let mut x0 = start.0 as i32;
        let mut y0 = start.1 as i32;
        let x1 = end.0 as i32;
        let y1 = end.1 as i32;

        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;

        loop {
            self.set_canvas_pixel(x0, y0, color);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = err.saturating_mul(2);
            if e2 >= dy {
                err = err.saturating_add(dy);
                x0 = x0.saturating_add(sx);
            }
            if e2 <= dx {
                err = err.saturating_add(dx);
                y0 = y0.saturating_add(sy);
            }
        }
    }

    fn layout_canvas_coord(layout: DesktopPaintLayout, local_x: i32, local_y: i32) -> Option<(usize, usize)> {
        if !layout.canvas_rect.contains(local_x, local_y) {
            return None;
        }

        let scale = layout.scale.max(1);
        let cx = (local_x.saturating_sub(layout.canvas_rect.x) / scale) as usize;
        let cy = (local_y.saturating_sub(layout.canvas_rect.y) / scale) as usize;
        if cx >= DESKTOP_PAINT_CANVAS_W || cy >= DESKTOP_PAINT_CANVAS_H {
            return None;
        }
        Some((cx, cy))
    }

    fn handle_mouse_down(
        &mut self,
        window_id: ui::WindowId,
        local_x: i32,
        local_y: i32,
        width: usize,
        height: usize,
    ) -> bool {
        let layout = desktop_paint_layout(width, height);
        if layout.clear_button.contains(local_x, local_y) {
            self.clear_canvas();
            self.end_stroke();
            return true;
        }

        for (index, swatch) in layout.palette.iter().enumerate() {
            if swatch.contains(local_x, local_y) {
                self.color_index = index;
                return true;
            }
        }

        let Some(point) = Self::layout_canvas_coord(layout, local_x, local_y) else {
            return false;
        };

        let color = self.paint_color();
        self.set_canvas_pixel(point.0 as i32, point.1 as i32, color);
        self.drawing = true;
        self.drawing_window = Some(window_id);
        self.last_point = Some(point);
        true
    }

    fn handle_mouse_move(
        &mut self,
        window_id: ui::WindowId,
        local_x: i32,
        local_y: i32,
        width: usize,
        height: usize,
    ) -> bool {
        if !self.drawing || self.drawing_window != Some(window_id) {
            return false;
        }

        let layout = desktop_paint_layout(width, height);
        let Some(point) = Self::layout_canvas_coord(layout, local_x, local_y) else {
            return false;
        };

        let color = self.paint_color();
        if let Some(previous) = self.last_point {
            self.draw_canvas_line(previous, point, color);
        } else {
            self.set_canvas_pixel(point.0 as i32, point.1 as i32, color);
        }
        self.last_point = Some(point);
        true
    }

    fn handle_event(&mut self, event: InputEvent, window_id: ui::WindowId, client_rect: ui::Rect) -> bool {
        match event {
            InputEvent::KeyPress {
                key: KeyEvent::Char('c'),
            }
            | InputEvent::KeyPress {
                key: KeyEvent::Char('C'),
            } => {
                self.clear_canvas();
                true
            }
            InputEvent::MouseDown {
                button: MouseButton::Left,
                ..
            } => {
                let Some((local_x, local_y)) = desktop_mouse_local(event, client_rect) else {
                    return false;
                };
                self.handle_mouse_down(
                    window_id,
                    local_x,
                    local_y,
                    client_rect.width.max(0) as usize,
                    client_rect.height.max(0) as usize,
                )
            }
            InputEvent::MouseMove { .. } => {
                let Some((local_x, local_y)) = desktop_mouse_local(event, client_rect) else {
                    return false;
                };
                self.handle_mouse_move(
                    window_id,
                    local_x,
                    local_y,
                    client_rect.width.max(0) as usize,
                    client_rect.height.max(0) as usize,
                )
            }
            InputEvent::MouseUp {
                button: MouseButton::Left,
                ..
            } => self.end_stroke(),
            _ => false,
        }
    }

    fn draw(
        &mut self,
        pixels: &mut [u32],
        width: usize,
        height: usize,
        app: DesktopAppSpec,
        _tick: u32,
        _focused: bool,
    ) {
        if width == 0 || height == 0 {
            return;
        }

        multdemo_fill_gradient_vertical(pixels, width, height, app.fill_a, app.fill_b);
        desktop_draw_border(
            pixels,
            width,
            height,
            ui::Rect::new(0, 0, width as i32, height as i32),
            app.stripe,
        );

        let layout = desktop_paint_layout(width, height);
        multdemo_fill_rect(
            pixels,
            width,
            height,
            0,
            0,
            width as i32,
            26,
            app.background,
        );

        for (index, swatch) in layout.palette.iter().enumerate() {
            multdemo_fill_rect(
                pixels,
                width,
                height,
                swatch.x,
                swatch.y,
                swatch.width,
                swatch.height,
                DESKTOP_PAINT_PALETTE[index],
            );
            desktop_draw_border(
                pixels,
                width,
                height,
                *swatch,
                if index == self.color_index {
                    0xFFFFFF
                } else {
                    0x1E1E1E
                },
            );
        }

        multdemo_fill_rect(
            pixels,
            width,
            height,
            layout.clear_button.x,
            layout.clear_button.y,
            layout.clear_button.width,
            layout.clear_button.height,
            0x213048,
        );
        desktop_draw_border(pixels, width, height, layout.clear_button, 0x8AB4FF);
        desktop_draw_text(
            pixels,
            width,
            height,
            layout.clear_button.x + 8,
            layout.clear_button.y + 4,
            "Clear",
            0xEBF3FF,
            0x213048,
        );

        multdemo_fill_rect(
            pixels,
            width,
            height,
            layout.canvas_rect.x,
            layout.canvas_rect.y,
            layout.canvas_rect.width,
            layout.canvas_rect.height,
            0xDCDCDC,
        );
        desktop_draw_border(pixels, width, height, layout.canvas_rect, 0x4A618A);

        let scale = layout.scale.max(1) as usize;
        for cy in 0..DESKTOP_PAINT_CANVAS_H {
            for sy in 0..scale {
                let py = layout.canvas_rect.y + (cy * scale + sy) as i32;
                if py < 0 || py >= height as i32 {
                    continue;
                }
                let row_offset = py as usize * width;
                for cx in 0..DESKTOP_PAINT_CANVAS_W {
                    let color = self.canvas[cy * DESKTOP_PAINT_CANVAS_W + cx];
                    let px0 = layout.canvas_rect.x + (cx * scale) as i32;
                    for sx in 0..scale {
                        let px = px0 + sx as i32;
                        if px < 0 || px >= width as i32 {
                            continue;
                        }
                        pixels[row_offset + px as usize] = color;
                    }
                }
            }
        }
    }
}

fn desktop_paint_running_windows(
    manager: &mut ui::WindowManager,
    running_windows: &[Option<ui::WindowId>; DESKTOP_APP_COUNT],
    apps: &mut DesktopApps,
    tick: u32,
) {
    let focused = manager.focused_window();
    for (app_index, window_id) in running_windows.iter().enumerate() {
        let Some(id) = *window_id else {
            continue;
        };
        desktop_paint_window(manager, id, app_index, apps, tick, focused == Some(id));
    }
}

fn desktop_paint_window(
    manager: &mut ui::WindowManager,
    id: ui::WindowId,
    app_index: usize,
    apps: &mut DesktopApps,
    tick: u32,
    focused: bool,
) {
    let app = DESKTOP_APP_REGISTRY[app_index];
    match app_index {
        DESKTOP_APP_TERMINAL => {
            let _ = manager.with_window_buffer_mut(id, |pixels, width, height| {
                apps.terminal
                    .draw(pixels, width, height, app, tick, focused);
            });
        }
        DESKTOP_APP_FILES => {
            let _ = manager.with_window_buffer_mut(id, |pixels, width, height| {
                apps.files.draw(pixels, width, height, app, tick, focused);
            });
        }
        DESKTOP_APP_MONITOR => {
            let _ = manager.with_window_buffer_mut(id, |pixels, width, height| {
                apps.monitor
                    .draw(pixels, width, height, app, tick, focused);
            });
        }
        DESKTOP_APP_NOTES => {
            let _ = manager.with_window_buffer_mut(id, |pixels, width, height| {
                apps.notes.draw(pixels, width, height, app, tick, focused);
            });
        }
        DESKTOP_APP_PAINT => {
            let _ = manager.with_window_buffer_mut(id, |pixels, width, height| {
                apps.paint.draw(pixels, width, height, app, tick, focused);
            });
        }
        _ => {}
    }
}

fn desktop_draw_scene(
    manager: &ui::WindowManager,
    layout: &DesktopLayout,
    launcher_open: bool,
    tick: u32,
) {
    vga::begin_draw_batch();
    desktop_draw_background(layout.desktop_bounds, tick);
    manager.compose_windows();
    desktop_draw_taskbar(layout, launcher_open);
    if launcher_open {
        desktop_draw_launcher(layout);
    }
    vga::end_draw_batch();
}

fn desktop_draw_background(bounds: ui::Rect, tick: u32) {
    if bounds.width <= 0 || bounds.height <= 0 {
        return;
    }

    let h = bounds.height.max(1);
    for row in 0..h {
        let t = ((row as u32).saturating_mul(255) / (h as u32)) as u8;
        let color = desktop_blend_color(0x102746, 0x070E1A, t);
        let _ = vga::draw_horizontal_line(bounds.x, bounds.y + row, bounds.width, color);
    }

    let diagonal_step = 96;
    let drift = ((tick >> 1) as i32).rem_euclid(diagonal_step);
    let mut diag_x = bounds
        .x
        .saturating_sub(bounds.height)
        .saturating_add(drift);
    let diag_limit = bounds.x.saturating_add(bounds.width);
    while diag_x < diag_limit {
        let _ = vga::draw_line(
            diag_x,
            bounds.y.saturating_add(bounds.height).saturating_sub(1),
            diag_x.saturating_add(bounds.height),
            bounds.y,
            0x12304E,
        );
        diag_x = diag_x.saturating_add(diagonal_step);
    }

    let mut y = bounds.y.saturating_add(26);
    let y_limit = bounds.y.saturating_add(bounds.height);
    while y < y_limit {
        let _ = vga::draw_horizontal_line(bounds.x, y, bounds.width, 0x12385A);
        y = y.saturating_add(34);
    }

    let mut x = bounds.x.saturating_add(24);
    let x_limit = bounds.x.saturating_add(bounds.width);
    while x < x_limit {
        let _ = vga::draw_vertical_line(x, bounds.y, bounds.height, 0x0F2640);
        x = x.saturating_add(66);
    }

    let sway_x = 0;
    let sway_y = 0;

    let orb_a_x = bounds
        .x
        .saturating_add((bounds.width * 3) / 4)
        .saturating_add(sway_x);
    let orb_a_y = bounds.y.saturating_add(bounds.height / 3).saturating_add(sway_y);
    let _ = vga::draw_circle(orb_a_x, orb_a_y, 48, 0x35638E);
    let _ = vga::draw_circle(orb_a_x, orb_a_y, 82, 0x274B70);
    let _ = vga::draw_circle(orb_a_x, orb_a_y, 116, 0x1A3658);

    let orb_b_x = bounds
        .x
        .saturating_add(bounds.width / 4)
        .saturating_sub(sway_x / 2);
    let orb_b_y = bounds
        .y
        .saturating_add(bounds.height / 4)
        .saturating_sub(sway_y / 2);
    let _ = vga::draw_circle(orb_b_x, orb_b_y, 34, 0x3F6F9A);
    let _ = vga::draw_circle(orb_b_x, orb_b_y, 58, 0x234766);

}

fn desktop_blend_color(a: u32, b: u32, t: u8) -> u32 {
    let t = t as u32;
    let inv = 255u32.saturating_sub(t);

    let ar = (a >> 16) & 0xFF;
    let ag = (a >> 8) & 0xFF;
    let ab = a & 0xFF;
    let br = (b >> 16) & 0xFF;
    let bg = (b >> 8) & 0xFF;
    let bb = b & 0xFF;

    let r = (ar.saturating_mul(inv) + br.saturating_mul(t)) / 255;
    let g = (ag.saturating_mul(inv) + bg.saturating_mul(t)) / 255;
    let b = (ab.saturating_mul(inv) + bb.saturating_mul(t)) / 255;
    (r << 16) | (g << 8) | b
}

fn desktop_draw_taskbar(layout: &DesktopLayout, launcher_open: bool) {
    let panel = layout.panel_rect;
    let _ = vga::draw_filled_rect(panel.x, panel.y, panel.width, panel.height, 0x0A101C);
    let _ = vga::draw_horizontal_line(panel.x, panel.y, panel.width, 0x3B587D);
    let _ = vga::draw_horizontal_line(
        panel.x,
        panel.y.saturating_add(panel.height).saturating_sub(1),
        panel.width,
        0x19273B,
    );

    let start_bg = if launcher_open { 0x2B5684 } else { 0x17314F };
    let start_border = if launcher_open { 0x79B5FF } else { 0x345579 };
    let _ = vga::draw_filled_rect(
        layout.start_button.x,
        layout.start_button.y,
        layout.start_button.width,
        layout.start_button.height,
        start_bg,
    );
    let _ = vga::draw_horizontal_line(
        layout.start_button.x,
        layout.start_button.y,
        layout.start_button.width,
        start_border,
    );
    let _ = vga::draw_horizontal_line(
        layout.start_button.x,
        layout.start_button
            .y
            .saturating_add(layout.start_button.height)
            .saturating_sub(1),
        layout.start_button.width,
        start_border,
    );
    let _ = vga::draw_vertical_line(
        layout.start_button.x,
        layout.start_button.y,
        layout.start_button.height,
        start_border,
    );
    let _ = vga::draw_vertical_line(
        layout
            .start_button
            .x
            .saturating_add(layout.start_button.width)
            .saturating_sub(1),
        layout.start_button.y,
        layout.start_button.height,
        start_border,
    );
    let _ = vga::draw_text(
        layout.start_button.x + 14,
        layout.start_button.y + 4,
        "Start",
        0xEAF4FF,
        start_bg,
    );

    for button in layout.task_buttons[..layout.task_button_count].iter() {
        let bg = if button.focused {
            0x234C74
        } else if button.minimized {
            0x1A2333
        } else {
            0x1E2C42
        };
        let border = if button.focused { 0x82C8FF } else { 0x415874 };
        let fg = if button.minimized { 0x9EB2CD } else { 0xE1EEFF };

        let _ = vga::draw_filled_rect(button.rect.x, button.rect.y, button.rect.width, button.rect.height, bg);
        let _ = vga::draw_horizontal_line(button.rect.x, button.rect.y, button.rect.width, border);
        let _ = vga::draw_horizontal_line(
            button.rect.x,
            button.rect.y.saturating_add(button.rect.height).saturating_sub(1),
            button.rect.width,
            border,
        );
        let _ = vga::draw_vertical_line(button.rect.x, button.rect.y, button.rect.height, border);
        let _ = vga::draw_vertical_line(
            button.rect.x.saturating_add(button.rect.width).saturating_sub(1),
            button.rect.y,
            button.rect.height,
            border,
        );

        let marker = if button.focused { 0x5ED6FF } else { 0x3A4E67 };
        let _ = vga::draw_filled_rect(
            button.rect.x + 2,
            button.rect.y + 2,
            3,
            button.rect.height.saturating_sub(4),
            marker,
        );

        let max_chars = ((button.rect.width.saturating_sub(14)) / 8).max(0) as usize;
        let label = desktop_trim_text(button.title, max_chars);
        let _ = vga::draw_text(button.rect.x + 8, button.rect.y + 4, label, fg, bg);
    }

    let clock_bg = 0x14283F;
    let _ = vga::draw_filled_rect(
        layout.clock_rect.x,
        layout.clock_rect.y,
        layout.clock_rect.width,
        layout.clock_rect.height,
        clock_bg,
    );
    let _ = vga::draw_horizontal_line(
        layout.clock_rect.x,
        layout.clock_rect.y,
        layout.clock_rect.width,
        0x45688D,
    );
    let _ = vga::draw_horizontal_line(
        layout.clock_rect.x,
        layout
            .clock_rect
            .y
            .saturating_add(layout.clock_rect.height)
            .saturating_sub(1),
        layout.clock_rect.width,
        0x45688D,
    );
    let _ = vga::draw_vertical_line(
        layout.clock_rect.x,
        layout.clock_rect.y,
        layout.clock_rect.height,
        0x45688D,
    );
    let _ = vga::draw_vertical_line(
        layout
            .clock_rect
            .x
            .saturating_add(layout.clock_rect.width)
            .saturating_sub(1),
        layout.clock_rect.y,
        layout.clock_rect.height,
        0x45688D,
    );

    let mut clock_buf = [0u8; 16];
    let clock_len = desktop_format_clock_text(&mut clock_buf);
    if let Ok(clock_text) = core::str::from_utf8(&clock_buf[..clock_len]) {
        let _ = vga::draw_text(
            layout.clock_rect.x + 10,
            layout.clock_rect.y + 4,
            clock_text,
            0xE5F0FF,
            clock_bg,
        );
    }
}

fn desktop_draw_launcher(layout: &DesktopLayout) {
    let panel = layout.launcher_rect;
    if panel.width <= 0 || panel.height <= 0 {
        return;
    }

    let _ = vga::draw_filled_rect(panel.x, panel.y, panel.width, panel.height, 0x101B2D);
    let _ = vga::draw_horizontal_line(panel.x, panel.y, panel.width, 0x5F83AA);
    let _ = vga::draw_horizontal_line(
        panel.x,
        panel.y.saturating_add(panel.height).saturating_sub(1),
        panel.width,
        0x2F4764,
    );
    let _ = vga::draw_vertical_line(panel.x, panel.y, panel.height, 0x5F83AA);
    let _ = vga::draw_vertical_line(
        panel.x.saturating_add(panel.width).saturating_sub(1),
        panel.y,
        panel.height,
        0x5F83AA,
    );
    let _ = vga::draw_text(
        panel.x + 10,
        panel.y + 6,
        "Applications",
        0xE8F1FF,
        0x101B2D,
    );

    for item in layout.launcher_items[..layout.launcher_item_count].iter() {
        let app = DESKTOP_APP_REGISTRY[item.app_index];
        let bg = if item.running { 0x1E3653 } else { 0x16273E };
        let border = if item.running { 0x70A8E2 } else { 0x3C5775 };
        let status = if item.running { "open" } else { "launch" };
        let status_x = if item.running {
            item.rect.x.saturating_add(item.rect.width).saturating_sub(42)
        } else {
            item.rect.x.saturating_add(item.rect.width).saturating_sub(58)
        };

        let _ = vga::draw_filled_rect(item.rect.x, item.rect.y, item.rect.width, item.rect.height, bg);
        let _ = vga::draw_horizontal_line(item.rect.x, item.rect.y, item.rect.width, border);
        let _ = vga::draw_horizontal_line(
            item.rect.x,
            item.rect.y.saturating_add(item.rect.height).saturating_sub(1),
            item.rect.width,
            border,
        );
        let _ = vga::draw_text(item.rect.x + 8, item.rect.y + 5, app.name, 0xE4EEFF, bg);
        let _ = vga::draw_text(status_x, item.rect.y + 5, status, 0xA9C9EE, bg);
        let _ = vga::draw_text(item.rect.x + 112, item.rect.y + 5, app.description, 0xB4C7DE, bg);
        let _ = vga::draw_text(item.rect.x + 84, item.rect.y + 5, app.key, 0x86A4C8, bg);
    }
}

fn desktop_trim_text(text: &str, max_chars: usize) -> &str {
    if max_chars == 0 {
        return "";
    }

    let mut count = 0usize;
    let mut end = text.len();
    for (index, ch) in text.char_indices() {
        if count == max_chars {
            end = index;
            break;
        }
        count += 1;
        end = index + ch.len_utf8();
    }

    if count < max_chars {
        text
    } else {
        &text[..end]
    }
}

fn desktop_clock_now() -> DesktopClock {
    if let Some(now) = rtc::now() {
        return DesktopClock {
            hour: now.hour,
            minute: now.minute,
            second: now.second,
        };
    }

    let seconds = timer::uptime().seconds as u32;
    let hour = ((seconds / 3600) % 24) as u8;
    let minute = ((seconds / 60) % 60) as u8;
    let second = (seconds % 60) as u8;
    DesktopClock {
        hour,
        minute,
        second,
    }
}

fn desktop_clock_second_key() -> u32 {
    let clock = desktop_clock_now();
    (clock.hour as u32)
        .saturating_mul(3600)
        .saturating_add((clock.minute as u32).saturating_mul(60))
        .saturating_add(clock.second as u32)
}

fn desktop_format_clock_text(buffer: &mut [u8; 16]) -> usize {
    let clock = desktop_clock_now();
    let mut len = 0usize;
    desktop_push_two_digits(buffer, &mut len, clock.hour);
    windemo_push_bytes(buffer, &mut len, b":");
    desktop_push_two_digits(buffer, &mut len, clock.minute);
    windemo_push_bytes(buffer, &mut len, b":");
    desktop_push_two_digits(buffer, &mut len, clock.second);
    len
}

fn desktop_push_two_digits<const N: usize>(buffer: &mut [u8; N], len: &mut usize, value: u8) {
    if *len + 2 > N {
        return;
    }
    buffer[*len] = b'0' + ((value / 10) % 10);
    buffer[*len + 1] = b'0' + (value % 10);
    *len += 2;
}

#[derive(Clone, Copy)]
struct MultDemoVisualWorkerArg {
    shared: *const MultDemoVisualShared,
    slot: usize,
    phase_step: u32,
    sleep_ticks: u32,
}

#[derive(Clone, Copy)]
struct MultDemoVisualSnapshot {
    phases: [u32; MULTDEMO_WORKERS],
    updates: [u32; MULTDEMO_WORKERS],
    switches: u32,
    shared_ticks: u32,
}

struct MultDemoVisualState {
    phases: [u32; MULTDEMO_WORKERS],
    updates: [u32; MULTDEMO_WORKERS],
    switches: u32,
    shared_ticks: u32,
    last_task_id: Option<u32>,
}

impl MultDemoVisualState {
    const fn new() -> Self {
        Self {
            phases: [0; MULTDEMO_WORKERS],
            updates: [0; MULTDEMO_WORKERS],
            switches: 0,
            shared_ticks: 0,
            last_task_id: None,
        }
    }
}

struct MultDemoVisualShared {
    state: sync::Mutex<MultDemoVisualState>,
    gate: sync::Semaphore,
    stop: AtomicBool,
    completed: AtomicU32,
    active_workers: AtomicU32,
    max_parallel: AtomicU32,
}

impl MultDemoVisualShared {
    const fn new() -> Self {
        Self {
            state: sync::Mutex::new(MultDemoVisualState::new()),
            gate: sync::Semaphore::new(2),
            stop: AtomicBool::new(false),
            completed: AtomicU32::new(0),
            active_workers: AtomicU32::new(0),
            max_parallel: AtomicU32::new(0),
        }
    }

    fn snapshot(&self) -> MultDemoVisualSnapshot {
        let state = self.state.lock();
        MultDemoVisualSnapshot {
            phases: state.phases,
            updates: state.updates,
            switches: state.switches,
            shared_ticks: state.shared_ticks,
        }
    }

    fn update_max_parallel(&self, candidate: u32) {
        let mut observed = self.max_parallel.load(Ordering::Acquire);
        while candidate > observed {
            match self.max_parallel.compare_exchange_weak(
                observed,
                candidate,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(current) => observed = current,
            }
        }
    }
}

fn multdemo_visual_worker_entry(raw_arg: *mut u8) {
    let Some(arg_ref) = (unsafe { (raw_arg as *const MultDemoVisualWorkerArg).as_ref() }) else {
        return;
    };
    let arg = *arg_ref;
    let Some(shared) = (unsafe { arg.shared.as_ref() }) else {
        return;
    };

    loop {
        if shared.stop.load(Ordering::Acquire) {
            break;
        }

        shared.gate.acquire();
        if shared.stop.load(Ordering::Acquire) {
            shared.gate.release();
            break;
        }

        let active = shared
            .active_workers
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        shared.update_max_parallel(active);

        {
            let mut state = shared.state.lock();
            if arg.slot < state.phases.len() {
                state.phases[arg.slot] = state.phases[arg.slot].wrapping_add(arg.phase_step);
                state.updates[arg.slot] = state.updates[arg.slot].wrapping_add(1);
            }
            state.shared_ticks = state.shared_ticks.wrapping_add(1);

            let current_task = task::current_task_id().unwrap_or(0);
            if state.last_task_id != Some(current_task) {
                state.switches = state.switches.saturating_add(1);
                state.last_task_id = Some(current_task);
            }
        }

        shared.active_workers.fetch_sub(1, Ordering::AcqRel);
        shared.gate.release();

        if (arg.phase_step & 1) == 0 {
            task::yield_now();
        }
        task::sleep_ticks(arg.sleep_ticks.max(1));
    }

    shared.completed.fetch_add(1, Ordering::AcqRel);
}

fn handle_multdemo_graphics_command() {
    let Some((fb_width, fb_height)) = vga::framebuffer_resolution() else {
        shell_println!("multdemo requires VBE/framebuffer mode");
        shell_println!("tip: run `multdemo bench` for the text benchmark");
        return;
    };

    let width = fb_width.min(i32::MAX as usize) as i32;
    let height = fb_height.min(i32::MAX as usize) as i32;
    if width <= 0 || height <= 0 {
        shell_println!("multdemo: invalid framebuffer size");
        return;
    }
    if width < 720 || height < 420 {
        shell_println!("multdemo: framebuffer too small (need at least 720x420)");
        return;
    }

    shell_println!("multdemo: graphical multitasking demo");
    shell_println!("multdemo: q exits, d toggles debug, drag/resize windows");
    shell_println!("multdemo: workers update clock/gears/wave in parallel");

    for _ in 0..512 {
        if input::pop_event().is_none() {
            break;
        }
    }

    let shared = MultDemoVisualShared::new();
    let mut worker_args = [MultDemoVisualWorkerArg {
        shared: core::ptr::null(),
        slot: 0,
        phase_step: 1,
        sleep_ticks: 1,
    }; MULTDEMO_WORKERS];
    let mut worker_ids = [0u32; MULTDEMO_WORKERS];
    let mut spawned = 0u32;

    const STEPS: [u32; MULTDEMO_WORKERS] = [5, 7, 11];
    const SLEEPS: [u32; MULTDEMO_WORKERS] = [2, 3, 1];
    for slot in 0..MULTDEMO_WORKERS {
        worker_args[slot] = MultDemoVisualWorkerArg {
            shared: core::ptr::from_ref(&shared),
            slot,
            phase_step: STEPS[slot],
            sleep_ticks: SLEEPS[slot],
        };

        let arg_ptr = (&mut worker_args[slot] as *mut MultDemoVisualWorkerArg).cast::<u8>();
        match task::spawn_kernel(multdemo_visual_worker_entry, arg_ptr) {
            Ok(id) => {
                worker_ids[slot] = id;
                spawned = spawned.saturating_add(1);
                shell_println!("multdemo: visual worker {} task_id={}", slot, id);
            }
            Err(reason) => {
                shell_println!(
                    "multdemo: failed to spawn visual worker {}: {}",
                    slot,
                    reason
                );
                break;
            }
        }
    }

    if spawned == 0 {
        shell_println!("multdemo: no visual workers spawned");
        return;
    }

    let desktop = ui::Rect::new(0, 0, width, height);
    let mut manager = ui::WindowManager::new(0x071320);

    let clock_window = manager.add_window(ui::WindowSpec {
        title: "Clock",
        rect: ui::Rect::new(40, 46, 284, 226),
        min_width: 190,
        min_height: 146,
        background: 0x0A1728,
        accent: 0x3BA7E4,
    });
    let gears_window = manager.add_window(ui::WindowSpec {
        title: "Gears",
        rect: ui::Rect::new(300, 78, 390, 244),
        min_width: 220,
        min_height: 156,
        background: 0x1A1410,
        accent: 0xEE9A3A,
    });
    let scope_window = manager.add_window(ui::WindowSpec {
        title: "Wave",
        rect: ui::Rect::new(188, 248, 366, 210),
        min_width: 220,
        min_height: 140,
        background: 0x0E1813,
        accent: 0x4EE17C,
    });

    let Ok(clock_id) = clock_window else {
        shell_println!("multdemo: failed to create Clock window");
        multdemo_stop_visual_workers(&shared, spawned);
        return;
    };
    let Ok(gears_id) = gears_window else {
        shell_println!("multdemo: failed to create Gears window");
        multdemo_stop_visual_workers(&shared, spawned);
        return;
    };
    let Ok(scope_id) = scope_window else {
        shell_println!("multdemo: failed to create Wave window");
        multdemo_stop_visual_workers(&shared, spawned);
        return;
    };

    let mut clock_open = Some(clock_id);
    let mut gears_open = Some(gears_id);
    let mut scope_open = Some(scope_id);
    let mut debug_enabled = false;
    let mut last_frame_tick = timer::ticks().wrapping_sub(MULTDEMO_FRAME_TICKS);

    let first = shared.snapshot();
    paint_multdemo_windows(
        &mut manager,
        clock_open,
        gears_open,
        scope_open,
        first,
    );
    draw_multdemo_scene(
        &manager,
        desktop,
        width,
        height,
        first,
        shared.max_parallel.load(Ordering::Acquire),
        debug_enabled,
    );

    'demo: loop {
        let mut redraw = false;
        let mut processed = 0usize;

        for _ in 0..128 {
            let Some(event) = input::pop_event() else {
                break;
            };
            processed += 1;

            match event {
                InputEvent::KeyPress {
                    key: KeyEvent::Char('q'),
                }
                | InputEvent::KeyPress {
                    key: KeyEvent::Char('Q'),
                } => break 'demo,
                InputEvent::KeyPress {
                    key: KeyEvent::Char('d'),
                }
                | InputEvent::KeyPress {
                    key: KeyEvent::Char('D'),
                } => {
                    debug_enabled = !debug_enabled;
                    redraw = true;
                    continue;
                }
                _ => {}
            }

            let response = manager.handle_event(event, desktop);
            if response.redraw || response.closed.is_some() {
                redraw = true;
            }
            if let Some(closed) = response.closed {
                if clock_open == Some(closed) {
                    clock_open = None;
                }
                if gears_open == Some(closed) {
                    gears_open = None;
                }
                if scope_open == Some(closed) {
                    scope_open = None;
                }
            }
            if debug_enabled && matches!(event, InputEvent::MouseMove { .. }) {
                redraw = true;
            }
        }

        if manager.window_count() == 0 {
            break;
        }

        let now = timer::ticks();
        if now.wrapping_sub(last_frame_tick) >= MULTDEMO_FRAME_TICKS {
            redraw = true;
            last_frame_tick = now;
        }

        if redraw {
            let snapshot = shared.snapshot();
            paint_multdemo_windows(
                &mut manager,
                clock_open,
                gears_open,
                scope_open,
                snapshot,
            );
            draw_multdemo_scene(
                &manager,
                desktop,
                width,
                height,
                snapshot,
                shared.max_parallel.load(Ordering::Acquire),
                debug_enabled,
            );
        }

        if processed == 0 {
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
            }
        }
    }

    multdemo_stop_visual_workers(&shared, spawned);
    let summary = shared.snapshot();
    let max_parallel = shared.max_parallel.load(Ordering::Acquire);
    shell_println!(
        "multdemo: visual stats updates=[{},{},{}] switches={} max_parallel={}",
        summary.updates[0],
        summary.updates[1],
        summary.updates[2],
        summary.switches,
        max_parallel
    );

    let _ = worker_ids;
    vga::clear_screen();
}

fn multdemo_stop_visual_workers(shared: &MultDemoVisualShared, spawned: u32) {
    if spawned == 0 {
        return;
    }

    shared.stop.store(true, Ordering::Release);
    while shared.completed.load(Ordering::Acquire) < spawned {
        task::sleep_ticks(1);
    }
}

fn draw_multdemo_scene(
    manager: &ui::WindowManager,
    desktop: ui::Rect,
    width: i32,
    height: i32,
    snapshot: MultDemoVisualSnapshot,
    max_parallel: u32,
    debug_enabled: bool,
) {
    vga::begin_draw_batch();
    manager.compose(desktop);
    draw_multdemo_overlay(width, height, snapshot, max_parallel);
    if debug_enabled {
        let pointer = mouse::state();
        let snapshot = manager.debug_snapshot(pointer.x, pointer.y);
        draw_windemo_debug_overlay(pointer.x, pointer.y, snapshot);
    }
    vga::end_draw_batch();
}

fn draw_multdemo_overlay(width: i32, height: i32, snapshot: MultDemoVisualSnapshot, max_parallel: u32) {
    let panel_h = 38;
    let panel_w = (width - 20).max(260);
    let panel_x = 10;
    let panel_y = height.saturating_sub(panel_h + 10);
    let bg = 0x0E1B2D;
    let border = 0x4A6C95;
    let fg = 0xDFEAFF;

    let _ = vga::draw_filled_rect(panel_x, panel_y, panel_w, panel_h, bg);
    let _ = vga::draw_horizontal_line(panel_x, panel_y, panel_w, border);
    let _ = vga::draw_horizontal_line(panel_x, panel_y + panel_h - 1, panel_w, border);

    let mut line0 = [0u8; 160];
    let mut line0_len = 0usize;
    windemo_push_bytes(&mut line0, &mut line0_len, b"multdemo windows updates=");
    windemo_push_u32(&mut line0, &mut line0_len, snapshot.updates[0]);
    windemo_push_bytes(&mut line0, &mut line0_len, b"/");
    windemo_push_u32(&mut line0, &mut line0_len, snapshot.updates[1]);
    windemo_push_bytes(&mut line0, &mut line0_len, b"/");
    windemo_push_u32(&mut line0, &mut line0_len, snapshot.updates[2]);
    windemo_push_bytes(&mut line0, &mut line0_len, b"  switches=");
    windemo_push_u32(&mut line0, &mut line0_len, snapshot.switches);
    draw_windemo_debug_line(panel_x + 8, panel_y + 5, &line0[..line0_len], fg, bg);

    let mut line1 = [0u8; 160];
    let mut line1_len = 0usize;
    windemo_push_bytes(&mut line1, &mut line1_len, b"shared=");
    windemo_push_u32(&mut line1, &mut line1_len, snapshot.shared_ticks);
    windemo_push_bytes(&mut line1, &mut line1_len, b" max_parallel=");
    windemo_push_u32(&mut line1, &mut line1_len, max_parallel);
    windemo_push_bytes(&mut line1, &mut line1_len, b"  q:exit d:debug");
    draw_windemo_debug_line(panel_x + 8, panel_y + 21, &line1[..line1_len], fg, bg);
}

fn paint_multdemo_windows(
    manager: &mut ui::WindowManager,
    clock_id: Option<ui::WindowId>,
    gears_id: Option<ui::WindowId>,
    wave_id: Option<ui::WindowId>,
    snapshot: MultDemoVisualSnapshot,
) {
    if let Some(id) = clock_id {
        paint_multdemo_clock_window(manager, id, snapshot);
    }
    if let Some(id) = gears_id {
        paint_multdemo_gears_window(manager, id, snapshot);
    }
    if let Some(id) = wave_id {
        paint_multdemo_wave_window(manager, id, snapshot);
    }
}

fn paint_multdemo_clock_window(
    manager: &mut ui::WindowManager,
    id: ui::WindowId,
    snapshot: MultDemoVisualSnapshot,
) {
    let _ = manager.with_window_buffer_mut(id, |pixels, width, height| {
        if width == 0 || height == 0 {
            return;
        }

        multdemo_fill_gradient_vertical(pixels, width, height, 0x102845, 0x071320);
        let cx = (width as i32) / 2;
        let cy = (height as i32) / 2;
        let radius = ((width.min(height) as i32) / 2).saturating_sub(14).max(10);

        multdemo_draw_filled_circle(pixels, width, height, cx, cy, radius, 0x142D4A);
        multdemo_draw_circle_outline(pixels, width, height, cx, cy, radius, 0x7AC9FF);
        multdemo_draw_circle_outline(
            pixels,
            width,
            height,
            cx,
            cy,
            radius.saturating_sub(1),
            0xCBE9FF,
        );

        for mark in 0..12 {
            let phase = 192u32.wrapping_add((mark as u32).saturating_mul(256 / 12));
            let (dx, dy) = multdemo_direction(phase);
            let inner = radius.saturating_sub(8);
            let outer = radius.saturating_sub(2);
            let x0 = cx.saturating_add(dx.saturating_mul(inner) / 256);
            let y0 = cy.saturating_add(dy.saturating_mul(inner) / 256);
            let x1 = cx.saturating_add(dx.saturating_mul(outer) / 256);
            let y1 = cy.saturating_add(dy.saturating_mul(outer) / 256);
            let color = if mark % 3 == 0 { 0xD6F3FF } else { 0x5EA3CF };
            multdemo_draw_line(pixels, width, height, x0, y0, x1, y1, color);
        }

        let second_phase = snapshot.phases[0].wrapping_add(192);
        let minute_phase = snapshot
            .phases[0]
            .wrapping_add(snapshot.shared_ticks >> 2)
            .wrapping_add(192);
        let hour_phase = snapshot
            .phases[1]
            .wrapping_add(snapshot.shared_ticks >> 4)
            .wrapping_add(192);

        multdemo_draw_clock_hand(
            pixels,
            width,
            height,
            cx,
            cy,
            radius.saturating_sub(12),
            second_phase,
            1,
            0xFF7E66,
        );
        multdemo_draw_clock_hand(
            pixels,
            width,
            height,
            cx,
            cy,
            radius.saturating_sub(20),
            minute_phase,
            2,
            0xFFE39B,
        );
        multdemo_draw_clock_hand(
            pixels,
            width,
            height,
            cx,
            cy,
            radius.saturating_sub(30),
            hour_phase,
            3,
            0xFFFFFF,
        );
        multdemo_draw_filled_circle(pixels, width, height, cx, cy, 3, 0xFFFFFF);
    });
}

fn paint_multdemo_gears_window(
    manager: &mut ui::WindowManager,
    id: ui::WindowId,
    snapshot: MultDemoVisualSnapshot,
) {
    let _ = manager.with_window_buffer_mut(id, |pixels, width, height| {
        if width == 0 || height == 0 {
            return;
        }

        multdemo_fill_gradient_vertical(pixels, width, height, 0x2A1C12, 0x120C08);
        let w = width as i32;
        let h = height as i32;
        let cx_left = w / 3;
        let cx_right = (w * 2) / 3;
        let cy = h / 2;
        let radius = ((width.min(height) as i32) / 4).clamp(14, 72);

        multdemo_draw_gear(
            pixels,
            width,
            height,
            cx_left,
            cy,
            radius,
            snapshot.phases[1],
            0x91582A,
            0xFFD29A,
        );
        multdemo_draw_gear(
            pixels,
            width,
            height,
            cx_right,
            cy,
            radius.saturating_sub(3),
            256u32.wrapping_sub(snapshot.phases[1].wrapping_mul(2)),
            0x4D6A2A,
            0xC7F59C,
        );

        let bridge_y = cy.saturating_add(radius).saturating_add(8);
        multdemo_fill_rect(
            pixels,
            width,
            height,
            cx_left.saturating_sub(8),
            bridge_y,
            cx_right.saturating_sub(cx_left).saturating_add(16),
            6,
            0x3A2A1C,
        );
        multdemo_draw_line(
            pixels,
            width,
            height,
            cx_left,
            bridge_y + 2,
            cx_right,
            bridge_y + 2,
            0x74563E,
        );
    });
}

fn paint_multdemo_wave_window(
    manager: &mut ui::WindowManager,
    id: ui::WindowId,
    snapshot: MultDemoVisualSnapshot,
) {
    let _ = manager.with_window_buffer_mut(id, |pixels, width, height| {
        if width == 0 || height == 0 {
            return;
        }

        multdemo_fill_gradient_vertical(pixels, width, height, 0x102419, 0x07120D);
        let w = width as i32;
        let h = height as i32;
        let mid = h / 2;

        for y in (8..h.saturating_sub(8)).step_by(16) {
            multdemo_draw_line(pixels, width, height, 6, y, w.saturating_sub(7), y, 0x1B3D2A);
        }
        for x in (8..w.saturating_sub(8)).step_by(20) {
            multdemo_draw_line(pixels, width, height, x, 6, x, h.saturating_sub(7), 0x123322);
        }

        let amp = (h / 3).max(8);
        let mut prev_x = 6;
        let mut prev_y = mid;
        for x in 6..w.saturating_sub(6) {
            let phase = snapshot
                .phases[2]
                .wrapping_add((x as u32).wrapping_mul(3))
                .wrapping_add(snapshot.shared_ticks);
            let offset = multdemo_wave_sample(phase, amp);
            let y = mid.saturating_sub(offset).clamp(6, h.saturating_sub(7));
            if x != 6 {
                multdemo_draw_line(pixels, width, height, prev_x, prev_y, x, y, 0x80FFB4);
            }
            prev_x = x;
            prev_y = y;
        }

        let total_updates = snapshot
            .updates
            .iter()
            .copied()
            .fold(0u32, |acc, value| acc.saturating_add(value))
            .max(1);
        let bar_colors = [0x3BA7E4, 0xEE9A3A, 0x4EE17C];
        for slot in 0..MULTDEMO_WORKERS {
            let y = h.saturating_sub(24).saturating_add((slot as i32) * 6);
            let usable_w = w.saturating_sub(18).max(8);
            let bar_w = ((usable_w as u64 * snapshot.updates[slot] as u64) / total_updates as u64) as i32;
            multdemo_fill_rect(
                pixels,
                width,
                height,
                9,
                y,
                bar_w.max(4),
                4,
                bar_colors[slot],
            );
        }
    });
}

const MULTDEMO_DIR_32: [(i16, i16); 32] = [
    (256, 0),
    (251, 50),
    (236, 98),
    (212, 142),
    (181, 181),
    (142, 212),
    (98, 236),
    (50, 251),
    (0, 256),
    (-50, 251),
    (-98, 236),
    (-142, 212),
    (-181, 181),
    (-212, 142),
    (-236, 98),
    (-251, 50),
    (-256, 0),
    (-251, -50),
    (-236, -98),
    (-212, -142),
    (-181, -181),
    (-142, -212),
    (-98, -236),
    (-50, -251),
    (0, -256),
    (50, -251),
    (98, -236),
    (142, -212),
    (181, -181),
    (212, -142),
    (236, -98),
    (251, -50),
];

fn multdemo_direction(phase: u32) -> (i32, i32) {
    let index = ((phase >> 3) & 31) as usize;
    let (dx, dy) = MULTDEMO_DIR_32[index];
    (dx as i32, dy as i32)
}

fn multdemo_fill_gradient_vertical(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    top: u32,
    bottom: u32,
) {
    if width == 0 || height == 0 {
        return;
    }

    let top_r = ((top >> 16) & 0xFF) as i32;
    let top_g = ((top >> 8) & 0xFF) as i32;
    let top_b = (top & 0xFF) as i32;
    let bot_r = ((bottom >> 16) & 0xFF) as i32;
    let bot_g = ((bottom >> 8) & 0xFF) as i32;
    let bot_b = (bottom & 0xFF) as i32;
    let denom = (height as i32 - 1).max(1);

    for y in 0..height {
        let t = y as i32;
        let r = top_r + ((bot_r - top_r) * t) / denom;
        let g = top_g + ((bot_g - top_g) * t) / denom;
        let b = top_b + ((bot_b - top_b) * t) / denom;
        let color = ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
        let row = y * width;
        for x in 0..width {
            pixels[row + x] = color;
        }
    }
}

fn multdemo_fill_rect(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: u32,
) {
    if width == 0 || height == 0 || w <= 0 || h <= 0 {
        return;
    }

    let x0 = x.max(0).min(width as i32);
    let y0 = y.max(0).min(height as i32);
    let x1 = x.saturating_add(w).max(0).min(width as i32);
    let y1 = y.saturating_add(h).max(0).min(height as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }

    for row in y0 as usize..y1 as usize {
        let base = row * width;
        for col in x0 as usize..x1 as usize {
            pixels[base + col] = color;
        }
    }
}

fn multdemo_plot(pixels: &mut [u32], width: usize, height: usize, x: i32, y: i32, color: u32) {
    if width == 0 || height == 0 {
        return;
    }
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }

    let idx = y as usize * width + x as usize;
    if idx < pixels.len() {
        pixels[idx] = color;
    }
}

fn multdemo_draw_line(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    color: u32,
) {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        multdemo_plot(pixels, width, height, x0, y0, color);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = err.saturating_mul(2);
        if e2 >= dy {
            err = err.saturating_add(dy);
            x0 = x0.saturating_add(sx);
        }
        if e2 <= dx {
            err = err.saturating_add(dx);
            y0 = y0.saturating_add(sy);
        }
    }
}

fn multdemo_draw_hline(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    y: i32,
    x0: i32,
    x1: i32,
    color: u32,
) {
    if width == 0 || height == 0 {
        return;
    }
    if y < 0 || y >= height as i32 {
        return;
    }

    let left = x0.min(x1).max(0).min(width as i32 - 1);
    let right = x0.max(x1).max(0).min(width as i32 - 1);
    let row = y as usize * width;
    for x in left as usize..=right as usize {
        pixels[row + x] = color;
    }
}

fn multdemo_draw_circle_outline(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    cx: i32,
    cy: i32,
    radius: i32,
    color: u32,
) {
    if radius <= 0 {
        return;
    }

    let mut x = radius;
    let mut y = 0;
    let mut err = 1 - x;

    while x >= y {
        multdemo_plot(pixels, width, height, cx + x, cy + y, color);
        multdemo_plot(pixels, width, height, cx + y, cy + x, color);
        multdemo_plot(pixels, width, height, cx - y, cy + x, color);
        multdemo_plot(pixels, width, height, cx - x, cy + y, color);
        multdemo_plot(pixels, width, height, cx - x, cy - y, color);
        multdemo_plot(pixels, width, height, cx - y, cy - x, color);
        multdemo_plot(pixels, width, height, cx + y, cy - x, color);
        multdemo_plot(pixels, width, height, cx + x, cy - y, color);

        y += 1;
        if err < 0 {
            err += 2 * y + 1;
        } else {
            x -= 1;
            err += 2 * (y - x + 1);
        }
    }
}

fn multdemo_draw_filled_circle(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    cx: i32,
    cy: i32,
    radius: i32,
    color: u32,
) {
    if radius <= 0 {
        return;
    }

    let mut x = radius;
    let mut y = 0;
    let mut err = 1 - x;

    while x >= y {
        multdemo_draw_hline(pixels, width, height, cy + y, cx - x, cx + x, color);
        multdemo_draw_hline(pixels, width, height, cy - y, cx - x, cx + x, color);
        multdemo_draw_hline(pixels, width, height, cy + x, cx - y, cx + y, color);
        multdemo_draw_hline(pixels, width, height, cy - x, cx - y, cx + y, color);

        y += 1;
        if err < 0 {
            err += 2 * y + 1;
        } else {
            x -= 1;
            err += 2 * (y - x + 1);
        }
    }
}

fn multdemo_draw_clock_hand(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    cx: i32,
    cy: i32,
    length: i32,
    phase: u32,
    thickness: i32,
    color: u32,
) {
    let (dx, dy) = multdemo_direction(phase);
    let x1 = cx.saturating_add(dx.saturating_mul(length.max(1)) / 256);
    let y1 = cy.saturating_add(dy.saturating_mul(length.max(1)) / 256);

    let nx = dy.signum();
    let ny = -dx.signum();
    let half = (thickness.max(1) - 1) / 2;
    for offset in -half..=half {
        multdemo_draw_line(
            pixels,
            width,
            height,
            cx.saturating_add(nx.saturating_mul(offset)),
            cy.saturating_add(ny.saturating_mul(offset)),
            x1.saturating_add(nx.saturating_mul(offset)),
            y1.saturating_add(ny.saturating_mul(offset)),
            color,
        );
    }
}

fn multdemo_draw_gear(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    cx: i32,
    cy: i32,
    radius: i32,
    rotation: u32,
    fill: u32,
    edge: u32,
) {
    if radius <= 4 {
        return;
    }

    multdemo_draw_filled_circle(pixels, width, height, cx, cy, radius, fill);
    multdemo_draw_circle_outline(pixels, width, height, cx, cy, radius, edge);
    multdemo_draw_circle_outline(
        pixels,
        width,
        height,
        cx,
        cy,
        radius.saturating_sub(2),
        0x1A1A1A,
    );

    for tooth in 0..12 {
        let phase = rotation.wrapping_add((tooth as u32).saturating_mul(256 / 12));
        let (dx, dy) = multdemo_direction(phase);
        let x0 = cx.saturating_add(dx.saturating_mul(radius.saturating_sub(2)) / 256);
        let y0 = cy.saturating_add(dy.saturating_mul(radius.saturating_sub(2)) / 256);
        let x1 = cx.saturating_add(dx.saturating_mul(radius.saturating_add(6)) / 256);
        let y1 = cy.saturating_add(dy.saturating_mul(radius.saturating_add(6)) / 256);
        multdemo_draw_line(pixels, width, height, x0, y0, x1, y1, edge);
    }

    for spoke in 0..4 {
        let phase = rotation.wrapping_add((spoke as u32).saturating_mul(64));
        let (dx, dy) = multdemo_direction(phase);
        let x1 = cx.saturating_add(dx.saturating_mul(radius.saturating_sub(8)) / 256);
        let y1 = cy.saturating_add(dy.saturating_mul(radius.saturating_sub(8)) / 256);
        multdemo_draw_line(pixels, width, height, cx, cy, x1, y1, 0xE9DBBE);
    }

    multdemo_draw_filled_circle(
        pixels,
        width,
        height,
        cx,
        cy,
        (radius / 4).max(3),
        0x111111,
    );
}

fn multdemo_triangle_wave(phase: u32) -> i32 {
    let value = (phase & 0xFF) as i32;
    if value < 128 {
        value.saturating_mul(2).saturating_sub(127)
    } else {
        (255 - value).saturating_mul(2).saturating_sub(127)
    }
}

fn multdemo_wave_sample(phase: u32, amplitude: i32) -> i32 {
    let a = multdemo_triangle_wave(phase);
    let b = multdemo_triangle_wave(phase.wrapping_mul(3).wrapping_add(37));
    let c = multdemo_triangle_wave(phase.wrapping_mul(5).wrapping_add(91));
    (a.saturating_mul(amplitude)
        + b.saturating_mul(amplitude / 2)
        + c.saturating_mul(amplitude / 4))
        / 127
}

#[derive(Clone, Copy)]
struct MultDemoBenchWorkerArg {
    shared: *const MultDemoBenchShared,
    slot: usize,
    iterations: u32,
    pause_ticks: u32,
}

#[derive(Clone, Copy)]
struct MultDemoBenchSnapshot {
    total_ops: u32,
    per_worker: [u32; MULTDEMO_WORKERS],
    switches: u32,
}

struct MultDemoBenchState {
    total_ops: u32,
    per_worker: [u32; MULTDEMO_WORKERS],
    switches: u32,
    last_task_id: Option<u32>,
}

impl MultDemoBenchState {
    const fn new() -> Self {
        Self {
            total_ops: 0,
            per_worker: [0; MULTDEMO_WORKERS],
            switches: 0,
            last_task_id: None,
        }
    }
}

struct MultDemoBenchShared {
    state: sync::Mutex<MultDemoBenchState>,
    gate: sync::Semaphore,
    completed: AtomicU32,
    active_workers: AtomicU32,
    max_parallel: AtomicU32,
}

impl MultDemoBenchShared {
    const fn new() -> Self {
        Self {
            state: sync::Mutex::new(MultDemoBenchState::new()),
            gate: sync::Semaphore::new(2),
            completed: AtomicU32::new(0),
            active_workers: AtomicU32::new(0),
            max_parallel: AtomicU32::new(0),
        }
    }

    fn snapshot(&self) -> MultDemoBenchSnapshot {
        let state = self.state.lock();
        MultDemoBenchSnapshot {
            total_ops: state.total_ops,
            per_worker: state.per_worker,
            switches: state.switches,
        }
    }

    fn update_max_parallel(&self, candidate: u32) {
        let mut observed = self.max_parallel.load(Ordering::Acquire);
        while candidate > observed {
            match self.max_parallel.compare_exchange_weak(
                observed,
                candidate,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(current) => observed = current,
            }
        }
    }
}

fn multdemo_bench_worker_entry(raw_arg: *mut u8) {
    let Some(arg_ref) = (unsafe { (raw_arg as *const MultDemoBenchWorkerArg).as_ref() }) else {
        return;
    };
    let arg = *arg_ref;
    let Some(shared) = (unsafe { arg.shared.as_ref() }) else {
        return;
    };

    let mut use_sleep = false;
    for iteration in 0..arg.iterations {
        shared.gate.acquire();

        let active = shared
            .active_workers
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        shared.update_max_parallel(active);

        {
            let mut state = shared.state.lock();
            state.total_ops = state.total_ops.saturating_add(1);
            if arg.slot < state.per_worker.len() {
                state.per_worker[arg.slot] = state.per_worker[arg.slot].saturating_add(1);
            }

            let current_task = task::current_task_id().unwrap_or(0);
            if state.last_task_id != Some(current_task) {
                state.switches = state.switches.saturating_add(1);
                state.last_task_id = Some(current_task);
            }
        }

        if (iteration.wrapping_add(arg.slot as u32) % 6) == 0 {
            task::yield_now();
        }

        shared.active_workers.fetch_sub(1, Ordering::AcqRel);
        shared.gate.release();

        if use_sleep {
            task::sleep_ticks(arg.pause_ticks.max(1));
        } else {
            task::yield_now();
        }
        use_sleep = !use_sleep;
    }

    shared.completed.fetch_add(1, Ordering::AcqRel);
}

fn handle_multdemo_command<'a, I>(mut parts: I)
where
    I: Iterator<Item = &'a str>,
{
    let Some(first) = parts.next() else {
        handle_multdemo_graphics_command();
        return;
    };

    match first {
        "bench" => handle_multdemo_bench_command(parts),
        "gui" | "gfx" => {
            if parts.next().is_some() {
                shell_println!("usage: multdemo | multdemo bench [iterations]");
                return;
            }
            handle_multdemo_graphics_command();
        }
        "-h" | "--help" | "help" => {
            shell_println!("usage: multdemo");
            shell_println!("       multdemo bench [iterations]");
        }
        token if parse_u32(token).is_some() => {
            handle_multdemo_bench_command(core::iter::once(token).chain(parts));
        }
        _ => {
            shell_println!("multdemo: unknown mode '{}'", first);
            shell_println!("usage: multdemo | multdemo bench [iterations]");
        }
    }
}

fn handle_multdemo_bench_command<'a, I>(mut parts: I)
where
    I: Iterator<Item = &'a str>,
{
    let mut iterations = MULTDEMO_DEFAULT_ITERATIONS;
    if let Some(token) = parts.next() {
        let Some(parsed) = parse_u32(token) else {
            shell_println!("multdemo bench: invalid iteration count '{}'", token);
            shell_println!("usage: multdemo bench [iterations]");
            return;
        };
        if parsed == 0 {
            shell_println!("multdemo bench: iterations must be > 0");
            return;
        }
        if parsed > MULTDEMO_MAX_ITERATIONS {
            iterations = MULTDEMO_MAX_ITERATIONS;
            shell_println!(
                "multdemo bench: capping iterations from {} to {}",
                parsed,
                MULTDEMO_MAX_ITERATIONS
            );
        } else {
            iterations = parsed;
        }
    }

    if parts.next().is_some() {
        shell_println!("usage: multdemo bench [iterations]");
        return;
    }

    let shared = MultDemoBenchShared::new();
    let mut worker_args = [MultDemoBenchWorkerArg {
        shared: core::ptr::null(),
        slot: 0,
        iterations: 0,
        pause_ticks: 1,
    }; MULTDEMO_WORKERS];
    let mut worker_ids = [0u32; MULTDEMO_WORKERS];
    let mut spawned = 0u32;

    shell_println!(
        "multdemo bench: spawning {} workers, {} iterations each",
        MULTDEMO_WORKERS,
        iterations
    );
    shell_println!("multdemo bench: semaphore permits=2, mutex-protected shared counters");

    for slot in 0..MULTDEMO_WORKERS {
        worker_args[slot] = MultDemoBenchWorkerArg {
            shared: core::ptr::from_ref(&shared),
            slot,
            iterations,
            pause_ticks: (slot as u32).saturating_add(1),
        };

        let arg_ptr = (&mut worker_args[slot] as *mut MultDemoBenchWorkerArg).cast::<u8>();
        match task::spawn_kernel(multdemo_bench_worker_entry, arg_ptr) {
            Ok(id) => {
                worker_ids[slot] = id;
                spawned = spawned.saturating_add(1);
                shell_println!("multdemo bench: worker {} task_id={}", slot, id);
            }
            Err(reason) => {
                shell_println!(
                    "multdemo bench: failed to spawn worker {}: {}",
                    slot,
                    reason
                );
                break;
            }
        }
    }

    if spawned == 0 {
        shell_println!("multdemo bench: no workers spawned");
        return;
    }

    let expected_ops = iterations.saturating_mul(spawned);
    let start_tick = timer::ticks();
    let mut last_report = start_tick;

    loop {
        let done = shared.completed.load(Ordering::Acquire);
        if done >= spawned {
            break;
        }

        let now = timer::ticks();
        if now.wrapping_sub(last_report) >= MULTDEMO_PROGRESS_TICKS {
            let snap = shared.snapshot();
            shell_println!(
                "multdemo bench: progress done={}/{} ops={}/{} switches={}",
                done,
                spawned,
                snap.total_ops,
                expected_ops,
                snap.switches
            );
            last_report = now;
        }

        task::sleep_ticks(2);
    }

    let elapsed_ticks = timer::ticks().wrapping_sub(start_tick);
    let hz = timer::frequency_hz().max(1);
    let elapsed_ms = (elapsed_ticks as u64).saturating_mul(1000) / hz as u64;
    let snap = shared.snapshot();
    let max_parallel = shared.max_parallel.load(Ordering::Acquire);

    shell_println!(
        "multdemo bench: finished in {} ticks ({} ms at {} Hz)",
        elapsed_ticks,
        elapsed_ms,
        hz
    );
    shell_println!(
        "multdemo bench: totals ops={} expected={} switches={} max_parallel={}",
        snap.total_ops,
        expected_ops,
        snap.switches,
        max_parallel
    );

    for slot in 0..spawned as usize {
        shell_println!(
            "multdemo bench: worker {} (task {}) ops={}",
            slot,
            worker_ids[slot],
            snap.per_worker[slot]
        );
    }

    let per_worker_ok = (0..spawned as usize).all(|slot| snap.per_worker[slot] == iterations);
    let pass = snap.total_ops == expected_ops && per_worker_ok && max_parallel <= 2;
    shell_println!(
        "multdemo bench: {}",
        if pass {
            "PASS"
        } else {
            "CHECK RESULTS ABOVE"
        }
    );
}

fn handle_mouse_command() {
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

    shell_println!(
        "ata disk: {}",
        if info.present { "present" } else { "missing" }
    );
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

fn handle_fsdelete_command<'a, I>(mut parts: I)
where
    I: Iterator<Item = &'a str>,
{
    let Some(name) = parts.next() else {
        shell_println!("usage: fsdelete <name>");
        return;
    };

    match fs::delete_file(name) {
        Ok(()) => shell_println!("deleted {}", name),
        Err(error) => shell_println!("fsdelete failed: {}", error.as_str()),
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

fn handle_edit_command<'a, I>(mut parts: I)
where
    I: Iterator<Item = &'a str>,
{
    let Some(filename) = parts.next() else {
        shell_println!("usage: edit <name>");
        return;
    };

    if parts.next().is_some() {
        shell_println!("usage: edit <name>");
        return;
    }

    let mut document = TextDocument::new();
    let mut load_buffer = [0u8; EDITOR_MAX_BYTES];

    match fs::read_file(filename, &mut load_buffer) {
        Ok(result) => {
            let truncated = document.load_from_bytes(&load_buffer[..result.copied_size]);
            if truncated {
                shell_println!("editor: file was truncated to editor limits");
            }
            shell_println!(
                "editor: loaded {} bytes from {}",
                result.total_size,
                filename
            );
        }
        Err(fs::FsError::NotFound) => {
            document.load_from_bytes(&[]);
            shell_println!("editor: creating new file {}", filename);
        }
        Err(error) => {
            shell_println!("editor: failed to load {}: {}", filename, error.as_str());
            return;
        }
    }

    shell_println!("editor commands:");
    shell_println!("  <text>         append line");
    shell_println!("  .show          show document");
    shell_println!("  .ins N <text>  insert before line N");
    shell_println!("  .set N <text>  replace line N");
    shell_println!("  .del N         delete line N");
    shell_println!("  .save          save");
    shell_println!("  .wq            save and quit");
    shell_println!("  .quit          quit without saving");

    let mut dirty = false;
    loop {
        let mut input = [0u8; MAX_LINE];
        let len = read_line_interactive("edit> ", &mut input);
        let line = core::str::from_utf8(&input[..len])
            .unwrap_or("")
            .trim_end_matches('\r');

        if line.is_empty() {
            continue;
        }

        if line == ".help" {
            shell_println!("editor commands: .show .ins .set .del .save .wq .quit");
            continue;
        }

        if line == ".show" {
            print_document(&document);
            continue;
        }

        if line == ".quit" {
            if dirty {
                shell_println!("editor: unsaved changes discarded");
            }
            shell_println!("editor: exit");
            return;
        }

        if line == ".save" || line == ".wq" {
            let mut save_buffer = [0u8; EDITOR_MAX_BYTES];
            match document.write_to_buffer(&mut save_buffer) {
                Ok(size) => match fs::write_file(filename, &save_buffer[..size]) {
                    Ok(()) => {
                        dirty = false;
                        shell_println!("editor: saved {} bytes to {}", size, filename);
                        if line == ".wq" {
                            shell_println!("editor: exit");
                            return;
                        }
                    }
                    Err(error) => {
                        shell_println!("editor: save failed: {}", error.as_str());
                    }
                },
                Err(error) => shell_println!("editor: save failed: {}", error),
            }
            continue;
        }

        if let Some(token) = line.strip_prefix(".del ") {
            let line_no = parse_u32(token.trim()).unwrap_or(0) as usize;
            match document.delete_line(line_no) {
                Ok(()) => {
                    dirty = true;
                    shell_println!("editor: deleted line {}", line_no);
                }
                Err(error) => shell_println!("editor: {}", error),
            }
            continue;
        }

        if let Some((line_no, text)) = parse_editor_line_text_command(line, ".set ") {
            let bytes = text.as_bytes();
            match document.set_line(line_no, bytes) {
                Ok(()) => {
                    dirty = true;
                    shell_println!("editor: set line {}", line_no);
                }
                Err(error) => shell_println!("editor: {}", error),
            }
            continue;
        }

        if let Some((line_no, text)) = parse_editor_line_text_command(line, ".ins ") {
            let bytes = text.as_bytes();
            match document.insert_line(line_no, bytes) {
                Ok(()) => {
                    dirty = true;
                    shell_println!("editor: inserted at line {}", line_no);
                }
                Err(error) => shell_println!("editor: {}", error),
            }
            continue;
        }

        if line.starts_with('.') {
            shell_println!("editor: unknown command '{}'", line);
            continue;
        }

        match document.append_line(line.as_bytes()) {
            Ok(()) => {
                dirty = true;
                shell_println!("editor: appended line {}", document.count);
            }
            Err(error) => shell_println!("editor: {}", error),
        }
    }
}

fn read_line_interactive(prompt: &str, line: &mut [u8; MAX_LINE]) -> usize {
    let mut len = 0usize;
    let mut cursor = 0usize;
    shell_print!("{}", prompt);

    loop {
        if let Some(key) = read_input() {
            match key {
                KeyEvent::Char('\n') => {
                    shell_println!();
                    return len;
                }
                KeyEvent::Char('\x08') => handle_backspace(line, &mut len, &mut cursor),
                KeyEvent::Char('\t') => {
                    for _ in 0..4 {
                        insert_input_char(line, &mut len, &mut cursor, ' ');
                    }
                }
                KeyEvent::Char(ch) => {
                    if is_printable(ch) {
                        insert_input_char(line, &mut len, &mut cursor, ch);
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

fn print_document(document: &TextDocument) {
    if document.count == 0 {
        shell_println!("editor: document is empty");
        return;
    }

    for index in 0..document.count {
        let text =
            core::str::from_utf8(&document.lines[index][..document.lengths[index]]).unwrap_or("?");
        shell_println!("{:03}: {}", index + 1, text);
    }
}

fn parse_editor_line_text_command<'a>(line: &'a str, prefix: &str) -> Option<(usize, &'a str)> {
    let rest = line.strip_prefix(prefix)?;
    let rest = rest.trim_start();
    if rest.is_empty() {
        return None;
    }

    let mut split = rest.splitn(2, |ch: char| ch.is_ascii_whitespace());
    let line_token = split.next()?;
    let text = split.next().unwrap_or("").trim_start();
    let line_no = parse_u32(line_token)? as usize;
    Some((line_no, text))
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
