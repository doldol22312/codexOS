#![allow(dead_code)]

extern crate alloc;

use alloc::vec::Vec;
use core::cmp::{max, min};

use crate::input::{self, HitRegion, InputEvent, KeyEvent, MouseButton};
use crate::vga;

pub type WidgetId = u16;

pub const MAX_WIDGETS: usize = 32;

const EMPTY_REGION: HitRegion = HitRegion {
    id: 0,
    x: 0,
    y: 0,
    width: 0,
    height: 0,
};

const CLIPBOARD_CAPACITY: usize = 1024;
static mut CLIPBOARD: [u8; CLIPBOARD_CAPACITY] = [0; CLIPBOARD_CAPACITY];
static mut CLIPBOARD_LEN: usize = 0;

const COLOR_TEXT: u32 = 0xE8F1FF;
const COLOR_TEXT_MUTED: u32 = 0x8392AB;
const COLOR_BG_DARK: u32 = 0x0F1523;
const COLOR_BG_MID: u32 = 0x172033;
const COLOR_BG_LIGHT: u32 = 0x24334E;
const COLOR_BORDER: u32 = 0x5C6B85;
const COLOR_BORDER_FOCUS: u32 = 0x39FF14;
const COLOR_SELECTION: u32 = 0x2C568B;
const COLOR_ACCENT: u32 = 0x00E5FF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

#[inline]
fn is_printable_ascii(ch: char) -> bool {
    ch >= ' ' && ch <= '~'
}

#[inline]
fn border_color(focused: bool) -> u32 {
    if focused {
        COLOR_BORDER_FOCUS
    } else {
        COLOR_BORDER
    }
}

fn popup_y_for_anchor(anchor: Rect, popup_height: i32) -> i32 {
    let below_y = anchor.y.saturating_add(anchor.height);
    if popup_height <= 0 {
        return below_y;
    }

    let mut y = below_y;
    if let Some((_, fb_h)) = vga::framebuffer_resolution() {
        let fb_h = fb_h as i32;
        let above_y = anchor.y.saturating_sub(popup_height);
        let fits_below = below_y.saturating_add(popup_height) <= fb_h;
        let fits_above = above_y >= 0;
        if !fits_below && fits_above {
            y = above_y;
        } else if !fits_below {
            y = fb_h.saturating_sub(popup_height).max(0);
        }
    }

    y
}

