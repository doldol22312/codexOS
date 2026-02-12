use super::*;

const EDITOR_MAX_LINES: usize = 128;
const EDITOR_MAX_LINE_LEN: usize = 200;
const EDITOR_MAX_BYTES: usize = 4096;

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

pub(super) fn handle_edit_command<'a, I>(mut parts: I)
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
