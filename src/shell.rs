use core::str;

use crate::{
    allocator, ata, fs,
    input::{self, InputEvent, KeyEvent},
    keyboard, matrix, mouse, paging, print, println, reboot, rtc, serial, shutdown, timer, ui,
    vga,
};

const MAX_LINE: usize = 256;
const HISTORY_SIZE: usize = 32;
const MAX_FS_COMPLETION_FILES: usize = 64;
const EDITOR_MAX_LINES: usize = 128;
const EDITOR_MAX_LINE_LEN: usize = 200;
const EDITOR_MAX_BYTES: usize = 4096;
const UIDEMO_BUTTON_PING_ID: u16 = 10;
const UIDEMO_BUTTON_EXIT_ID: u16 = 11;

const COMMANDS: [&str; 28] = [
    "help", "clear", "echo", "info", "disk", "fsinfo", "fsformat", "fsls", "fswrite", "fsdelete",
    "fscat", "edit", "date", "time", "rtc", "paging", "uptime", "heap", "memtest", "hexdump",
    "mouse", "matrix", "gfxdemo", "uidemo", "color", "reboot", "shutdown", "panic",
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
            shell_println!("  gfxdemo - draw framebuffer primitives demo");
            shell_println!("  uidemo - UI dispatcher/widgets demo");
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
        "gfxdemo" => {
            handle_gfxdemo_command();
        }
        "uidemo" => {
            handle_uidemo_command();
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
