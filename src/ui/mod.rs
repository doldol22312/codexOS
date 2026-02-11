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


mod widgets;
mod dispatcher;

#[allow(unused_imports)]
pub use dispatcher::{DispatchBatch, EventDispatcher};
pub use widgets::*;