fn union_rect(a: Rect, b: Rect) -> Rect {
    let x0 = min(a.x, b.x);
    let y0 = min(a.y, b.y);
    let x1 = max(a.x.saturating_add(a.width), b.x.saturating_add(b.width));
    let y1 = max(a.y.saturating_add(a.height), b.y.saturating_add(b.height));
    Rect::new(x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
}

fn draw_rect_outline(rect: Rect, color: u32) {
    let _ = vga::draw_horizontal_line(rect.x, rect.y, rect.width, color);
    let _ = vga::draw_horizontal_line(
        rect.x,
        rect.y.saturating_add(rect.height).saturating_sub(1),
        rect.width,
        color,
    );
    let _ = vga::draw_vertical_line(rect.x, rect.y, rect.height, color);
    let _ = vga::draw_vertical_line(
        rect.x.saturating_add(rect.width).saturating_sub(1),
        rect.y,
        rect.height,
        color,
    );
}

fn draw_text_line(x: i32, y: i32, text: &[u8], fg: u32, bg: u32) {
    if text.is_empty() {
        return;
    }
    if let Ok(value) = core::str::from_utf8(text) {
        let _ = vga::draw_text(x, y, value, fg, bg);
    }
}

fn write_clipboard(bytes: &[u8]) {
    unsafe {
        let copy_len = bytes.len().min(CLIPBOARD_CAPACITY);
        CLIPBOARD[..copy_len].copy_from_slice(&bytes[..copy_len]);
        CLIPBOARD_LEN = copy_len;
    }
}

fn read_clipboard() -> Vec<u8> {
    let mut out = Vec::new();
    unsafe {
        if CLIPBOARD_LEN > 0 {
            out.extend_from_slice(&CLIPBOARD[..CLIPBOARD_LEN]);
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        if self.width <= 0 || self.height <= 0 {
            return false;
        }

        let x1 = self.x.saturating_add(self.width);
        let y1 = self.y.saturating_add(self.height);
        px >= self.x && py >= self.y && px < x1 && py < y1
    }

    pub fn as_hit_region(&self, id: WidgetId) -> HitRegion {
        HitRegion {
            id,
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WidgetResponse {
    pub redraw: bool,
    pub clicked: bool,
}

impl WidgetResponse {
    #[inline]
    pub const fn none() -> Self {
        Self {
            redraw: false,
            clicked: false,
        }
    }
}

pub struct Panel {
    pub id: WidgetId,
    pub rect: Rect,
    pub background: u32,
    pub border: u32,
}

impl Panel {
    pub const fn new(id: WidgetId, rect: Rect, background: u32, border: u32) -> Self {
        Self {
            id,
            rect,
            background,
            border,
        }
    }

    pub fn draw(&self, _focused: bool) {
        let _ = vga::draw_filled_rect(
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            self.background,
        );
        let _ = vga::draw_horizontal_line(self.rect.x, self.rect.y, self.rect.width, self.border);
        let _ = vga::draw_horizontal_line(
            self.rect.x,
            self.rect
                .y
                .saturating_add(self.rect.height)
                .saturating_sub(1),
            self.rect.width,
            self.border,
        );
        let _ = vga::draw_vertical_line(self.rect.x, self.rect.y, self.rect.height, self.border);
        let _ = vga::draw_vertical_line(
            self.rect
                .x
                .saturating_add(self.rect.width)
                .saturating_sub(1),
            self.rect.y,
            self.rect.height,
            self.border,
        );
    }

    pub fn handle_event(&mut self, _event: &InputEvent, _focused: bool) -> WidgetResponse {
        WidgetResponse::none()
    }
}

pub struct Label {
    pub id: WidgetId,
    pub rect: Rect,
    pub text: &'static str,
    pub foreground: u32,
    pub background: u32,
}

impl Label {
    pub const fn new(
        id: WidgetId,
        rect: Rect,
        text: &'static str,
        foreground: u32,
        background: u32,
    ) -> Self {
        Self {
            id,
            rect,
            text,
            foreground,
            background,
        }
    }

    pub fn draw(&self, _focused: bool) {
        let _ = vga::draw_filled_rect(
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            self.background,
        );

        let Some((font_w, font_h)) = vga::font_metrics() else {
            return;
        };

        let text_width = self.text.len().saturating_mul(font_w) as i32;
        let text_height = font_h as i32;
        let text_x = self
            .rect
            .x
            .saturating_add(((self.rect.width - text_width) / 2).max(2));
        let text_y = self
            .rect
            .y
            .saturating_add(((self.rect.height - text_height) / 2).max(1));

        let _ = vga::draw_text(text_x, text_y, self.text, self.foreground, self.background);
    }

    pub fn handle_event(&mut self, _event: &InputEvent, _focused: bool) -> WidgetResponse {
        WidgetResponse::none()
    }
}

pub struct Button {
    pub id: WidgetId,
    pub rect: Rect,
    pub text: &'static str,
    pub text_color: u32,
    pub fill_normal: u32,
    pub fill_hover: u32,
    pub fill_pressed: u32,
    pub border: u32,
    pub border_focused: u32,
    hovered: bool,
    pressed: bool,
}

impl Button {
    pub const fn new(id: WidgetId, rect: Rect, text: &'static str) -> Self {
        Self {
            id,
            rect,
            text,
            text_color: 0xFFFFFF,
            fill_normal: 0x20293A,
            fill_hover: 0x2F3B53,
            fill_pressed: 0x111A29,
            border: 0x5C6B85,
            border_focused: 0x39FF14,
            hovered: false,
            pressed: false,
        }
    }

    pub fn draw(&self, focused: bool) {
        let fill = if self.pressed {
            self.fill_pressed
        } else if self.hovered {
            self.fill_hover
        } else {
            self.fill_normal
        };
        let border = if focused {
            self.border_focused
        } else {
            self.border
        };

        let _ = vga::draw_filled_rect(self.rect.x, self.rect.y, self.rect.width, self.rect.height, fill);
        let _ = vga::draw_horizontal_line(self.rect.x, self.rect.y, self.rect.width, border);
        let _ = vga::draw_horizontal_line(
            self.rect.x,
            self.rect
                .y
                .saturating_add(self.rect.height)
                .saturating_sub(1),
            self.rect.width,
            border,
        );
        let _ = vga::draw_vertical_line(self.rect.x, self.rect.y, self.rect.height, border);
        let _ = vga::draw_vertical_line(
            self.rect
                .x
                .saturating_add(self.rect.width)
                .saturating_sub(1),
            self.rect.y,
            self.rect.height,
            border,
        );

        let Some((font_w, font_h)) = vga::font_metrics() else {
            return;
        };

        let text_width = self.text.len().saturating_mul(font_w) as i32;
        let text_height = font_h as i32;
        let text_x = self.rect.x.saturating_add(((self.rect.width - text_width) / 2).max(2));
        let text_y = self
            .rect
            .y
            .saturating_add(((self.rect.height - text_height) / 2).max(1));
        let _ = vga::draw_text(text_x, text_y, self.text, self.text_color, fill);
    }

    pub fn handle_event(&mut self, event: &InputEvent, focused: bool) -> WidgetResponse {
        match *event {
            InputEvent::MouseDown {
                button: MouseButton::Left,
                x,
                y,
            } => {
                if self.rect.contains(x, y) {
                    self.pressed = true;
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }
            }
            InputEvent::MouseUp {
                button: MouseButton::Left,
                ..
            } => {
                if self.pressed {
                    self.pressed = false;
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }
            }
            InputEvent::MouseClick {
                button: MouseButton::Left,
                x,
                y,
            } => {
                if self.rect.contains(x, y) {
                    self.pressed = false;
                    return WidgetResponse {
                        redraw: true,
                        clicked: true,
                    };
                }
            }
            InputEvent::KeyPress {
                key: KeyEvent::Char('\n'),
            }
            | InputEvent::KeyPress {
                key: KeyEvent::Char(' '),
            } => {
                if focused {
                    return WidgetResponse {
                        redraw: true,
                        clicked: true,
                    };
                }
            }
            _ => {}
        }

        WidgetResponse::none()
    }

    #[inline]
    pub fn set_hovered(&mut self, hovered: bool) -> bool {
        if self.hovered == hovered {
            return false;
        }
        self.hovered = hovered;
        true
    }
}

pub struct TextBox {
    pub id: WidgetId,
    pub rect: Rect,
    pub placeholder: &'static str,
    pub text_color: u32,
    pub placeholder_color: u32,
    pub background: u32,
    pub selection_color: u32,
    pub cursor_color: u32,
    text: Vec<u8>,
    cursor: usize,
    selection_anchor: Option<usize>,
    selecting_with_mouse: bool,
    scroll_x: usize,
}

impl TextBox {
    pub fn new(id: WidgetId, rect: Rect) -> Self {
        Self {
            id,
            rect,
            placeholder: "",
            text_color: COLOR_TEXT,
            placeholder_color: COLOR_TEXT_MUTED,
            background: COLOR_BG_DARK,
            selection_color: COLOR_SELECTION,
            cursor_color: COLOR_ACCENT,
            text: Vec::new(),
            cursor: 0,
            selection_anchor: None,
            selecting_with_mouse: false,
            scroll_x: 0,
        }
    }

    pub fn set_text(&mut self, value: &str) {
        self.text.clear();
        for byte in value.bytes() {
            if (byte as char).is_ascii_graphic() || byte == b' ' {
                self.text.push(byte);
            }
        }
        self.cursor = self.text.len();
        self.scroll_x = 0;
        self.selection_anchor = None;
    }

    pub fn text(&self) -> &[u8] {
        &self.text
    }

    pub fn selection_range(&self) -> Option<(usize, usize)> {
        let start = self.selection_anchor?;
        let end = self.cursor;
        if start == end {
            return None;
        }
        Some((min(start, end), max(start, end)))
    }

    pub fn copy_selection_to_clipboard(&self) {
        if let Some((start, end)) = self.selection_range() {
            write_clipboard(&self.text[start..end]);
        }
    }

    pub fn cut_selection_to_clipboard(&mut self) -> bool {
        let Some((start, end)) = self.selection_range() else {
            return false;
        };
        write_clipboard(&self.text[start..end]);
        self.text.drain(start..end);
        self.cursor = start;
        self.selection_anchor = None;
        true
    }

    pub fn paste_from_clipboard(&mut self) -> bool {
        let pasted = read_clipboard();
        if pasted.is_empty() {
            return false;
        }

        self.delete_selection();
        for byte in pasted {
            if (byte as char).is_ascii_graphic() || byte == b' ' {
                self.text.insert(self.cursor, byte);
                self.cursor += 1;
            }
        }
        true
    }

    pub fn draw(&self, focused: bool) {
        let _ = vga::draw_filled_rect(
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            self.background,
        );
        draw_rect_outline(self.rect, border_color(focused));

        let Some((font_w, font_h)) = vga::font_metrics() else {
            return;
        };

        if self.rect.width <= 8 || self.rect.height <= 4 {
            return;
        }

        let visible_chars = self.visible_chars(font_w);
        let text_y = self
            .rect
            .y
            .saturating_add(((self.rect.height - font_h as i32) / 2).max(1));
        let text_x = self.rect.x + 4;
        let visible_start = self.scroll_x.min(self.text.len());
        let visible_end = min(self.text.len(), visible_start.saturating_add(visible_chars));

        if let Some((sel_start, sel_end)) = self.selection_range() {
            let draw_start = max(sel_start, visible_start);
            let draw_end = min(sel_end, visible_end);
            if draw_start < draw_end {
                let from = draw_start - visible_start;
                let to = draw_end - visible_start;
                let width = (to - from) as i32 * font_w as i32;
                let sel_x = text_x.saturating_add(from as i32 * font_w as i32);
                let _ = vga::draw_filled_rect(sel_x, text_y, width, font_h as i32, self.selection_color);
            }
        }

        if visible_start < visible_end {
            draw_text_line(
                text_x,
                text_y,
                &self.text[visible_start..visible_end],
                self.text_color,
                self.background,
            );
        } else if self.text.is_empty() && !self.placeholder.is_empty() {
            let _ = vga::draw_text(
                text_x,
                text_y,
                self.placeholder,
                self.placeholder_color,
                self.background,
            );
        }

        if focused {
            let cursor_col = self.cursor.saturating_sub(visible_start).min(visible_chars);
            let cursor_x = text_x.saturating_add(cursor_col as i32 * font_w as i32);
            let _ = vga::draw_filled_rect(cursor_x, text_y, 1, font_h as i32, self.cursor_color);
        }
    }

    pub fn handle_event(&mut self, event: &InputEvent, focused: bool) -> WidgetResponse {
        match *event {
            InputEvent::MouseDown {
                button: MouseButton::Left,
                x,
                y,
            } => {
                if self.rect.contains(x, y) {
                    let next_cursor = self.cursor_from_x(x);
                    self.cursor = next_cursor;
                    self.selection_anchor = Some(next_cursor);
                    self.selecting_with_mouse = true;
                    self.ensure_cursor_visible();
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }
            }
            InputEvent::MouseMove { x, y, .. } => {
                if self.selecting_with_mouse {
                    let clamped_x = x.clamp(self.rect.x, self.rect.x.saturating_add(self.rect.width));
                    let clamped_y = y.clamp(self.rect.y, self.rect.y.saturating_add(self.rect.height));
                    if self.rect.contains(clamped_x, clamped_y) || self.rect.contains(x, y) {
                        let next_cursor = self.cursor_from_x(clamped_x);
                        if next_cursor != self.cursor {
                            self.cursor = next_cursor;
                            self.ensure_cursor_visible();
                            return WidgetResponse {
                                redraw: true,
                                clicked: false,
                            };
                        }
                    }
                }
            }
            InputEvent::MouseUp {
                button: MouseButton::Left,
                ..
            } => {
                if self.selecting_with_mouse {
                    self.selecting_with_mouse = false;
                    if self.selection_range().is_none() {
                        self.selection_anchor = None;
                    }
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }
            }
            InputEvent::KeyPress { key } if focused => {
                let changed = match key {
                    KeyEvent::Left => {
                        if self.cursor > 0 {
                            self.cursor -= 1;
                            self.selection_anchor = None;
                            self.ensure_cursor_visible();
                            true
                        } else {
                            false
                        }
                    }
                    KeyEvent::Right => {
                        if self.cursor < self.text.len() {
                            self.cursor += 1;
                            self.selection_anchor = None;
                            self.ensure_cursor_visible();
                            true
                        } else {
                            false
                        }
                    }
                    KeyEvent::Char('\x08') => {
                        if self.delete_selection() {
                            true
                        } else if self.cursor > 0 {
                            let remove_at = self.cursor - 1;
                            self.text.remove(remove_at);
                            self.cursor = remove_at;
                            self.ensure_cursor_visible();
                            true
                        } else {
                            false
                        }
                    }
                    KeyEvent::Char('\x03') => {
                        self.copy_selection_to_clipboard();
                        false
                    }
                    KeyEvent::Char('\x18') => self.cut_selection_to_clipboard(),
                    KeyEvent::Char('\x16') => self.paste_from_clipboard(),
                    KeyEvent::Char(ch) if is_printable_ascii(ch) => {
                        self.delete_selection();
                        self.text.insert(self.cursor, ch as u8);
                        self.cursor += 1;
                        self.ensure_cursor_visible();
                        true
                    }
                    _ => false,
                };

                if changed {
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }
            }
            _ => {}
        }

        WidgetResponse::none()
    }

    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection_range() else {
            return false;
        };
        self.text.drain(start..end);
        self.cursor = start;
        self.selection_anchor = None;
        self.ensure_cursor_visible();
        true
    }

    fn visible_chars(&self, font_w: usize) -> usize {
        let available = self.rect.width.saturating_sub(8).max(1) as usize;
        max(1, available / max(font_w, 1))
    }

    fn ensure_cursor_visible(&mut self) {
        let Some((font_w, _)) = vga::font_metrics() else {
            return;
        };
        let visible = self.visible_chars(font_w);
        if self.cursor < self.scroll_x {
            self.scroll_x = self.cursor;
        } else if self.cursor >= self.scroll_x.saturating_add(visible) {
            self.scroll_x = self.cursor.saturating_sub(visible.saturating_sub(1));
        }
    }

    fn cursor_from_x(&self, x: i32) -> usize {
        let Some((font_w, _)) = vga::font_metrics() else {
            return self.cursor.min(self.text.len());
        };
        let inner_x = self.rect.x + 4;
        let relative = x.saturating_sub(inner_x).max(0) as usize;
        let char_col = relative / max(font_w, 1);
        min(self.text.len(), self.scroll_x.saturating_add(char_col))
    }
}

pub struct TextArea {
    pub id: WidgetId,
    pub rect: Rect,
    pub placeholder: &'static str,
    pub text_color: u32,
    pub placeholder_color: u32,
    pub background: u32,
    pub cursor_color: u32,
    text: Vec<u8>,
    cursor: usize,
    scroll_x: usize,
    scroll_y: usize,
}

impl TextArea {
    pub fn new(id: WidgetId, rect: Rect) -> Self {
        Self {
            id,
            rect,
            placeholder: "",
            text_color: COLOR_TEXT,
            placeholder_color: COLOR_TEXT_MUTED,
            background: COLOR_BG_DARK,
            cursor_color: COLOR_ACCENT,
            text: Vec::new(),
            cursor: 0,
            scroll_x: 0,
            scroll_y: 0,
        }
    }

    pub fn set_text(&mut self, value: &str) {
        self.text.clear();
        for byte in value.bytes() {
            if byte == b'\n' || (byte as char).is_ascii_graphic() || byte == b' ' {
                self.text.push(byte);
            }
        }
        self.cursor = self.text.len();
        self.scroll_x = 0;
        self.scroll_y = 0;
    }

    pub fn draw(&self, focused: bool) {
        let _ = vga::draw_filled_rect(
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            self.background,
        );
        draw_rect_outline(self.rect, border_color(focused));

        let Some((font_w, font_h)) = vga::font_metrics() else {
            return;
        };

        let rows = self.visible_rows(font_h);
        let cols = self.visible_cols(font_w);
        if rows == 0 || cols == 0 {
            return;
        }

        if self.text.is_empty() && !self.placeholder.is_empty() {
            let _ = vga::draw_text(
                self.rect.x + 4,
                self.rect.y + 4,
                self.placeholder,
                self.placeholder_color,
                self.background,
            );
        } else {
            for row in 0..rows {
                let line_index = self.scroll_y + row;
                let Some((start, end)) = self.line_bounds(line_index) else {
                    break;
                };
                let line_start = min(end, start.saturating_add(self.scroll_x));
                let line_end = min(end, line_start.saturating_add(cols));
                if line_start < line_end {
                    let y = self.rect.y + 4 + row as i32 * font_h as i32;
                    draw_text_line(
                        self.rect.x + 4,
                        y,
                        &self.text[line_start..line_end],
                        self.text_color,
                        self.background,
                    );
                }
            }
        }

        if focused {
            let (cursor_line, cursor_col) = self.line_col_from_index(self.cursor);
            if cursor_line >= self.scroll_y && cursor_line < self.scroll_y + rows {
                let visible_col = cursor_col.saturating_sub(self.scroll_x).min(cols);
                let x = self.rect.x + 4 + visible_col as i32 * font_w as i32;
                let y = self.rect.y + 4 + (cursor_line - self.scroll_y) as i32 * font_h as i32;
                let _ = vga::draw_filled_rect(x, y, 1, font_h as i32, self.cursor_color);
            }
        }
    }

    pub fn handle_event(&mut self, event: &InputEvent, focused: bool) -> WidgetResponse {
        match *event {
            InputEvent::MouseDown {
                button: MouseButton::Left,
                x,
                y,
            } => {
                if self.rect.contains(x, y) {
                    self.cursor = self.cursor_from_point(x, y);
                    self.ensure_cursor_visible();
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }
            }
            InputEvent::KeyPress { key } if focused => {
                let changed = match key {
                    KeyEvent::Left => {
                        if self.cursor > 0 {
                            self.cursor -= 1;
                            self.ensure_cursor_visible();
                            true
                        } else {
                            false
                        }
                    }
                    KeyEvent::Right => {
                        if self.cursor < self.text.len() {
                            self.cursor += 1;
                            self.ensure_cursor_visible();
                            true
                        } else {
                            false
                        }
                    }
                    KeyEvent::Up => {
                        self.move_vertical(-1);
                        true
                    }
                    KeyEvent::Down => {
                        self.move_vertical(1);
                        true
                    }
                    KeyEvent::PageUp => {
                        let step = self.visible_rows(vga::font_metrics().map_or(16, |(_, h)| h));
                        self.scroll_y = self.scroll_y.saturating_sub(step);
                        self.ensure_cursor_visible();
                        true
                    }
                    KeyEvent::PageDown => {
                        let step = self.visible_rows(vga::font_metrics().map_or(16, |(_, h)| h));
                        self.scroll_y = self.scroll_y.saturating_add(step);
                        self.ensure_cursor_visible();
                        true
                    }
                    KeyEvent::Char('\x08') => {
                        if self.cursor > 0 {
                            let index = self.cursor - 1;
                            self.text.remove(index);
                            self.cursor = index;
                            self.ensure_cursor_visible();
                            true
                        } else {
                            false
                        }
                    }
                    KeyEvent::Char('\n') => {
                        self.text.insert(self.cursor, b'\n');
                        self.cursor += 1;
                        self.ensure_cursor_visible();
                        true
                    }
                    KeyEvent::Char(ch) if is_printable_ascii(ch) => {
                        self.text.insert(self.cursor, ch as u8);
                        self.cursor += 1;
                        self.ensure_cursor_visible();
                        true
                    }
                    _ => false,
                };

                if changed {
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }
            }
            _ => {}
        }

        WidgetResponse::none()
    }

    fn visible_cols(&self, font_w: usize) -> usize {
        let available = self.rect.width.saturating_sub(8).max(1) as usize;
        max(1, available / max(font_w, 1))
    }

    fn visible_rows(&self, font_h: usize) -> usize {
        let available = self.rect.height.saturating_sub(8).max(1) as usize;
        max(1, available / max(font_h, 1))
    }

    fn line_bounds(&self, line_index: usize) -> Option<(usize, usize)> {
        let mut current_line = 0usize;
        let mut start = 0usize;
        for (index, byte) in self.text.iter().enumerate() {
            if *byte == b'\n' {
                if current_line == line_index {
                    return Some((start, index));
                }
                current_line += 1;
                start = index + 1;
            }
        }

        if current_line == line_index {
            return Some((start, self.text.len()));
        }
        None
    }

    fn line_col_from_index(&self, index: usize) -> (usize, usize) {
        let mut line = 0usize;
        let mut col = 0usize;
        let end = min(index, self.text.len());
        for byte in self.text.iter().take(end) {
            if *byte == b'\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    fn index_from_line_col(&self, target_line: usize, target_col: usize) -> usize {
        let mut line = 0usize;
        let mut col = 0usize;
        for (index, byte) in self.text.iter().enumerate() {
            if line == target_line && col == target_col {
                return index;
            }
            if *byte == b'\n' {
                if line == target_line {
                    return index;
                }
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }

        self.text.len()
    }

    fn ensure_cursor_visible(&mut self) {
        let Some((font_w, font_h)) = vga::font_metrics() else {
            return;
        };
        let rows = self.visible_rows(font_h);
        let cols = self.visible_cols(font_w);
        let (line, col) = self.line_col_from_index(self.cursor);

        if line < self.scroll_y {
            self.scroll_y = line;
        } else if line >= self.scroll_y.saturating_add(rows) {
            self.scroll_y = line.saturating_sub(rows.saturating_sub(1));
        }

        if col < self.scroll_x {
            self.scroll_x = col;
        } else if col >= self.scroll_x.saturating_add(cols) {
            self.scroll_x = col.saturating_sub(cols.saturating_sub(1));
        }
    }

    fn move_vertical(&mut self, delta: i32) {
        let (line, col) = self.line_col_from_index(self.cursor);
        let next_line = if delta < 0 {
            line.saturating_sub((-delta) as usize)
        } else {
            line.saturating_add(delta as usize)
        };
        self.cursor = self.index_from_line_col(next_line, col);
        self.ensure_cursor_visible();
    }

    fn cursor_from_point(&self, x: i32, y: i32) -> usize {
        let Some((font_w, font_h)) = vga::font_metrics() else {
            return self.cursor.min(self.text.len());
        };

        let row = y.saturating_sub(self.rect.y + 4).max(0) as usize / max(font_h, 1);
        let col = x.saturating_sub(self.rect.x + 4).max(0) as usize / max(font_w, 1);
        let line = self.scroll_y.saturating_add(row);
        let column = self.scroll_x.saturating_add(col);
        self.index_from_line_col(line, column)
    }
}

pub struct Checkbox {
    pub id: WidgetId,
    pub rect: Rect,
    pub label: &'static str,
    pub checked: bool,
    pub background: u32,
    pub text_color: u32,
}

impl Checkbox {
    pub const fn new(id: WidgetId, rect: Rect, label: &'static str) -> Self {
        Self {
            id,
            rect,
            label,
            checked: false,
            background: COLOR_BG_DARK,
            text_color: COLOR_TEXT,
        }
    }

    pub fn draw(&self, focused: bool) {
        let _ = vga::draw_filled_rect(
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            self.background,
        );
        draw_rect_outline(self.rect, border_color(focused));

        let box_size = min(14, self.rect.height.saturating_sub(6)).max(8);
        let box_rect = Rect::new(self.rect.x + 4, self.rect.y + (self.rect.height - box_size) / 2, box_size, box_size);
        let _ = vga::draw_filled_rect(box_rect.x, box_rect.y, box_rect.width, box_rect.height, COLOR_BG_MID);
        draw_rect_outline(box_rect, COLOR_BORDER);
        if self.checked {
            let _ = vga::draw_line(
                box_rect.x + 2,
                box_rect.y + box_rect.height / 2,
                box_rect.x + box_rect.width / 2,
                box_rect.y + box_rect.height - 3,
                COLOR_BORDER_FOCUS,
            );
            let _ = vga::draw_line(
                box_rect.x + box_rect.width / 2,
                box_rect.y + box_rect.height - 3,
                box_rect.x + box_rect.width - 2,
                box_rect.y + 2,
                COLOR_BORDER_FOCUS,
            );
        }

        if let Some((_, font_h)) = vga::font_metrics() {
            let text_y = self
                .rect
                .y
                .saturating_add(((self.rect.height - font_h as i32) / 2).max(1));
            let _ = vga::draw_text(
                box_rect.x + box_rect.width + 6,
                text_y,
                self.label,
                self.text_color,
                self.background,
            );
        }
    }

    pub fn handle_event(&mut self, event: &InputEvent, focused: bool) -> WidgetResponse {
        let toggled = match *event {
            InputEvent::MouseClick {
                button: MouseButton::Left,
                x,
                y,
            } => self.rect.contains(x, y),
            InputEvent::KeyPress {
                key: KeyEvent::Char(' '),
            } if focused => true,
            _ => false,
        };

        if toggled {
            self.checked = !self.checked;
            return WidgetResponse {
                redraw: true,
                clicked: true,
            };
        }

        WidgetResponse::none()
    }
}

pub struct RadioButton {
    pub id: WidgetId,
    pub group: u16,
    pub rect: Rect,
    pub label: &'static str,
    pub selected: bool,
    pub background: u32,
    pub text_color: u32,
}

impl RadioButton {
    pub const fn new(id: WidgetId, group: u16, rect: Rect, label: &'static str) -> Self {
        Self {
            id,
            group,
            rect,
            label,
            selected: false,
            background: COLOR_BG_DARK,
            text_color: COLOR_TEXT,
        }
    }

    pub fn draw(&self, focused: bool) {
        let _ = vga::draw_filled_rect(
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            self.background,
        );
        draw_rect_outline(self.rect, border_color(focused));

        let radius = min(7, self.rect.height.saturating_sub(8) / 2).max(4);
        let cx = self.rect.x + 6 + radius;
        let cy = self.rect.y + self.rect.height / 2;
        let _ = vga::draw_circle(cx, cy, radius, COLOR_BORDER);
        if self.selected {
            let _ = vga::draw_filled_rect(cx - 2, cy - 2, 5, 5, COLOR_BORDER_FOCUS);
        }

        if let Some((_, font_h)) = vga::font_metrics() {
            let text_y = self
                .rect
                .y
                .saturating_add(((self.rect.height - font_h as i32) / 2).max(1));
            let _ = vga::draw_text(
                cx + radius + 6,
                text_y,
                self.label,
                self.text_color,
                self.background,
            );
        }
    }

    pub fn handle_event(&mut self, event: &InputEvent, focused: bool) -> WidgetResponse {
        let activate = match *event {
            InputEvent::MouseClick {
                button: MouseButton::Left,
                x,
                y,
            } => self.rect.contains(x, y),
            InputEvent::KeyPress {
                key: KeyEvent::Char(' '),
            } if focused => true,
            _ => false,
        };

        if activate {
            let changed = !self.selected;
            self.selected = true;
            return WidgetResponse {
                redraw: changed,
                clicked: true,
            };
        }

        WidgetResponse::none()
    }
}

pub struct Dropdown {
    pub id: WidgetId,
    pub rect: Rect,
    pub items: Vec<&'static str>,
    pub selected: usize,
    pub expanded: bool,
    hovered_item: Option<usize>,
}

impl Dropdown {
    pub fn new(id: WidgetId, rect: Rect, items: Vec<&'static str>) -> Self {
        Self {
            id,
            rect,
            items,
            selected: 0,
            expanded: false,
            hovered_item: None,
        }
    }

    pub fn hit_rect(&self) -> Rect {
        if !self.expanded {
            return self.rect;
        }

        union_rect(self.rect, self.popup_rect())
    }

    pub fn draw(&self, focused: bool) {
        let _ = vga::draw_filled_rect(
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            COLOR_BG_DARK,
        );
        draw_rect_outline(self.rect, border_color(focused));

        let label = if self.items.is_empty() {
            "<empty>"
        } else {
            self.items[self.selected.min(self.items.len() - 1)]
        };
        if let Some((_, font_h)) = vga::font_metrics() {
            let text_y = self
                .rect
                .y
                .saturating_add(((self.rect.height - font_h as i32) / 2).max(1));
            let _ = vga::draw_text(self.rect.x + 5, text_y, label, COLOR_TEXT, COLOR_BG_DARK);
            let _ = vga::draw_text(
                self.rect.x + self.rect.width - 12,
                text_y,
                if self.expanded { "^" } else { "v" },
                COLOR_ACCENT,
                COLOR_BG_DARK,
            );
        }

        if !self.expanded {
            return;
        }

        let row_h = self.row_height() as i32;
        let popup = self.popup_rect();
        let _ = vga::draw_filled_rect(popup.x, popup.y, popup.width, popup.height, COLOR_BG_MID);
        draw_rect_outline(popup, COLOR_BORDER);

        for (idx, item) in self.items.iter().enumerate() {
            let row_y = popup.y + idx as i32 * row_h;
            if self.hovered_item == Some(idx) || self.selected == idx {
                let _ = vga::draw_filled_rect(
                    popup.x + 1,
                    row_y + 1,
                    popup.width - 2,
                    row_h - 1,
                    COLOR_BG_LIGHT,
                );
            }
            if let Some((_, font_h)) = vga::font_metrics() {
                let text_y = row_y + ((row_h - font_h as i32) / 2).max(1);
                let _ = vga::draw_text(popup.x + 5, text_y, item, COLOR_TEXT, COLOR_BG_MID);
            }
        }
    }

    pub fn handle_event(&mut self, event: &InputEvent, focused: bool) -> WidgetResponse {
        match *event {
            InputEvent::MouseMove { x, y, .. } => {
                if self.expanded {
                    let hovered = self.item_at(x, y);
                    if hovered != self.hovered_item {
                        self.hovered_item = hovered;
                        return WidgetResponse {
                            redraw: true,
                            clicked: false,
                        };
                    }
                }
            }
            InputEvent::MouseDown {
                button: MouseButton::Left,
                x,
                y,
            } => {
                if self.header_rect().contains(x, y) {
                    self.expanded = !self.expanded;
                    self.hovered_item = None;
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }

                if self.expanded {
                    if let Some(index) = self.item_at(x, y) {
                        let changed = self.selected != index;
                        self.selected = index;
                        self.expanded = false;
                        return WidgetResponse {
                            redraw: true,
                            clicked: changed,
                        };
                    }

                    self.expanded = false;
                    self.hovered_item = None;
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }
            }
            InputEvent::KeyPress { key } if focused => match key {
                KeyEvent::Up => {
                    if self.selected > 0 {
                        self.selected -= 1;
                        return WidgetResponse {
                            redraw: true,
                            clicked: true,
                        };
                    }
                }
                KeyEvent::Down => {
                    if self.selected + 1 < self.items.len() {
                        self.selected += 1;
                        return WidgetResponse {
                            redraw: true,
                            clicked: true,
                        };
                    }
                }
                KeyEvent::Char('\n') | KeyEvent::Char(' ') => {
                    self.expanded = !self.expanded;
                    self.hovered_item = None;
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }
                _ => {}
            },
            _ => {}
        }

        WidgetResponse::none()
    }

    fn row_height(&self) -> usize {
        vga::font_metrics().map_or(18, |(_, h)| h + 6)
    }

    fn header_rect(&self) -> Rect {
        self.rect
    }

    fn popup_rect(&self) -> Rect {
        let row_h = self.row_height() as i32;
        let popup_h = row_h.saturating_mul(self.items.len() as i32);
        Rect::new(
            self.rect.x,
            popup_y_for_anchor(self.rect, popup_h),
            self.rect.width,
            popup_h,
        )
    }

    fn item_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.expanded || self.items.is_empty() {
            return None;
        }
        let row_h = self.row_height() as i32;
        let popup_rect = self.popup_rect();
        if !popup_rect.contains(x, y) {
            return None;
        }

        let row = (y - popup_rect.y) / row_h;
        Some(row as usize)
    }
}

pub struct ComboBox {
    pub id: WidgetId,
    pub rect: Rect,
    pub items: Vec<&'static str>,
    pub selected: Option<usize>,
    pub expanded: bool,
    input: Vec<u8>,
    cursor: usize,
    hovered_item: Option<usize>,
}

impl ComboBox {
    pub fn new(id: WidgetId, rect: Rect, items: Vec<&'static str>) -> Self {
        Self {
            id,
            rect,
            items,
            selected: None,
            expanded: false,
            input: Vec::new(),
            cursor: 0,
            hovered_item: None,
        }
    }

    pub fn hit_rect(&self) -> Rect {
        if !self.expanded {
            return self.rect;
        }

        union_rect(self.rect, self.popup_rect())
    }

    pub fn draw(&self, focused: bool) {
        let _ = vga::draw_filled_rect(
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            COLOR_BG_DARK,
        );
        draw_rect_outline(self.rect, border_color(focused));

        if let Some((font_w, font_h)) = vga::font_metrics() {
            let text_y = self
                .rect
                .y
                .saturating_add(((self.rect.height - font_h as i32) / 2).max(1));
            if !self.input.is_empty() {
                draw_text_line(
                    self.rect.x + 4,
                    text_y,
                    &self.input,
                    COLOR_TEXT,
                    COLOR_BG_DARK,
                );
            } else if let Some(index) = self.selected {
                if let Some(item) = self.items.get(index) {
                    let _ = vga::draw_text(self.rect.x + 4, text_y, item, COLOR_TEXT, COLOR_BG_DARK);
                }
            } else {
                let _ = vga::draw_text(
                    self.rect.x + 4,
                    text_y,
                    "type to filter...",
                    COLOR_TEXT_MUTED,
                    COLOR_BG_DARK,
                );
            }

            let _ = vga::draw_text(
                self.rect.x + self.rect.width - 12,
                text_y,
                if self.expanded { "^" } else { "v" },
                COLOR_ACCENT,
                COLOR_BG_DARK,
            );

            if focused {
                let cursor_x = self.rect.x + 4 + min(self.cursor, self.input.len()) as i32 * font_w as i32;
                let _ = vga::draw_filled_rect(cursor_x, text_y, 1, font_h as i32, COLOR_ACCENT);
            }
        }

        if !self.expanded {
            return;
        }

        let row_h = self.row_height() as i32;
        let popup = self.popup_rect();
        let _ = vga::draw_filled_rect(popup.x, popup.y, popup.width, popup.height, COLOR_BG_MID);
        draw_rect_outline(popup, COLOR_BORDER);
        for (idx, item) in self.items.iter().enumerate() {
            let row_y = popup.y + idx as i32 * row_h;
            if self.hovered_item == Some(idx) || self.selected == Some(idx) {
                let _ = vga::draw_filled_rect(
                    popup.x + 1,
                    row_y + 1,
                    popup.width - 2,
                    row_h - 1,
                    COLOR_BG_LIGHT,
                );
            }
            if let Some((_, font_h)) = vga::font_metrics() {
                let text_y = row_y + ((row_h - font_h as i32) / 2).max(1);
                let _ = vga::draw_text(popup.x + 5, text_y, item, COLOR_TEXT, COLOR_BG_MID);
            }
        }
    }

    pub fn handle_event(&mut self, event: &InputEvent, focused: bool) -> WidgetResponse {
        match *event {
            InputEvent::MouseMove { x, y, .. } => {
                if self.expanded {
                    let hovered = self.item_at(x, y);
                    if hovered != self.hovered_item {
                        self.hovered_item = hovered;
                        return WidgetResponse {
                            redraw: true,
                            clicked: false,
                        };
                    }
                }
            }
            InputEvent::MouseDown {
                button: MouseButton::Left,
                x,
                y,
            } => {
                if self.rect.contains(x, y) {
                    self.expanded = !self.expanded;
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }

                if self.expanded {
                    if let Some(index) = self.item_at(x, y) {
                        self.selected = Some(index);
                        self.input.clear();
                        self.input.extend_from_slice(self.items[index].as_bytes());
                        self.cursor = self.input.len();
                        self.expanded = false;
                        self.hovered_item = None;
                        return WidgetResponse {
                            redraw: true,
                            clicked: true,
                        };
                    }

                    self.expanded = false;
                    self.hovered_item = None;
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }
            }
            InputEvent::KeyPress { key } if focused => {
                let changed = match key {
                    KeyEvent::Left => {
                        if self.cursor > 0 {
                            self.cursor -= 1;
                            true
                        } else {
                            false
                        }
                    }
                    KeyEvent::Right => {
                        if self.cursor < self.input.len() {
                            self.cursor += 1;
                            true
                        } else {
                            false
                        }
                    }
                    KeyEvent::Char('\x08') => {
                        if self.cursor > 0 && !self.input.is_empty() {
                            self.input.remove(self.cursor - 1);
                            self.cursor -= 1;
                            true
                        } else {
                            false
                        }
                    }
                    KeyEvent::Char('\n') => {
                        self.expanded = !self.expanded;
                        true
                    }
                    KeyEvent::Down => {
                        if self.items.is_empty() {
                            false
                        } else {
                            let next = self.selected.unwrap_or(0).min(self.items.len() - 1);
                            let next = min(next + 1, self.items.len() - 1);
                            self.selected = Some(next);
                            self.input.clear();
                            self.input.extend_from_slice(self.items[next].as_bytes());
                            self.cursor = self.input.len();
                            true
                        }
                    }
                    KeyEvent::Up => {
                        if self.items.is_empty() {
                            false
                        } else {
                            let current = self.selected.unwrap_or(0).min(self.items.len() - 1);
                            let next = current.saturating_sub(1);
                            self.selected = Some(next);
                            self.input.clear();
                            self.input.extend_from_slice(self.items[next].as_bytes());
                            self.cursor = self.input.len();
                            true
                        }
                    }
                    KeyEvent::Char(ch) if is_printable_ascii(ch) => {
                        self.input.insert(self.cursor, ch as u8);
                        self.cursor += 1;
                        self.select_matching_item();
                        true
                    }
                    _ => false,
                };

                if changed {
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }
            }
            _ => {}
        }

        WidgetResponse::none()
    }

    fn row_height(&self) -> usize {
        vga::font_metrics().map_or(18, |(_, h)| h + 6)
    }

    fn item_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.expanded || self.items.is_empty() {
            return None;
        }
        let row_h = self.row_height() as i32;
        let popup_rect = self.popup_rect();
        if !popup_rect.contains(x, y) {
            return None;
        }

        let row = (y - popup_rect.y) / row_h;
        Some(row as usize)
    }

    fn select_matching_item(&mut self) {
        if self.input.is_empty() {
            self.selected = None;
            return;
        }
        for (index, item) in self.items.iter().enumerate() {
            if item.as_bytes().starts_with(&self.input) {
                self.selected = Some(index);
                return;
            }
        }
    }

    fn popup_rect(&self) -> Rect {
        let row_h = self.row_height() as i32;
        let popup_h = row_h.saturating_mul(self.items.len() as i32);
        Rect::new(
            self.rect.x,
            popup_y_for_anchor(self.rect, popup_h),
            self.rect.width,
            popup_h,
        )
    }
}

pub struct Scrollbar {
    pub id: WidgetId,
    pub rect: Rect,
    pub orientation: Orientation,
    pub value: i32,
    pub max: i32,
    pub page: i32,
    dragging: bool,
    drag_origin_pos: i32,
    drag_origin_value: i32,
}

impl Scrollbar {
    pub const fn new(id: WidgetId, rect: Rect, orientation: Orientation) -> Self {
        Self {
            id,
            rect,
            orientation,
            value: 0,
            max: 0,
            page: 10,
            dragging: false,
            drag_origin_pos: 0,
            drag_origin_value: 0,
        }
    }

    pub fn draw(&self, focused: bool) {
        let _ = vga::draw_filled_rect(
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            COLOR_BG_MID,
        );
        draw_rect_outline(self.rect, border_color(focused));

        let thumb = self.thumb_rect();
        let _ = vga::draw_filled_rect(thumb.x, thumb.y, thumb.width, thumb.height, COLOR_BG_LIGHT);
        draw_rect_outline(thumb, COLOR_ACCENT);
    }

    pub fn handle_event(&mut self, event: &InputEvent, focused: bool) -> WidgetResponse {
        match *event {
            InputEvent::MouseDown {
                button: MouseButton::Left,
                x,
                y,
            } => {
                if !self.rect.contains(x, y) {
                    return WidgetResponse::none();
                }

                let thumb = self.thumb_rect();
                if thumb.contains(x, y) {
                    self.dragging = true;
                    self.drag_origin_pos = self.axis_position(x, y);
                    self.drag_origin_value = self.value;
                    return WidgetResponse {
                        redraw: false,
                        clicked: false,
                    };
                }

                let pointer = self.axis_position(x, y);
                let thumb_pos = self.axis_position(thumb.x, thumb.y);
                let next = if pointer < thumb_pos {
                    self.value.saturating_sub(self.page.max(1))
                } else {
                    self.value.saturating_add(self.page.max(1))
                };
                let changed = self.set_value(next);
                if changed {
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }
            }
            InputEvent::MouseMove { x, y, .. } => {
                if self.dragging {
                    let pointer = self.axis_position(x, y);
                    let track = self.track_len().max(1);
                    let thumb_len = self.thumb_len().max(1);
                    let movable = max(1, track - thumb_len);
                    let delta = pointer - self.drag_origin_pos;
                    let next = self.drag_origin_value
                        + (delta.saturating_mul(self.max.max(1))) / movable;
                    if self.set_value(next) {
                        return WidgetResponse {
                            redraw: true,
                            clicked: false,
                        };
                    }
                }
            }
            InputEvent::MouseUp {
                button: MouseButton::Left,
                ..
            } => {
                if self.dragging {
                    self.dragging = false;
                    return WidgetResponse {
                        redraw: false,
                        clicked: false,
                    };
                }
            }
            InputEvent::KeyPress { key } if focused => {
                let step = max(1, self.page / 4);
                let next = match (self.orientation, key) {
                    (Orientation::Horizontal, KeyEvent::Left) => Some(self.value - step),
                    (Orientation::Horizontal, KeyEvent::Right) => Some(self.value + step),
                    (Orientation::Vertical, KeyEvent::Up) => Some(self.value - step),
                    (Orientation::Vertical, KeyEvent::Down) => Some(self.value + step),
                    (_, KeyEvent::PageUp) => Some(self.value - self.page.max(1)),
                    (_, KeyEvent::PageDown) => Some(self.value + self.page.max(1)),
                    _ => None,
                };
                if let Some(next) = next {
                    if self.set_value(next) {
                        return WidgetResponse {
                            redraw: true,
                            clicked: false,
                        };
                    }
                }
            }
            _ => {}
        }

        WidgetResponse::none()
    }

    fn set_value(&mut self, next: i32) -> bool {
        let clamped = next.clamp(0, self.max.max(0));
        if clamped == self.value {
            return false;
        }
        self.value = clamped;
        true
    }

    fn track_len(&self) -> i32 {
        match self.orientation {
            Orientation::Horizontal => self.rect.width.saturating_sub(2),
            Orientation::Vertical => self.rect.height.saturating_sub(2),
        }
    }

    fn thumb_len(&self) -> i32 {
        let track = self.track_len().max(1);
        if self.max <= 0 {
            return track;
        }

        let page = self.page.max(1);
        let len = (track.saturating_mul(page)) / (self.max + page);
        len.clamp(8, track)
    }

    fn thumb_rect(&self) -> Rect {
        let track = self.track_len().max(1);
        let thumb_len = self.thumb_len().max(1);
        let movable = max(1, track - thumb_len);
        let pos = if self.max <= 0 {
            0
        } else {
            (self.value.saturating_mul(movable)) / self.max.max(1)
        };

        match self.orientation {
            Orientation::Horizontal => Rect::new(
                self.rect.x + 1 + pos,
                self.rect.y + 1,
                thumb_len,
                self.rect.height - 2,
            ),
            Orientation::Vertical => Rect::new(
                self.rect.x + 1,
                self.rect.y + 1 + pos,
                self.rect.width - 2,
                thumb_len,
            ),
        }
    }

    fn axis_position(&self, x: i32, y: i32) -> i32 {
        match self.orientation {
            Orientation::Horizontal => x - self.rect.x - 1,
            Orientation::Vertical => y - self.rect.y - 1,
        }
    }
}

pub struct ListView {
    pub id: WidgetId,
    pub rect: Rect,
    pub items: Vec<&'static str>,
    pub selected: Option<usize>,
    pub scroll: usize,
}

impl ListView {
    pub fn new(id: WidgetId, rect: Rect, items: Vec<&'static str>) -> Self {
        Self {
            id,
            rect,
            items,
            selected: None,
            scroll: 0,
        }
    }

    pub fn draw(&self, focused: bool) {
        let _ = vga::draw_filled_rect(
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            COLOR_BG_DARK,
        );
        draw_rect_outline(self.rect, border_color(focused));

        let row_h = self.row_height() as i32;
        let rows = self.visible_rows();
        for row in 0..rows {
            let item_idx = self.scroll + row;
            if item_idx >= self.items.len() {
                break;
            }
            let y = self.rect.y + 2 + row as i32 * row_h;
            if self.selected == Some(item_idx) {
                let _ = vga::draw_filled_rect(
                    self.rect.x + 1,
                    y,
                    self.rect.width - 2,
                    row_h,
                    COLOR_BG_LIGHT,
                );
            }
            if let Some((_, font_h)) = vga::font_metrics() {
                let text_y = y + ((row_h - font_h as i32) / 2).max(1);
                let _ = vga::draw_text(
                    self.rect.x + 4,
                    text_y,
                    self.items[item_idx],
                    COLOR_TEXT,
                    COLOR_BG_DARK,
                );
            }
        }
    }

    pub fn handle_event(&mut self, event: &InputEvent, focused: bool) -> WidgetResponse {
        match *event {
            InputEvent::MouseClick {
                button: MouseButton::Left,
                x,
                y,
            } => {
                if self.rect.contains(x, y) {
                    if let Some(index) = self.row_at(y) {
                        if index < self.items.len() {
                            let changed = self.selected != Some(index);
                            self.selected = Some(index);
                            self.ensure_visible_selected();
                            return WidgetResponse {
                                redraw: true,
                                clicked: changed,
                            };
                        }
                    }
                }
            }
            InputEvent::KeyPress { key } if focused => {
                let changed = match key {
                    KeyEvent::Up => self.move_selection(-1),
                    KeyEvent::Down => self.move_selection(1),
                    KeyEvent::PageUp => self.move_selection(-(self.visible_rows() as i32)),
                    KeyEvent::PageDown => self.move_selection(self.visible_rows() as i32),
                    _ => false,
                };
                if changed {
                    return WidgetResponse {
                        redraw: true,
                        clicked: true,
                    };
                }
            }
            _ => {}
        }
        WidgetResponse::none()
    }

    fn row_height(&self) -> usize {
        vga::font_metrics().map_or(18, |(_, h)| h + 4)
    }

    fn visible_rows(&self) -> usize {
        let row_h = self.row_height().max(1) as i32;
        max(1, ((self.rect.height - 4).max(1) / row_h) as usize)
    }

    fn row_at(&self, y: i32) -> Option<usize> {
        let row_h = self.row_height() as i32;
        if row_h <= 0 {
            return None;
        }
        let relative = y - (self.rect.y + 2);
        if relative < 0 {
            return None;
        }
        Some(self.scroll + (relative / row_h) as usize)
    }

    fn move_selection(&mut self, delta: i32) -> bool {
        if self.items.is_empty() {
            return false;
        }

        let current = self.selected.unwrap_or(0).min(self.items.len() - 1) as i32;
        let next = (current + delta).clamp(0, (self.items.len() - 1) as i32) as usize;
        let changed = self.selected != Some(next);
        self.selected = Some(next);
        self.ensure_visible_selected();
        changed
    }

    fn ensure_visible_selected(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let rows = self.visible_rows();
        if selected < self.scroll {
            self.scroll = selected;
        } else if selected >= self.scroll.saturating_add(rows) {
            self.scroll = selected.saturating_sub(rows.saturating_sub(1));
        }
    }
}

pub struct TreeNode {
    pub label: &'static str,
    pub depth: u8,
    pub has_children: bool,
    pub expanded: bool,
}

impl TreeNode {
    pub const fn new(label: &'static str, depth: u8, has_children: bool, expanded: bool) -> Self {
        Self {
            label,
            depth,
            has_children,
            expanded,
        }
    }
}

pub struct TreeView {
    pub id: WidgetId,
    pub rect: Rect,
    pub nodes: Vec<TreeNode>,
    pub selected: Option<usize>,
    pub scroll: usize,
}

impl TreeView {
    pub fn new(id: WidgetId, rect: Rect, nodes: Vec<TreeNode>) -> Self {
        Self {
            id,
            rect,
            nodes,
            selected: None,
            scroll: 0,
        }
    }

    pub fn draw(&self, focused: bool) {
        let _ = vga::draw_filled_rect(
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            COLOR_BG_DARK,
        );
        draw_rect_outline(self.rect, border_color(focused));

        let visible = self.visible_nodes();
        let row_h = self.row_height() as i32;
        let rows = self.visible_rows();
        for row in 0..rows {
            let visible_idx = self.scroll + row;
            if visible_idx >= visible.len() {
                break;
            }
            let node_idx = visible[visible_idx];
            let node = &self.nodes[node_idx];
            let y = self.rect.y + 2 + row as i32 * row_h;

            if self.selected == Some(node_idx) {
                let _ = vga::draw_filled_rect(
                    self.rect.x + 1,
                    y,
                    self.rect.width - 2,
                    row_h,
                    COLOR_BG_LIGHT,
                );
            }

            let indent = node.depth as i32 * 12;
            let marker_x = self.rect.x + 4 + indent;
            if node.has_children {
                let _ = vga::draw_filled_rect(marker_x, y + 3, 9, 9, COLOR_BG_MID);
                draw_rect_outline(Rect::new(marker_x, y + 3, 9, 9), COLOR_BORDER);
                let symbol = if node.expanded { "-" } else { "+" };
                let _ = vga::draw_text(marker_x + 2, y + 2, symbol, COLOR_ACCENT, COLOR_BG_MID);
            }
            let text_x = marker_x + if node.has_children { 14 } else { 2 };
            if let Some((_, font_h)) = vga::font_metrics() {
                let text_y = y + ((row_h - font_h as i32) / 2).max(1);
                let _ = vga::draw_text(text_x, text_y, node.label, COLOR_TEXT, COLOR_BG_DARK);
            }
        }
    }

    pub fn handle_event(&mut self, event: &InputEvent, focused: bool) -> WidgetResponse {
        match *event {
            InputEvent::MouseClick {
                button: MouseButton::Left,
                x,
                y,
            } => {
                if self.rect.contains(x, y) {
                    if let Some(node_idx) = self.node_at(y) {
                        let mut clicked = self.selected != Some(node_idx);
                        self.selected = Some(node_idx);
                        if self.toggle_if_marker_hit(node_idx, x, y) {
                            clicked = true;
                        }
                        self.ensure_visible_selected();
                        return WidgetResponse {
                            redraw: true,
                            clicked,
                        };
                    }
                }
            }
            InputEvent::KeyPress { key } if focused => {
                let mut changed = false;
                match key {
                    KeyEvent::Up => {
                        changed = self.move_selection(-1);
                    }
                    KeyEvent::Down => {
                        changed = self.move_selection(1);
                    }
                    KeyEvent::Left => {
                        changed = self.collapse_selected();
                    }
                    KeyEvent::Right => {
                        changed = self.expand_selected();
                    }
                    KeyEvent::PageUp => {
                        changed = self.move_selection(-(self.visible_rows() as i32));
                    }
                    KeyEvent::PageDown => {
                        changed = self.move_selection(self.visible_rows() as i32);
                    }
                    _ => {}
                }

                if changed {
                    return WidgetResponse {
                        redraw: true,
                        clicked: true,
                    };
                }
            }
            _ => {}
        }

        WidgetResponse::none()
    }

    fn row_height(&self) -> usize {
        vga::font_metrics().map_or(18, |(_, h)| h + 4)
    }

    fn visible_rows(&self) -> usize {
        let row_h = self.row_height().max(1) as i32;
        max(1, ((self.rect.height - 4).max(1) / row_h) as usize)
    }

    fn visible_nodes(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut hidden_depth: Option<u8> = None;
        for (index, node) in self.nodes.iter().enumerate() {
            if let Some(depth) = hidden_depth {
                if node.depth > depth {
                    continue;
                }
                hidden_depth = None;
            }

            out.push(index);
            if node.has_children && !node.expanded {
                hidden_depth = Some(node.depth);
            }
        }
        out
    }

    fn node_at(&self, y: i32) -> Option<usize> {
        let row_h = self.row_height() as i32;
        if row_h <= 0 {
            return None;
        }
        let relative = y - (self.rect.y + 2);
        if relative < 0 {
            return None;
        }
        let row = (relative / row_h) as usize;
        let visible = self.visible_nodes();
        visible.get(self.scroll + row).copied()
    }

    fn toggle_if_marker_hit(&mut self, node_idx: usize, x: i32, y: i32) -> bool {
        let visible = self.visible_nodes();
        let Some(visible_row) = visible.iter().position(|value| *value == node_idx) else {
            return false;
        };
        let row = visible_row.saturating_sub(self.scroll);
        if row >= self.visible_rows() {
            return false;
        }
        let row_h = self.row_height() as i32;
        let node = &mut self.nodes[node_idx];
        if !node.has_children {
            return false;
        }
        let marker_x = self.rect.x + 4 + node.depth as i32 * 12;
        let marker_y = self.rect.y + 2 + row as i32 * row_h + 3;
        let marker = Rect::new(marker_x, marker_y, 9, 9);
        if marker.contains(x, y) {
            node.expanded = !node.expanded;
            return true;
        }
        false
    }

    fn move_selection(&mut self, delta: i32) -> bool {
        let visible = self.visible_nodes();
        if visible.is_empty() {
            return false;
        }
        let current_pos = self
            .selected
            .and_then(|value| visible.iter().position(|idx| *idx == value))
            .unwrap_or(0) as i32;
        let next_pos = (current_pos + delta).clamp(0, (visible.len() - 1) as i32) as usize;
        let next = visible[next_pos];
        let changed = self.selected != Some(next);
        self.selected = Some(next);
        self.ensure_visible_selected();
        changed
    }

    fn collapse_selected(&mut self) -> bool {
        let Some(index) = self.selected else {
            return false;
        };
        if let Some(node) = self.nodes.get_mut(index) {
            if node.has_children && node.expanded {
                node.expanded = false;
                return true;
            }
        }
        false
    }

    fn expand_selected(&mut self) -> bool {
        let Some(index) = self.selected else {
            return false;
        };
        if let Some(node) = self.nodes.get_mut(index) {
            if node.has_children && !node.expanded {
                node.expanded = true;
                return true;
            }
        }
        false
    }

    fn ensure_visible_selected(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let visible = self.visible_nodes();
        let Some(position) = visible.iter().position(|idx| *idx == selected) else {
            return;
        };
        let rows = self.visible_rows();
        if position < self.scroll {
            self.scroll = position;
        } else if position >= self.scroll.saturating_add(rows) {
            self.scroll = position.saturating_sub(rows.saturating_sub(1));
        }
    }
}

pub struct ProgressBar {
    pub id: WidgetId,
    pub rect: Rect,
    pub value: u32,
    pub max: u32,
    pub foreground: u32,
    pub background: u32,
    pub show_text: bool,
}

impl ProgressBar {
    pub const fn new(id: WidgetId, rect: Rect) -> Self {
        Self {
            id,
            rect,
            value: 0,
            max: 100,
            foreground: COLOR_ACCENT,
            background: COLOR_BG_MID,
            show_text: true,
        }
    }

    pub fn draw(&self, focused: bool) {
        let _ = vga::draw_filled_rect(
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            self.background,
        );
        draw_rect_outline(self.rect, border_color(focused));

        let max_value = self.max.max(1);
        let clamped = min(self.value, max_value);
        let inner_w = self.rect.width.saturating_sub(2);
        let fill_w = ((inner_w as u64 * clamped as u64) / max_value as u64) as i32;
        let _ = vga::draw_filled_rect(
            self.rect.x + 1,
            self.rect.y + 1,
            fill_w,
            self.rect.height.saturating_sub(2),
            self.foreground,
        );

        if self.show_text {
            let percent = (clamped as u64 * 100) / max_value as u64;
            let mut text = [0u8; 16];
            let len = write_percent(percent as u32, &mut text);
            if let Some((font_w, font_h)) = vga::font_metrics() {
                let text_x = self
                    .rect
                    .x
                    .saturating_add(((self.rect.width - (len as i32 * font_w as i32)) / 2).max(1));
                let text_y = self
                    .rect
                    .y
                    .saturating_add(((self.rect.height - font_h as i32) / 2).max(1));
                draw_text_line(text_x, text_y, &text[..len], COLOR_TEXT, self.background);
            }
        }
    }

    pub fn handle_event(&mut self, _event: &InputEvent, _focused: bool) -> WidgetResponse {
        WidgetResponse::none()
    }
}

pub struct PopupMenu {
    pub id: WidgetId,
    pub rect: Rect,
    pub items: Vec<&'static str>,
    pub visible: bool,
    pub selected: Option<usize>,
    hovered: Option<usize>,
}

impl PopupMenu {
    pub fn new(id: WidgetId, rect: Rect, items: Vec<&'static str>) -> Self {
        Self {
            id,
            rect,
            items,
            visible: false,
            selected: None,
            hovered: None,
        }
    }

    pub fn show_at(&mut self, x: i32, y: i32) {
        self.visible = true;
        let popup_h = self.item_height() as i32 * self.items.len() as i32;
        let mut popup_x = x;
        if let Some((fb_w, _)) = vga::framebuffer_resolution() {
            let max_x = (fb_w as i32).saturating_sub(self.rect.width).max(0);
            popup_x = popup_x.clamp(0, max_x);
        }

        let anchor = Rect::new(popup_x, y, self.rect.width, 0);
        self.rect.x = popup_x;
        self.rect.y = popup_y_for_anchor(anchor, popup_h);
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.hovered = None;
    }

    pub fn hit_rect(&self) -> Rect {
        if !self.visible {
            return Rect::new(0, 0, 0, 0);
        }
        Rect::new(
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.item_height() as i32 * self.items.len() as i32,
        )
    }

    pub fn draw(&self, _focused: bool) {
        if !self.visible {
            return;
        }
        let hit = self.hit_rect();
        let _ = vga::draw_filled_rect(hit.x, hit.y, hit.width, hit.height, COLOR_BG_MID);
        draw_rect_outline(hit, COLOR_BORDER);
        let row_h = self.item_height() as i32;
        for (index, item) in self.items.iter().enumerate() {
            let y = hit.y + index as i32 * row_h;
            if self.hovered == Some(index) {
                let _ = vga::draw_filled_rect(hit.x + 1, y + 1, hit.width - 2, row_h - 1, COLOR_BG_LIGHT);
            }
            if let Some((_, font_h)) = vga::font_metrics() {
                let text_y = y + ((row_h - font_h as i32) / 2).max(1);
                let _ = vga::draw_text(hit.x + 4, text_y, item, COLOR_TEXT, COLOR_BG_MID);
            }
        }
    }

    pub fn handle_event(&mut self, event: &InputEvent, _focused: bool) -> WidgetResponse {
        if !self.visible {
            return WidgetResponse::none();
        }

        match *event {
            InputEvent::MouseMove { x, y, .. } => {
                let hovered = self.row_at(x, y);
                if hovered != self.hovered {
                    self.hovered = hovered;
                    return WidgetResponse {
                        redraw: true,
                        clicked: false,
                    };
                }
            }
            InputEvent::MouseDown {
                button: MouseButton::Left,
                x,
                y,
            } => {
                if let Some(index) = self.row_at(x, y) {
                    self.selected = Some(index);
                    self.visible = false;
                    self.hovered = None;
                    return WidgetResponse {
                        redraw: true,
                        clicked: true,
                    };
                }

                self.visible = false;
                self.hovered = None;
                return WidgetResponse {
                    redraw: true,
                    clicked: false,
                };
            }
            _ => {}
        }

        WidgetResponse::none()
    }

    fn item_height(&self) -> usize {
        vga::font_metrics().map_or(18, |(_, h)| h + 6)
    }

    fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        let hit = self.hit_rect();
        if !hit.contains(x, y) {
            return None;
        }
        let row_h = self.item_height() as i32;
        let row = (y - hit.y) / row_h;
        Some(row as usize)
    }
}

pub type ContextMenu = PopupMenu;

fn write_percent(mut value: u32, out: &mut [u8; 16]) -> usize {
    let mut digits = [0u8; 10];
    let mut count = 0usize;
    if value == 0 {
        digits[0] = b'0';
        count = 1;
    } else {
        while value > 0 && count < digits.len() {
            digits[count] = b'0' + (value % 10) as u8;
            value /= 10;
            count += 1;
        }
    }

    let mut len = 0usize;
    for index in (0..count).rev() {
        out[len] = digits[index];
        len += 1;
    }
    out[len] = b'%';
    len + 1
}

pub enum Widget {
    Panel(Panel),
    Label(Label),
    Button(Button),
    TextBox(TextBox),
    TextArea(TextArea),
    Checkbox(Checkbox),
    RadioButton(RadioButton),
    Dropdown(Dropdown),
    ComboBox(ComboBox),
    Scrollbar(Scrollbar),
    ListView(ListView),
    TreeView(TreeView),
    ProgressBar(ProgressBar),
    PopupMenu(PopupMenu),
}

impl Widget {
    pub fn id(&self) -> WidgetId {
        match self {
            Widget::Panel(panel) => panel.id,
            Widget::Label(label) => label.id,
            Widget::Button(button) => button.id,
            Widget::TextBox(text_box) => text_box.id,
            Widget::TextArea(text_area) => text_area.id,
            Widget::Checkbox(checkbox) => checkbox.id,
            Widget::RadioButton(radio) => radio.id,
            Widget::Dropdown(dropdown) => dropdown.id,
            Widget::ComboBox(combo_box) => combo_box.id,
            Widget::Scrollbar(scrollbar) => scrollbar.id,
            Widget::ListView(list_view) => list_view.id,
            Widget::TreeView(tree_view) => tree_view.id,
            Widget::ProgressBar(progress) => progress.id,
            Widget::PopupMenu(menu) => menu.id,
        }
    }

    pub fn rect(&self) -> Rect {
        match self {
            Widget::Panel(panel) => panel.rect,
            Widget::Label(label) => label.rect,
            Widget::Button(button) => button.rect,
            Widget::TextBox(text_box) => text_box.rect,
            Widget::TextArea(text_area) => text_area.rect,
            Widget::Checkbox(checkbox) => checkbox.rect,
            Widget::RadioButton(radio) => radio.rect,
            Widget::Dropdown(dropdown) => dropdown.rect,
            Widget::ComboBox(combo_box) => combo_box.rect,
            Widget::Scrollbar(scrollbar) => scrollbar.rect,
            Widget::ListView(list_view) => list_view.rect,
            Widget::TreeView(tree_view) => tree_view.rect,
            Widget::ProgressBar(progress) => progress.rect,
            Widget::PopupMenu(menu) => menu.rect,
        }
    }

    pub fn hit_rect(&self) -> Rect {
        match self {
            Widget::Dropdown(dropdown) => dropdown.hit_rect(),
            Widget::ComboBox(combo_box) => combo_box.hit_rect(),
            Widget::PopupMenu(menu) => menu.hit_rect(),
            _ => self.rect(),
        }
    }

    pub fn is_focusable(&self) -> bool {
        matches!(
            self,
            Widget::Button(_)
                | Widget::TextBox(_)
                | Widget::TextArea(_)
                | Widget::Checkbox(_)
                | Widget::RadioButton(_)
                | Widget::Dropdown(_)
                | Widget::ComboBox(_)
                | Widget::Scrollbar(_)
                | Widget::ListView(_)
                | Widget::TreeView(_)
        )
    }

    pub fn draw(&self, focused: bool) {
        match self {
            Widget::Panel(panel) => panel.draw(focused),
            Widget::Label(label) => label.draw(focused),
            Widget::Button(button) => button.draw(focused),
            Widget::TextBox(text_box) => text_box.draw(focused),
            Widget::TextArea(text_area) => text_area.draw(focused),
            Widget::Checkbox(checkbox) => checkbox.draw(focused),
            Widget::RadioButton(radio) => radio.draw(focused),
            Widget::Dropdown(dropdown) => dropdown.draw(focused),
            Widget::ComboBox(combo_box) => combo_box.draw(focused),
            Widget::Scrollbar(scrollbar) => scrollbar.draw(focused),
            Widget::ListView(list_view) => list_view.draw(focused),
            Widget::TreeView(tree_view) => tree_view.draw(focused),
            Widget::ProgressBar(progress) => progress.draw(focused),
            Widget::PopupMenu(menu) => menu.draw(focused),
        }
    }

    pub fn handle_event(&mut self, event: &InputEvent, focused: bool) -> WidgetResponse {
        match self {
            Widget::Panel(panel) => panel.handle_event(event, focused),
            Widget::Label(label) => label.handle_event(event, focused),
            Widget::Button(button) => button.handle_event(event, focused),
            Widget::TextBox(text_box) => text_box.handle_event(event, focused),
            Widget::TextArea(text_area) => text_area.handle_event(event, focused),
            Widget::Checkbox(checkbox) => checkbox.handle_event(event, focused),
            Widget::RadioButton(radio) => radio.handle_event(event, focused),
            Widget::Dropdown(dropdown) => dropdown.handle_event(event, focused),
            Widget::ComboBox(combo_box) => combo_box.handle_event(event, focused),
            Widget::Scrollbar(scrollbar) => scrollbar.handle_event(event, focused),
            Widget::ListView(list_view) => list_view.handle_event(event, focused),
            Widget::TreeView(tree_view) => tree_view.handle_event(event, focused),
            Widget::ProgressBar(progress) => progress.handle_event(event, focused),
            Widget::PopupMenu(menu) => menu.handle_event(event, focused),
        }
    }

    pub fn update_hover_state(&mut self, x: i32, y: i32) -> bool {
        match self {
            Widget::Button(button) => button.set_hovered(button.rect.contains(x, y)),
            _ => false,
        }
    }

    pub fn is_active_overlay(&self) -> bool {
        match self {
            Widget::Dropdown(dropdown) => dropdown.expanded,
            Widget::ComboBox(combo_box) => combo_box.expanded,
            Widget::PopupMenu(menu) => menu.visible,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DispatchBatch {
    pub processed: usize,
    pub redraw: bool,
    pub clicked: Option<WidgetId>,
    pub key_press: Option<KeyEvent>,
}

pub struct EventDispatcher {
    widgets: Vec<Widget>,
    hit_regions: [HitRegion; MAX_WIDGETS],
    hit_count: usize,
    focused_id: Option<WidgetId>,
    pointer_capture_id: Option<WidgetId>,
}

impl EventDispatcher {
    pub fn new() -> Self {
        Self {
            widgets: Vec::new(),
            hit_regions: [EMPTY_REGION; MAX_WIDGETS],
            hit_count: 0,
            focused_id: None,
            pointer_capture_id: None,
        }
    }

    pub fn add_panel(&mut self, panel: Panel) -> Result<(), &'static str> {
        self.push_widget(Widget::Panel(panel))
    }

    pub fn add_label(&mut self, label: Label) -> Result<(), &'static str> {
        self.push_widget(Widget::Label(label))
    }

    pub fn add_button(&mut self, button: Button) -> Result<(), &'static str> {
        self.push_widget(Widget::Button(button))
    }

    pub fn add_text_box(&mut self, text_box: TextBox) -> Result<(), &'static str> {
        self.push_widget(Widget::TextBox(text_box))
    }

    pub fn add_text_area(&mut self, text_area: TextArea) -> Result<(), &'static str> {
        self.push_widget(Widget::TextArea(text_area))
    }

    pub fn add_checkbox(&mut self, checkbox: Checkbox) -> Result<(), &'static str> {
        self.push_widget(Widget::Checkbox(checkbox))
    }

    pub fn add_radio_button(&mut self, radio_button: RadioButton) -> Result<(), &'static str> {
        self.push_widget(Widget::RadioButton(radio_button))
    }

    pub fn add_dropdown(&mut self, dropdown: Dropdown) -> Result<(), &'static str> {
        self.push_widget(Widget::Dropdown(dropdown))
    }

    pub fn add_combo_box(&mut self, combo_box: ComboBox) -> Result<(), &'static str> {
        self.push_widget(Widget::ComboBox(combo_box))
    }

    pub fn add_scrollbar(&mut self, scrollbar: Scrollbar) -> Result<(), &'static str> {
        self.push_widget(Widget::Scrollbar(scrollbar))
    }

    pub fn add_list_view(&mut self, list_view: ListView) -> Result<(), &'static str> {
        self.push_widget(Widget::ListView(list_view))
    }

    pub fn add_tree_view(&mut self, tree_view: TreeView) -> Result<(), &'static str> {
        self.push_widget(Widget::TreeView(tree_view))
    }

    pub fn add_progress_bar(&mut self, progress_bar: ProgressBar) -> Result<(), &'static str> {
        self.push_widget(Widget::ProgressBar(progress_bar))
    }

    pub fn add_popup_menu(&mut self, popup_menu: PopupMenu) -> Result<(), &'static str> {
        self.push_widget(Widget::PopupMenu(popup_menu))
    }

    pub fn set_progress_value(&mut self, widget_id: WidgetId, value: u32) -> bool {
        let Some(index) = self.widget_index_by_id(widget_id) else {
            return false;
        };

        let Widget::ProgressBar(progress) = &mut self.widgets[index] else {
            return false;
        };

        let clamped = min(value, progress.max.max(1));
        if progress.value == clamped {
            return false;
        }
        progress.value = clamped;
        true
    }

    pub fn scrollbar_value(&self, widget_id: WidgetId) -> Option<i32> {
        let index = self.widget_index_by_id(widget_id)?;
        let Widget::Scrollbar(scrollbar) = &self.widgets[index] else {
            return None;
        };
        Some(scrollbar.value)
    }

    pub fn popup_menu_selected(&self, widget_id: WidgetId) -> Option<usize> {
        let index = self.widget_index_by_id(widget_id)?;
        let Widget::PopupMenu(menu) = &self.widgets[index] else {
            return None;
        };
        menu.selected
    }

    pub fn show_popup_menu(&mut self, widget_id: WidgetId, x: i32, y: i32) -> bool {
        let Some(index) = self.widget_index_by_id(widget_id) else {
            return false;
        };

        let Widget::PopupMenu(menu) = &mut self.widgets[index] else {
            return false;
        };

        menu.show_at(x, y);
        self.rebuild_hit_regions();
        true
    }

    pub fn hide_popup_menu(&mut self, widget_id: WidgetId) -> bool {
        let Some(index) = self.widget_index_by_id(widget_id) else {
            return false;
        };

        let Widget::PopupMenu(menu) = &mut self.widgets[index] else {
            return false;
        };

        if !menu.visible {
            return false;
        }
        menu.hide();
        self.rebuild_hit_regions();
        true
    }

    pub fn draw(&self) {
        for widget in self.widgets.iter() {
            if widget.is_active_overlay() {
                continue;
            }
            let focused = self.focused_id == Some(widget.id());
            widget.draw(focused);
        }

        for widget in self.widgets.iter() {
            if !widget.is_active_overlay() {
                continue;
            }
            let focused = self.focused_id == Some(widget.id());
            widget.draw(focused);
        }
    }

    pub fn focused_widget(&self) -> Option<WidgetId> {
        self.focused_id
    }

    pub fn focus_first(&mut self) -> bool {
        self.cycle_focus_forward()
    }

    pub fn poll_and_dispatch(&mut self, budget: usize) -> DispatchBatch {
        let mut batch = DispatchBatch::default();
        if budget == 0 {
            return batch;
        }

        for _ in 0..budget {
            let Some(event) = input::pop_event() else {
                break;
            };

            batch.processed += 1;
            if let InputEvent::KeyPress { key } = event {
                batch.key_press = Some(key);
            }

            let result = self.dispatch_event(event);
            if result.redraw {
                batch.redraw = true;
            }
            if batch.clicked.is_none() {
                batch.clicked = result.clicked;
            }
        }

        batch
    }

    pub fn dispatch_input_event(&mut self, event: InputEvent) -> DispatchBatch {
        self.dispatch_event(event)
    }

    fn push_widget(&mut self, widget: Widget) -> Result<(), &'static str> {
        if self.widgets.len() >= MAX_WIDGETS {
            return Err("widget capacity reached");
        }

        let id = widget.id();
        if self.widget_index_by_id(id).is_some() {
            return Err("widget id already exists");
        }

        self.widgets.push(widget);
        self.rebuild_hit_regions();
        Ok(())
    }

    fn dispatch_event(&mut self, event: InputEvent) -> DispatchBatch {
        let mut batch = DispatchBatch {
            processed: 1,
            redraw: false,
            clicked: None,
            key_press: None,
        };

        let overlay = self.dispatch_popup_overlays(event);
        if overlay.redraw {
            batch.redraw = true;
        }
        if batch.clicked.is_none() {
            batch.clicked = overlay.clicked;
        }

        match event {
            InputEvent::MouseMove { x, y, .. } => {
                if self.update_hover_states(x, y) {
                    batch.redraw = true;
                }
                if let Some(capture_id) = self.pointer_capture_id {
                    let response = self.dispatch_to_widget(capture_id, event);
                    self.apply_widget_response(capture_id, response, &mut batch);
                } else if let Some(target_id) = self.hit_widget_id(x, y) {
                    let response = self.dispatch_to_widget(target_id, event);
                    self.apply_widget_response(target_id, response, &mut batch);
                }
            }
            InputEvent::MouseDown { button, x, y } => {
                let target_id = self.hit_widget_id(x, y);
                if button == MouseButton::Left {
                    self.pointer_capture_id =
                        target_id.filter(|id| self.is_pointer_capture_widget(*id));
                    let next_focus = target_id.filter(|id| self.is_focusable(*id));
                    if self.set_focus(next_focus) {
                        batch.redraw = true;
                    }
                }

                if let Some(target_id) = target_id {
                    let response = self.dispatch_to_widget(target_id, event);
                    self.apply_widget_response(target_id, response, &mut batch);
                }
            }
            InputEvent::MouseUp { button, x, y } => {
                let mut handled_by_capture = None;
                if let Some(capture_id) = self.pointer_capture_id {
                    let response = self.dispatch_to_widget(capture_id, event);
                    self.apply_widget_response(capture_id, response, &mut batch);
                    handled_by_capture = Some(capture_id);
                }
                if button == MouseButton::Left {
                    self.pointer_capture_id = None;
                }

                if let Some(target_id) = self.hit_widget_id(x, y) {
                    if Some(target_id) != handled_by_capture {
                        let response = self.dispatch_to_widget(target_id, event);
                        self.apply_widget_response(target_id, response, &mut batch);
                    }
                }
            }
            InputEvent::MouseClick { x, y, .. } => {
                if let Some(target_id) = self.hit_widget_id(x, y) {
                    let response = self.dispatch_to_widget(target_id, event);
                    self.apply_widget_response(target_id, response, &mut batch);
                }
            }
            InputEvent::KeyPress {
                key: KeyEvent::Char('\t'),
            } => {
                if self.cycle_focus_forward() {
                    batch.redraw = true;
                }
            }
            InputEvent::KeyPress { .. } | InputEvent::KeyRelease { .. } => {
                if let Some(focused_id) = self.focused_id {
                    let response = self.dispatch_to_widget(focused_id, event);
                    self.apply_widget_response(focused_id, response, &mut batch);
                }
            }
        }

        self.rebuild_hit_regions();
        batch
    }

    fn apply_widget_response(
        &mut self,
        target_id: WidgetId,
        response: WidgetResponse,
        batch: &mut DispatchBatch,
    ) {
        if response.redraw {
            batch.redraw = true;
        }
        if response.clicked {
            if batch.clicked.is_none() {
                batch.clicked = Some(target_id);
            }
            if self.activate_radio_group(target_id) {
                batch.redraw = true;
            }
        }
    }

    fn dispatch_popup_overlays(&mut self, event: InputEvent) -> DispatchBatch {
        let mut batch = DispatchBatch::default();
        for widget in self.widgets.iter_mut().rev() {
            let Widget::PopupMenu(menu) = widget else {
                continue;
            };
            if !menu.visible {
                continue;
            }

            let menu_id = menu.id;
            let response = menu.handle_event(&event, false);
            if response.redraw {
                batch.redraw = true;
            }
            if response.clicked && batch.clicked.is_none() {
                batch.clicked = Some(menu_id);
            }
        }
        batch
    }

    fn activate_radio_group(&mut self, selected_id: WidgetId) -> bool {
        let Some(selected_index) = self.widget_index_by_id(selected_id) else {
            return false;
        };
        let Widget::RadioButton(selected_button) = &self.widgets[selected_index] else {
            return false;
        };
        let group = selected_button.group;

        let mut changed = false;
        for widget in self.widgets.iter_mut() {
            let Widget::RadioButton(button) = widget else {
                continue;
            };
            let should_select = button.id == selected_id;
            if button.group == group && button.selected != should_select {
                button.selected = should_select;
                changed = true;
            }
        }

        changed
    }

    fn dispatch_to_widget(&mut self, target_id: WidgetId, event: InputEvent) -> WidgetResponse {
        let Some(index) = self.widget_index_by_id(target_id) else {
            return WidgetResponse::none();
        };

        let focused = self.focused_id == Some(target_id);
        self.widgets[index].handle_event(&event, focused)
    }

    fn update_hover_states(&mut self, x: i32, y: i32) -> bool {
        let mut changed = false;
        for widget in self.widgets.iter_mut() {
            if widget.update_hover_state(x, y) {
                changed = true;
            }
        }
        changed
    }

    fn set_focus(&mut self, target_id: Option<WidgetId>) -> bool {
        let normalized = target_id.filter(|id| self.is_focusable(*id));
        if self.focused_id == normalized {
            return false;
        }

        self.focused_id = normalized;
        true
    }

    fn cycle_focus_forward(&mut self) -> bool {
        let mut order = [0u16; MAX_WIDGETS];
        let mut count = 0usize;
        for widget in self.widgets.iter() {
            if !widget.is_focusable() {
                continue;
            }
            order[count] = widget.id();
            count += 1;
        }

        if count == 0 {
            return false;
        }

        let next = match self.focused_id {
            None => order[0],
            Some(current) => {
                let mut found_index = None;
                for (index, id) in order.iter().take(count).enumerate() {
                    if *id == current {
                        found_index = Some(index);
                        break;
                    }
                }

                match found_index {
                    Some(index) => order[(index + 1) % count],
                    None => order[0],
                }
            }
        };

        self.set_focus(Some(next))
    }

    fn hit_widget_id(&self, x: i32, y: i32) -> Option<WidgetId> {
        input::hit_test_id(&self.hit_regions[..self.hit_count], x, y)
    }

    fn rebuild_hit_regions(&mut self) {
        self.hit_count = 0;
        for widget in self.widgets.iter() {
            if widget.is_active_overlay() {
                continue;
            }
            if self.hit_count >= MAX_WIDGETS {
                break;
            }
            self.hit_regions[self.hit_count] = widget.hit_rect().as_hit_region(widget.id());
            self.hit_count += 1;
        }

        for widget in self.widgets.iter() {
            if !widget.is_active_overlay() {
                continue;
            }
            if self.hit_count >= MAX_WIDGETS {
                break;
            }
            self.hit_regions[self.hit_count] = widget.hit_rect().as_hit_region(widget.id());
            self.hit_count += 1;
        }
    }

    fn widget_index_by_id(&self, id: WidgetId) -> Option<usize> {
        for (index, widget) in self.widgets.iter().enumerate() {
            if widget.id() == id {
                return Some(index);
            }
        }
        None
    }

    fn is_pointer_capture_widget(&self, id: WidgetId) -> bool {
        self.widget_index_by_id(id)
            .is_some_and(|index| matches!(self.widgets[index], Widget::Scrollbar(_)))
    }

    fn is_focusable(&self, id: WidgetId) -> bool {
        self.widget_index_by_id(id)
            .is_some_and(|index| self.widgets[index].is_focusable())
    }
}
