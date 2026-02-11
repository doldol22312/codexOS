extern crate alloc;

use alloc::vec::Vec;
use core::fmt::{self, Write};

use crate::io::outb;
use crate::paging;
use crate::timer;

const DEFAULT_COLS: usize = 80;
const DEFAULT_ROWS: usize = 25;
const MIN_ROWS: usize = 2;
const MAX_COLS: usize = 256;
const MAX_ROWS: usize = 128;

const SCROLLBACK_ROWS: usize = 1024;

const FONT_WIDTH: usize = 8;
const STATUS_LINE_INPUT_WIDTH: usize = 80;
const CURSOR_BLINK_TICKS: u32 = 50;
const MOUSE_CURSOR_WIDTH: usize = 16;
const MOUSE_CURSOR_HEIGHT: usize = 24;
const MOUSE_CURSOR_SHADOW_OFFSET_X: usize = 1;
const MOUSE_CURSOR_SHADOW_OFFSET_Y: usize = 1;
const MOUSE_CURSOR_SAVE_WIDTH: usize = MOUSE_CURSOR_WIDTH + MOUSE_CURSOR_SHADOW_OFFSET_X;
const MOUSE_CURSOR_SAVE_HEIGHT: usize = MOUSE_CURSOR_HEIGHT + MOUSE_CURSOR_SHADOW_OFFSET_Y;
const MOUSE_CURSOR_FILL_COLOR: u32 = 0x39FF14;
const MOUSE_CURSOR_BORDER_COLOR: u32 = 0xFF00FF;
const MOUSE_CURSOR_SHADOW_COLOR: u32 = 0x101010;
const MOUSE_CURSOR_BITMAP: [u16; MOUSE_CURSOR_HEIGHT] = [
    0x8000, 0xC000, 0xE000, 0xF000, 0xF800, 0xFC00, 0xFE00, 0xFF00, 0xFF80, 0xFFC0, 0xFFE0,
    0xFFF0, 0xFFF8, 0xFC00, 0xFC00, 0xEE00, 0xCE00, 0x8700, 0x0700, 0x0700, 0x0700, 0x0380,
    0x0380, 0x01C0,
];

const VGA_BUFFER: *mut u16 = 0xB8000 as *mut u16;
const DEFAULT_COLOR: u8 = 0x0F;
const BLANK_CELL: TextCell = TextCell {
    ch: b' ',
    color: DEFAULT_COLOR,
};

const VGA_CRTC_INDEX: u16 = 0x3D4;
const VGA_CRTC_DATA: u16 = 0x3D5;

const PALETTE: [u32; 16] = [
    0x000000, 0x0000AA, 0x00AA00, 0x00AAAA, 0xAA0000, 0xAA00AA, 0xAA5500, 0xAAAAAA, 0x555555,
    0x5555FF, 0x55FF55, 0x55FFFF, 0xFF5555, 0xFF55FF, 0xFFFF55, 0xFFFFFF,
];

#[derive(Clone, Copy, PartialEq, Eq)]
struct TextCell {
    ch: u8,
    color: u8,
}

struct ConsoleState {
    cols: usize,
    rows: usize,
    terminal: Vec<TextCell>,
    status: Vec<TextCell>,
    scrollback: Vec<TextCell>,
    scrollback_head: usize,
    scrollback_count: usize,
    view_offset_rows: usize,
}

impl ConsoleState {
    #[inline]
    fn terminal_rows(&self) -> usize {
        self.rows - 1
    }

    #[inline]
    fn terminal_index(&self, row: usize, col: usize) -> usize {
        row * self.cols + col
    }

    #[inline]
    fn scrollback_index(&self, row: usize, col: usize) -> usize {
        row * self.cols + col
    }

    #[inline]
    fn max_cursor_index(&self) -> usize {
        self.terminal_rows() * self.cols - 1
    }
}

struct FramebufferState {
    front_ptr: *mut u8,
    surface_width: usize,
    surface_height: usize,
    pitch_bytes: usize,
    bpp: usize,
    font_ptr: *const u8,
    font_bytes: usize,
    font_height: usize,
    back: Vec<u32>,
    live_terminal_cache: Vec<TextCell>,
    live_status_cache: Vec<TextCell>,
    live_cache_valid: bool,
    cursor_drawn: bool,
    last_cursor_row: usize,
    last_cursor_col: usize,
    mouse_cursor_visible: bool,
    mouse_cursor_x: i32,
    mouse_cursor_y: i32,
    mouse_cursor_drawn: bool,
    mouse_saved_x: usize,
    mouse_saved_y: usize,
    mouse_saved_w: usize,
    mouse_saved_h: usize,
    mouse_saved_pixels: [u32; MOUSE_CURSOR_SAVE_WIDTH * MOUSE_CURSOR_SAVE_HEIGHT],
}

static mut INITIALIZED: bool = false;
static mut CONSOLE: Option<ConsoleState> = None;
static mut FRAMEBUFFER: Option<FramebufferState> = None;

static mut CURSOR_ROW: usize = 0;
static mut CURSOR_COL: usize = 0;
static mut CURRENT_COLOR: u8 = DEFAULT_COLOR;
static mut CURSOR_BLINK_VISIBLE: bool = true;
static mut CURSOR_BLINK_LAST_TICK: u32 = 0;

pub fn init() {
    unsafe {
        if INITIALIZED {
            return;
        }

        let mut target_cols = DEFAULT_COLS;
        let mut target_rows = DEFAULT_ROWS;

        if let Some((framebuffer, cols, rows)) = try_init_framebuffer() {
            FRAMEBUFFER = Some(framebuffer);
            target_cols = cols;
            target_rows = rows;
        }

        if !install_console(target_cols, target_rows) {
            // Last-resort fallback.
            FRAMEBUFFER = None;
            let _ = install_console(DEFAULT_COLS, DEFAULT_ROWS);
        }

        CURSOR_ROW = 0;
        CURSOR_COL = 0;
        CURSOR_BLINK_VISIBLE = true;
        CURSOR_BLINK_LAST_TICK = timer::ticks();

        if FRAMEBUFFER.is_none() {
            enable_hardware_cursor();
        }

        INITIALIZED = true;
        render_active_view_locked();
    }
}

pub fn using_framebuffer() -> bool {
    unsafe { FRAMEBUFFER.is_some() }
}

pub fn text_columns() -> usize {
    ensure_initialized();
    unsafe {
        if let Some(console) = CONSOLE.as_ref() {
            console.cols
        } else {
            DEFAULT_COLS
        }
    }
}

pub fn text_rows() -> usize {
    ensure_initialized();
    unsafe {
        if let Some(console) = CONSOLE.as_ref() {
            console.terminal_rows()
        } else {
            DEFAULT_ROWS - 1
        }
    }
}

#[inline]
fn ensure_initialized() {
    unsafe {
        if !INITIALIZED {
            init();
        }
    }
}

#[inline]
fn palette_color(index: u8) -> u32 {
    PALETTE[(index & 0x0F) as usize]
}

#[inline]
unsafe fn write_vga_cell(row: usize, col: usize, cell: TextCell) {
    let value = ((cell.color as u16) << 8) | cell.ch as u16;
    core::ptr::write_volatile(VGA_BUFFER.add(row * DEFAULT_COLS + col), value);
}

fn sync_hardware_cursor() {
    if using_framebuffer() {
        return;
    }

    unsafe {
        let Some(console) = CONSOLE.as_ref() else {
            return;
        };

        if console.view_offset_rows > 0 {
            return;
        }

        let cols = console.cols.min(DEFAULT_COLS);
        let row = CURSOR_ROW.min(DEFAULT_ROWS - 1);
        let col = CURSOR_COL.min(cols.saturating_sub(1));

        let position = (row * DEFAULT_COLS + col) as u16;
        outb(VGA_CRTC_INDEX, 0x0F);
        outb(VGA_CRTC_DATA, (position & 0xFF) as u8);
        outb(VGA_CRTC_INDEX, 0x0E);
        outb(VGA_CRTC_DATA, (position >> 8) as u8);
    }
}

fn enable_hardware_cursor() {
    if using_framebuffer() {
        return;
    }

    unsafe {
        outb(VGA_CRTC_INDEX, 0x0A);
        outb(VGA_CRTC_DATA, 0x0E);
        outb(VGA_CRTC_INDEX, 0x0B);
        outb(VGA_CRTC_DATA, 0x0F);
    }
}

fn disable_hardware_cursor() {
    if using_framebuffer() {
        return;
    }

    unsafe {
        outb(VGA_CRTC_INDEX, 0x0A);
        outb(VGA_CRTC_DATA, 0x20);
    }
}

#[inline]
fn current_color() -> u8 {
    unsafe { CURRENT_COLOR }
}

#[inline]
unsafe fn reset_cursor_blink_locked() {
    CURSOR_BLINK_VISIBLE = true;
    CURSOR_BLINK_LAST_TICK = timer::ticks();
}

pub fn color_code() -> u8 {
    current_color()
}

pub fn status_row() -> usize {
    ensure_initialized();
    unsafe {
        if let Some(console) = CONSOLE.as_ref() {
            console.rows - 1
        } else {
            DEFAULT_ROWS - 1
        }
    }
}

pub fn move_cursor_left(count: usize) {
    ensure_initialized();
    leave_scrollback_view_if_needed();

    unsafe {
        let Some(console) = CONSOLE.as_ref() else {
            return;
        };

        let index = CURSOR_ROW * console.cols + CURSOR_COL;
        let next = index.saturating_sub(count);
        CURSOR_ROW = next / console.cols;
        CURSOR_COL = next % console.cols;
        reset_cursor_blink_locked();
        render_active_view_locked();
    }
}

pub fn move_cursor_right(count: usize) {
    ensure_initialized();
    leave_scrollback_view_if_needed();

    unsafe {
        let Some(console) = CONSOLE.as_ref() else {
            return;
        };

        let index = CURSOR_ROW * console.cols + CURSOR_COL;
        let next = index.saturating_add(count).min(console.max_cursor_index());
        CURSOR_ROW = next / console.cols;
        CURSOR_COL = next % console.cols;
        reset_cursor_blink_locked();
        render_active_view_locked();
    }
}

pub fn foreground_color() -> u8 {
    color_code() & 0x0F
}

pub fn background_color() -> u8 {
    (color_code() >> 4) & 0x0F
}

pub fn set_color(foreground: u8, background: u8) {
    unsafe {
        CURRENT_COLOR = ((background & 0x0F) << 4) | (foreground & 0x0F);
    }
}

pub fn set_color_code(color: u8) {
    unsafe {
        CURRENT_COLOR = color;
    }
}

pub fn clear_screen() {
    ensure_initialized();

    unsafe {
        let Some(console) = CONSOLE.as_mut() else {
            return;
        };

        console.view_offset_rows = 0;
        for cell in console.terminal.iter_mut() {
            *cell = TextCell {
                ch: b' ',
                color: current_color(),
            };
        }

        for cell in console.status.iter_mut() {
            *cell = TextCell {
                ch: b' ',
                color: current_color(),
            };
        }

        CURSOR_ROW = 0;
        CURSOR_COL = 0;
        reset_cursor_blink_locked();

        if FRAMEBUFFER.is_none() {
            enable_hardware_cursor();
        }
        render_active_view_locked();
    }
}

pub fn write_status_line(bytes: &[u8; STATUS_LINE_INPUT_WIDTH], color: u8) {
    ensure_initialized();

    unsafe {
        let Some(console) = CONSOLE.as_mut() else {
            return;
        };

        for cell in console.status.iter_mut() {
            *cell = TextCell { ch: b' ', color };
        }

        let copy_len = STATUS_LINE_INPUT_WIDTH.min(console.cols);
        for (idx, byte) in bytes.iter().copied().take(copy_len).enumerate() {
            console.status[idx] = TextCell { ch: byte, color };
        }

        render_status_row_locked();
    }
}

pub fn write_char_at(row: usize, col: usize, ch: u8, color: u8) {
    ensure_initialized();

    unsafe {
        let Some(console) = CONSOLE.as_mut() else {
            return;
        };

        if row >= console.rows || col >= console.cols {
            return;
        }

        let cell = TextCell { ch, color };
        if row < console.terminal_rows() {
            let index = console.terminal_index(row, col);
            console.terminal[index] = cell;
        } else {
            console.status[col] = cell;
        }
    }
}

pub fn present() {
    ensure_initialized();
    unsafe {
        render_active_view_locked();
    }
}

pub fn tick_cursor_blink() {
    ensure_initialized();

    unsafe {
        if FRAMEBUFFER.is_none() {
            return;
        }

        let now = timer::ticks();
        if now.wrapping_sub(CURSOR_BLINK_LAST_TICK) < CURSOR_BLINK_TICKS {
            return;
        }

        CURSOR_BLINK_LAST_TICK = now;
        CURSOR_BLINK_VISIBLE = !CURSOR_BLINK_VISIBLE;
        render_active_view_locked();
    }
}

pub fn framebuffer_resolution() -> Option<(usize, usize)> {
    ensure_initialized();
    unsafe {
        FRAMEBUFFER
            .as_ref()
            .map(|state| (state.surface_width, state.surface_height))
    }
}

pub fn set_mouse_cursor(x: i32, y: i32, visible: bool) {
    ensure_initialized();

    unsafe {
        let Some(state) = FRAMEBUFFER.as_mut() else {
            return;
        };

        state.mouse_cursor_x = x;
        state.mouse_cursor_y = y;
        state.mouse_cursor_visible = visible;
        refresh_mouse_cursor_locked(state);
    }
}

pub fn draw_filled_rect(x: i32, y: i32, width: i32, height: i32, rgb: u32) -> bool {
    ensure_initialized();
    unsafe {
        let Some(state) = FRAMEBUFFER.as_mut() else {
            return false;
        };
        prepare_raw_framebuffer_draw(state);
        fill_rect_clipped(state, x, y, width, height, rgb);
    }
    true
}

pub fn draw_horizontal_line(x: i32, y: i32, length: i32, rgb: u32) -> bool {
    ensure_initialized();
    unsafe {
        let Some(state) = FRAMEBUFFER.as_mut() else {
            return false;
        };
        prepare_raw_framebuffer_draw(state);
        draw_hline_clipped(state, x, y, length, rgb);
    }
    true
}

pub fn draw_vertical_line(x: i32, y: i32, length: i32, rgb: u32) -> bool {
    ensure_initialized();
    unsafe {
        let Some(state) = FRAMEBUFFER.as_mut() else {
            return false;
        };
        prepare_raw_framebuffer_draw(state);
        draw_vline_clipped(state, x, y, length, rgb);
    }
    true
}

pub fn draw_line(x0: i32, y0: i32, x1: i32, y1: i32, rgb: u32) -> bool {
    ensure_initialized();
    unsafe {
        let Some(state) = FRAMEBUFFER.as_mut() else {
            return false;
        };
        prepare_raw_framebuffer_draw(state);

        let mut dirty: Option<(usize, usize, usize, usize)> = None;
        let mut x = x0;
        let mut y = y0;
        let dx = (x1 - x).abs();
        let sx = if x < x1 { 1 } else { -1 };
        let dy = -(y1 - y).abs();
        let sy = if y < y1 { 1 } else { -1 };
        let mut err = dx + dy;

        loop {
            draw_pixel_clipped(state, x, y, rgb, &mut dirty);

            if x == x1 && y == y1 {
                break;
            }

            let twice_err = 2 * err;
            if twice_err >= dy {
                err += dy;
                x += sx;
            }
            if twice_err <= dx {
                err += dx;
                y += sy;
            }
        }

        flush_dirty_rect(state, dirty);
    }
    true
}

pub fn draw_circle(cx: i32, cy: i32, radius: i32, rgb: u32) -> bool {
    if radius < 0 {
        return true;
    }

    ensure_initialized();
    unsafe {
        let Some(state) = FRAMEBUFFER.as_mut() else {
            return false;
        };
        prepare_raw_framebuffer_draw(state);

        let mut dirty: Option<(usize, usize, usize, usize)> = None;
        let mut x = radius;
        let mut y = 0;
        let mut decision = 1 - radius;

        while x >= y {
            draw_circle_octants(state, cx, cy, x, y, rgb, &mut dirty);
            y += 1;
            if decision < 0 {
                decision += 2 * y + 1;
            } else {
                x -= 1;
                decision += 2 * (y - x) + 1;
            }
        }

        flush_dirty_rect(state, dirty);
    }
    true
}

pub fn draw_ellipse(cx: i32, cy: i32, rx: i32, ry: i32, rgb: u32) -> bool {
    if rx < 0 || ry < 0 {
        return true;
    }

    ensure_initialized();
    unsafe {
        let Some(state) = FRAMEBUFFER.as_mut() else {
            return false;
        };
        prepare_raw_framebuffer_draw(state);

        let mut dirty: Option<(usize, usize, usize, usize)> = None;

        let rx64 = rx as i64;
        let ry64 = ry as i64;
        let rx2 = rx64 * rx64;
        let ry2 = ry64 * ry64;

        let mut x = 0i64;
        let mut y = ry64;
        let mut dx = 0i64;
        let mut dy = 2 * rx2 * y;
        let mut d1 = ry2 - rx2 * ry64 + (rx2 / 4);

        while dx < dy {
            draw_ellipse_quadrants(state, cx, cy, x as i32, y as i32, rgb, &mut dirty);
            x += 1;
            dx += 2 * ry2;
            if d1 < 0 {
                d1 += dx + ry2;
            } else {
                y -= 1;
                dy -= 2 * rx2;
                d1 += dx - dy + ry2;
            }
        }

        let two_x_plus_one = 2 * x + 1;
        let y_minus_one = y - 1;
        let mut d2 = (ry2 * two_x_plus_one * two_x_plus_one) / 4
            + (rx2 * y_minus_one * y_minus_one)
            - (rx2 * ry2);

        while y >= 0 {
            draw_ellipse_quadrants(state, cx, cy, x as i32, y as i32, rgb, &mut dirty);
            y -= 1;
            dy -= 2 * rx2;
            if d2 > 0 {
                d2 += rx2 - dy;
            } else {
                x += 1;
                dx += 2 * ry2;
                d2 += dx - dy + rx2;
            }
        }

        flush_dirty_rect(state, dirty);
    }
    true
}

pub fn blit_bitmap(
    dst_x: i32,
    dst_y: i32,
    src_pixels: &[u32],
    src_width: usize,
    src_height: usize,
    src_stride: usize,
) -> bool {
    ensure_initialized();

    if src_width == 0 || src_height == 0 || src_stride < src_width {
        return false;
    }

    let required = match src_stride.checked_mul(src_height) {
        Some(value) => value,
        None => return false,
    };
    if src_pixels.len() < required {
        return false;
    }

    unsafe {
        let Some(state) = FRAMEBUFFER.as_mut() else {
            return false;
        };
        prepare_raw_framebuffer_draw(state);

        let sw = state.surface_width as i32;
        let sh = state.surface_height as i32;
        if sw <= 0 || sh <= 0 {
            return true;
        }

        let mut src_x = 0i32;
        let mut src_y = 0i32;
        let mut dst_x0 = dst_x;
        let mut dst_y0 = dst_y;
        let mut copy_w = src_width as i32;
        let mut copy_h = src_height as i32;

        if dst_x0 < 0 {
            src_x = -dst_x0;
            copy_w -= src_x;
            dst_x0 = 0;
        }
        if dst_y0 < 0 {
            src_y = -dst_y0;
            copy_h -= src_y;
            dst_y0 = 0;
        }
        if dst_x0 >= sw || dst_y0 >= sh || copy_w <= 0 || copy_h <= 0 {
            return true;
        }

        let max_w = sw - dst_x0;
        let max_h = sh - dst_y0;
        if copy_w > max_w {
            copy_w = max_w;
        }
        if copy_h > max_h {
            copy_h = max_h;
        }
        if copy_w <= 0 || copy_h <= 0 {
            return true;
        }

        let dst_x_usize = dst_x0 as usize;
        let dst_y_usize = dst_y0 as usize;
        let src_x_usize = src_x as usize;
        let src_y_usize = src_y as usize;
        let copy_w_usize = copy_w as usize;
        let copy_h_usize = copy_h as usize;

        for row in 0..copy_h_usize {
            let src_row = (src_y_usize + row) * src_stride + src_x_usize;
            let dst_row = (dst_y_usize + row) * state.surface_width + dst_x_usize;
            let src_slice = &src_pixels[src_row..src_row + copy_w_usize];
            let dst_slice = &mut state.back[dst_row..dst_row + copy_w_usize];
            dst_slice.copy_from_slice(src_slice);
        }

        flush_backbuffer_rect(state, dst_x_usize, dst_y_usize, copy_w_usize, copy_h_usize);
    }
    true
}

#[inline]
unsafe fn prepare_raw_framebuffer_draw(state: &mut FramebufferState) {
    // Primitive pixel drawing bypasses text-cell cache; force text renderer
    // to rebuild on next text update.
    state.live_cache_valid = false;
    state.cursor_drawn = false;
}

unsafe fn fill_rect_clipped(
    state: &mut FramebufferState,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    rgb: u32,
) {
    if width <= 0 || height <= 0 {
        return;
    }

    let sw = state.surface_width as i32;
    let sh = state.surface_height as i32;
    if sw <= 0 || sh <= 0 {
        return;
    }

    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = x.saturating_add(width).min(sw);
    let y1 = y.saturating_add(height).min(sh);
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let x0_usize = x0 as usize;
    let x1_usize = x1 as usize;
    for py in y0 as usize..y1 as usize {
        let row_start = py * state.surface_width;
        for px in x0_usize..x1_usize {
            state.back[row_start + px] = rgb;
        }
    }

    flush_backbuffer_rect(
        state,
        x0_usize,
        y0 as usize,
        (x1 - x0) as usize,
        (y1 - y0) as usize,
    );
}

unsafe fn draw_hline_clipped(state: &mut FramebufferState, x: i32, y: i32, length: i32, rgb: u32) {
    if length <= 0 {
        return;
    }

    let sh = state.surface_height as i32;
    if y < 0 || y >= sh {
        return;
    }

    let sw = state.surface_width as i32;
    let x0 = x.max(0);
    let x1 = x.saturating_add(length).min(sw);
    if x0 >= x1 {
        return;
    }

    let row_start = y as usize * state.surface_width;
    for px in x0 as usize..x1 as usize {
        state.back[row_start + px] = rgb;
    }

    flush_backbuffer_rect(state, x0 as usize, y as usize, (x1 - x0) as usize, 1);
}

unsafe fn draw_vline_clipped(state: &mut FramebufferState, x: i32, y: i32, length: i32, rgb: u32) {
    if length <= 0 {
        return;
    }

    let sw = state.surface_width as i32;
    if x < 0 || x >= sw {
        return;
    }

    let sh = state.surface_height as i32;
    let y0 = y.max(0);
    let y1 = y.saturating_add(length).min(sh);
    if y0 >= y1 {
        return;
    }

    let x_usize = x as usize;
    for py in y0 as usize..y1 as usize {
        let row_start = py * state.surface_width;
        state.back[row_start + x_usize] = rgb;
    }

    flush_backbuffer_rect(state, x_usize, y0 as usize, 1, (y1 - y0) as usize);
}

#[inline]
unsafe fn draw_pixel_clipped(
    state: &mut FramebufferState,
    x: i32,
    y: i32,
    rgb: u32,
    dirty: &mut Option<(usize, usize, usize, usize)>,
) {
    if x < 0 || y < 0 {
        return;
    }

    let x_usize = x as usize;
    let y_usize = y as usize;
    if x_usize >= state.surface_width || y_usize >= state.surface_height {
        return;
    }

    let index = y_usize * state.surface_width + x_usize;
    state.back[index] = rgb;
    expand_dirty_rect(dirty, x_usize, y_usize, x_usize + 1, y_usize + 1);
}

#[inline]
unsafe fn draw_circle_octants(
    state: &mut FramebufferState,
    cx: i32,
    cy: i32,
    x: i32,
    y: i32,
    rgb: u32,
    dirty: &mut Option<(usize, usize, usize, usize)>,
) {
    draw_pixel_clipped(state, cx + x, cy + y, rgb, dirty);
    draw_pixel_clipped(state, cx - x, cy + y, rgb, dirty);
    draw_pixel_clipped(state, cx + x, cy - y, rgb, dirty);
    draw_pixel_clipped(state, cx - x, cy - y, rgb, dirty);
    draw_pixel_clipped(state, cx + y, cy + x, rgb, dirty);
    draw_pixel_clipped(state, cx - y, cy + x, rgb, dirty);
    draw_pixel_clipped(state, cx + y, cy - x, rgb, dirty);
    draw_pixel_clipped(state, cx - y, cy - x, rgb, dirty);
}

#[inline]
unsafe fn draw_ellipse_quadrants(
    state: &mut FramebufferState,
    cx: i32,
    cy: i32,
    x: i32,
    y: i32,
    rgb: u32,
    dirty: &mut Option<(usize, usize, usize, usize)>,
) {
    draw_pixel_clipped(state, cx + x, cy + y, rgb, dirty);
    draw_pixel_clipped(state, cx - x, cy + y, rgb, dirty);
    draw_pixel_clipped(state, cx + x, cy - y, rgb, dirty);
    draw_pixel_clipped(state, cx - x, cy - y, rgb, dirty);
}

#[inline]
unsafe fn flush_dirty_rect(
    state: &mut FramebufferState,
    dirty: Option<(usize, usize, usize, usize)>,
) {
    if let Some((x0, y0, x1, y1)) = dirty {
        flush_backbuffer_rect(state, x0, y0, x1 - x0, y1 - y0);
    }
}

fn scroll() {
    unsafe {
        let Some(console) = CONSOLE.as_mut() else {
            return;
        };

        push_scrollback_row_from_terminal(console, 0);

        let cols = console.cols;
        let terminal_rows = console.terminal_rows();
        let copy_start = cols;
        let copy_end = terminal_rows * cols;
        console.terminal.copy_within(copy_start..copy_end, 0);

        let last_row_start = (terminal_rows - 1) * cols;
        for col in 0..cols {
            console.terminal[last_row_start + col] = TextCell {
                ch: b' ',
                color: current_color(),
            };
        }

        if CURSOR_ROW > 0 {
            CURSOR_ROW -= 1;
        }
    }
}

pub fn backspace() {
    ensure_initialized();
    leave_scrollback_view_if_needed();

    unsafe {
        backspace_internal();
        render_active_view_locked();
    }
}

pub fn put_char(ch: char) {
    ensure_initialized();
    leave_scrollback_view_if_needed();

    unsafe {
        put_char_internal(ch);
        render_active_view_locked();
    }
}

unsafe fn backspace_internal() {
    let Some(console) = CONSOLE.as_mut() else {
        return;
    };

    if CURSOR_COL > 0 {
        CURSOR_COL -= 1;
    } else if CURSOR_ROW > 0 {
        CURSOR_ROW -= 1;
        CURSOR_COL = console.cols - 1;
    } else {
        return;
    }

    let index = console.terminal_index(CURSOR_ROW, CURSOR_COL);
    console.terminal[index] = TextCell {
        ch: b' ',
        color: current_color(),
    };
    reset_cursor_blink_locked();
}

unsafe fn put_char_internal(ch: char) {
    let Some(console) = CONSOLE.as_mut() else {
        return;
    };

    if CURSOR_ROW >= console.terminal_rows() {
        CURSOR_ROW = console.terminal_rows() - 1;
        CURSOR_COL = CURSOR_COL.min(console.cols - 1);
    }

    match ch {
        '\n' => {
            CURSOR_COL = 0;
            CURSOR_ROW += 1;
        }
        '\r' => {
            CURSOR_COL = 0;
        }
        '\x08' => {
            backspace_internal();
            return;
        }
        _ => {
            let index = console.terminal_index(CURSOR_ROW, CURSOR_COL);
            console.terminal[index] = TextCell {
                ch: ch as u8,
                color: current_color(),
            };
            CURSOR_COL += 1;
            if CURSOR_COL >= console.cols {
                CURSOR_COL = 0;
                CURSOR_ROW += 1;
            }
        }
    }

    if CURSOR_ROW >= console.terminal_rows() {
        scroll();
    }
    reset_cursor_blink_locked();
}

pub fn page_up() {
    ensure_initialized();

    unsafe {
        let Some(console) = CONSOLE.as_mut() else {
            return;
        };

        if console.scrollback_count == 0 {
            return;
        }

        let step = console.terminal_rows().max(1);
        let next = console
            .view_offset_rows
            .saturating_add(step)
            .min(console.scrollback_count);
        if next == console.view_offset_rows {
            return;
        }

        console.view_offset_rows = next;
        disable_hardware_cursor();
        render_active_view_locked();
    }
}

pub fn page_down() {
    ensure_initialized();

    unsafe {
        let Some(console) = CONSOLE.as_mut() else {
            return;
        };

        if console.view_offset_rows == 0 {
            return;
        }

        let step = console.terminal_rows().max(1);
        if console.view_offset_rows <= step {
            console.view_offset_rows = 0;
            if FRAMEBUFFER.is_none() {
                enable_hardware_cursor();
            }
        } else {
            console.view_offset_rows -= step;
        }

        render_active_view_locked();
    }
}

#[inline]
fn leave_scrollback_view_if_needed() {
    unsafe {
        let Some(console) = CONSOLE.as_mut() else {
            return;
        };

        if console.view_offset_rows == 0 {
            return;
        }

        console.view_offset_rows = 0;
        if FRAMEBUFFER.is_none() {
            enable_hardware_cursor();
        }
        render_active_view_locked();
    }
}

#[inline]
fn scrollback_oldest_index(console: &ConsoleState) -> usize {
    (console.scrollback_head + SCROLLBACK_ROWS - console.scrollback_count) % SCROLLBACK_ROWS
}

#[inline]
fn push_scrollback_row_from_terminal(console: &mut ConsoleState, row: usize) {
    let cols = console.cols;
    let src_start = console.terminal_index(row, 0);
    let dst_start = console.scrollback_index(console.scrollback_head, 0);

    console.scrollback[dst_start..dst_start + cols]
        .copy_from_slice(&console.terminal[src_start..src_start + cols]);

    console.scrollback_head = (console.scrollback_head + 1) % SCROLLBACK_ROWS;
    if console.scrollback_count < SCROLLBACK_ROWS {
        console.scrollback_count += 1;
    }
}

#[inline]
fn view_row_slice<'a>(console: &'a ConsoleState, source_row: usize) -> &'a [TextCell] {
    let cols = console.cols;
    if source_row < console.scrollback_count {
        let oldest = scrollback_oldest_index(console);
        let row = (oldest + source_row) % SCROLLBACK_ROWS;
        let start = console.scrollback_index(row, 0);
        &console.scrollback[start..start + cols]
    } else {
        let live_row = source_row - console.scrollback_count;
        let start = console.terminal_index(live_row, 0);
        &console.terminal[start..start + cols]
    }
}

unsafe fn render_active_view_locked() {
    if FRAMEBUFFER.is_some() {
        render_framebuffer_locked();
    } else {
        render_vga_locked();
    }
}

unsafe fn render_vga_locked() {
    let Some(console) = CONSOLE.as_ref() else {
        return;
    };

    let cols = console.cols.min(DEFAULT_COLS);
    let terminal_rows = console.terminal_rows().min(DEFAULT_ROWS - 1);

    if console.view_offset_rows == 0 {
        for row in 0..terminal_rows {
            let start = console.terminal_index(row, 0);
            for col in 0..cols {
                write_vga_cell(row, col, console.terminal[start + col]);
            }
        }
    } else {
        let total_rows = console.scrollback_count + terminal_rows;
        let start_row = total_rows.saturating_sub(terminal_rows + console.view_offset_rows);

        for screen_row in 0..terminal_rows {
            let source_row = start_row + screen_row;
            let row = view_row_slice(console, source_row);
            for col in 0..cols {
                write_vga_cell(screen_row, col, row[col]);
            }
        }
    }

    for col in 0..cols {
        write_vga_cell(DEFAULT_ROWS - 1, col, console.status[col]);
    }

    sync_hardware_cursor();
}

unsafe fn render_status_row_locked() {
    let Some(console) = CONSOLE.as_ref() else {
        return;
    };

    if let Some(state) = FRAMEBUFFER.as_mut() {
        let terminal_rows = console.terminal_rows();
        let cell_height = state.font_height.max(1);

        for col in 0..console.cols {
            draw_cell_to_backbuffer(state, terminal_rows, col, console.status[col]);
            if state.live_cache_valid && col < state.live_status_cache.len() {
                state.live_status_cache[col] = console.status[col];
            }
        }

        flush_backbuffer_rect(
            state,
            0,
            terminal_rows * cell_height,
            console.cols * FONT_WIDTH,
            cell_height,
        );
        return;
    }

    for col in 0..console.cols.min(DEFAULT_COLS) {
        write_vga_cell(DEFAULT_ROWS - 1, col, console.status[col]);
    }
    sync_hardware_cursor();
}

unsafe fn render_framebuffer_locked() {
    let Some(console) = CONSOLE.as_ref() else {
        return;
    };
    let Some(state) = FRAMEBUFFER.as_mut() else {
        return;
    };

    if console.view_offset_rows > 0 {
        state.live_cache_valid = false;
        state.cursor_drawn = false;
        render_framebuffer_scrollback_locked(console, state);
        return;
    }

    render_framebuffer_live_locked(console, state);
}

#[inline]
fn expand_dirty_rect(
    dirty: &mut Option<(usize, usize, usize, usize)>,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
) {
    match dirty {
        Some((min_x, min_y, max_x, max_y)) => {
            if x0 < *min_x {
                *min_x = x0;
            }
            if y0 < *min_y {
                *min_y = y0;
            }
            if x1 > *max_x {
                *max_x = x1;
            }
            if y1 > *max_y {
                *max_y = y1;
            }
        }
        None => *dirty = Some((x0, y0, x1, y1)),
    }
}

#[inline]
fn mark_dirty_cell(
    dirty: &mut Option<(usize, usize, usize, usize)>,
    row: usize,
    col: usize,
    cell_height: usize,
) {
    let x0 = col * FONT_WIDTH;
    let y0 = row * cell_height;
    expand_dirty_rect(dirty, x0, y0, x0 + FONT_WIDTH, y0 + cell_height);
}

unsafe fn render_framebuffer_live_locked(console: &ConsoleState, state: &mut FramebufferState) {
    let cols = console.cols;
    let terminal_rows = console.terminal_rows();
    let cell_height = state.font_height.max(1);
    let mut dirty: Option<(usize, usize, usize, usize)> = None;

    if !state.live_cache_valid {
        for row in 0..terminal_rows {
            let start = console.terminal_index(row, 0);
            for col in 0..cols {
                let cell = console.terminal[start + col];
                draw_cell_to_backbuffer(state, row, col, cell);
                state.live_terminal_cache[start + col] = cell;
            }
        }

        for col in 0..cols {
            let cell = console.status[col];
            draw_cell_to_backbuffer(state, terminal_rows, col, cell);
            state.live_status_cache[col] = cell;
        }

        expand_dirty_rect(
            &mut dirty,
            0,
            0,
            cols * FONT_WIDTH,
            (terminal_rows + 1) * cell_height,
        );
    } else {
        for idx in 0..console.terminal.len() {
            let cell = console.terminal[idx];
            if state.live_terminal_cache[idx] == cell {
                continue;
            }

            state.live_terminal_cache[idx] = cell;
            let row = idx / cols;
            let col = idx % cols;
            draw_cell_to_backbuffer(state, row, col, cell);
            mark_dirty_cell(&mut dirty, row, col, cell_height);
        }

        for col in 0..cols {
            let cell = console.status[col];
            if state.live_status_cache[col] == cell {
                continue;
            }

            state.live_status_cache[col] = cell;
            draw_cell_to_backbuffer(state, terminal_rows, col, cell);
            mark_dirty_cell(&mut dirty, terminal_rows, col, cell_height);
        }
    }

    if state.cursor_drawn && state.last_cursor_row < terminal_rows && state.last_cursor_col < cols {
        let idx = console.terminal_index(state.last_cursor_row, state.last_cursor_col);
        draw_cell_to_backbuffer(
            state,
            state.last_cursor_row,
            state.last_cursor_col,
            console.terminal[idx],
        );
        mark_dirty_cell(
            &mut dirty,
            state.last_cursor_row,
            state.last_cursor_col,
            cell_height,
        );
    }

    if CURSOR_BLINK_VISIBLE && CURSOR_ROW < terminal_rows && CURSOR_COL < cols {
        draw_cursor_overlay(state);
        mark_dirty_cell(&mut dirty, CURSOR_ROW, CURSOR_COL, cell_height);
        state.cursor_drawn = true;
        state.last_cursor_row = CURSOR_ROW;
        state.last_cursor_col = CURSOR_COL;
    } else {
        state.cursor_drawn = false;
    }

    state.live_cache_valid = true;

    if let Some((x0, y0, x1, y1)) = dirty {
        flush_backbuffer_rect(state, x0, y0, x1 - x0, y1 - y0);
    }
}

unsafe fn render_framebuffer_scrollback_locked(
    console: &ConsoleState,
    state: &mut FramebufferState,
) {
    let cols = console.cols;
    let terminal_rows = console.terminal_rows();

    let bg = palette_color((current_color() >> 4) & 0x0F);
    for pixel in state.back.iter_mut() {
        *pixel = bg;
    }

    let total_rows = console.scrollback_count + terminal_rows;
    let start_row = total_rows.saturating_sub(terminal_rows + console.view_offset_rows);

    for screen_row in 0..terminal_rows {
        let source_row = start_row + screen_row;
        let row = view_row_slice(console, source_row);
        for col in 0..cols {
            draw_cell_to_backbuffer(state, screen_row, col, row[col]);
        }
    }

    for col in 0..cols {
        draw_cell_to_backbuffer(state, terminal_rows, col, console.status[col]);
    }

    flush_backbuffer(state);
}

unsafe fn draw_cell_to_backbuffer(
    state: &mut FramebufferState,
    row: usize,
    col: usize,
    cell: TextCell,
) {
    let cell_height = state.font_height.max(1);
    let x0 = col * FONT_WIDTH;
    let y0 = row * cell_height;
    if x0 >= state.surface_width || y0 >= state.surface_height {
        return;
    }

    let fg = palette_color(cell.color & 0x0F);
    let bg = palette_color((cell.color >> 4) & 0x0F);

    for y in 0..cell_height {
        let py = y0 + y;
        if py >= state.surface_height {
            break;
        }

        let bits = glyph_bits(state, cell.ch, y);
        let row_offset = py * state.surface_width;

        for x in 0..FONT_WIDTH {
            let px = x0 + x;
            if px >= state.surface_width {
                break;
            }

            let mask = 0x80u8 >> x;
            state.back[row_offset + px] = if (bits & mask) != 0 { fg } else { bg };
        }
    }
}

unsafe fn draw_cursor_overlay(state: &mut FramebufferState) {
    let cell_height = state.font_height.max(1);
    let x0 = CURSOR_COL * FONT_WIDTH;
    let y0 = CURSOR_ROW * cell_height;
    if x0 >= state.surface_width || y0 >= state.surface_height {
        return;
    }

    let fg = palette_color(current_color() & 0x0F);
    let start = cell_height.saturating_sub(2);
    for y in start..cell_height {
        let py = y0 + y;
        if py >= state.surface_height {
            break;
        }
        let row_offset = py * state.surface_width;
        for x in 0..FONT_WIDTH {
            let px = x0 + x;
            if px >= state.surface_width {
                break;
            }
            state.back[row_offset + px] = fg;
        }
    }
}

#[inline]
unsafe fn glyph_bits(state: &FramebufferState, ch: u8, row: usize) -> u8 {
    if row < state.font_height && !state.font_ptr.is_null() {
        let index = ch as usize * state.font_height + row;
        if index < state.font_bytes {
            return core::ptr::read_volatile(state.font_ptr.add(index));
        }
    }

    if ch == b' ' {
        0
    } else if row == 0 || row + 1 == state.font_height {
        0x7E
    } else if row == state.font_height / 2 {
        0x7E
    } else {
        0x42
    }
}

#[inline]
fn bytes_per_pixel_for_bpp(bpp: usize) -> usize {
    match bpp {
        0..=23 => 2,
        24..=31 => 3,
        _ => 4,
    }
}

unsafe fn flush_backbuffer(state: &mut FramebufferState) {
    flush_backbuffer_rect(state, 0, 0, state.surface_width, state.surface_height);
}

unsafe fn flush_backbuffer_rect(
    state: &mut FramebufferState,
    x0: usize,
    y0: usize,
    width: usize,
    height: usize,
) {
    let bytes_per_pixel = bytes_per_pixel_for_bpp(state.bpp);

    let x_start = x0.min(state.surface_width);
    let y_start = y0.min(state.surface_height);
    let x_end = x_start.saturating_add(width).min(state.surface_width);
    let y_end = y_start.saturating_add(height).min(state.surface_height);
    if x_start >= x_end || y_start >= y_end {
        return;
    }

    let refresh_cursor = state.mouse_cursor_drawn
        && rects_intersect(
            x_start,
            y_start,
            x_end - x_start,
            y_end - y_start,
            state.mouse_saved_x,
            state.mouse_saved_y,
            state.mouse_saved_w,
            state.mouse_saved_h,
        );

    if refresh_cursor {
        restore_mouse_cursor_locked(state);
    }

    for y in y_start..y_end {
        let src_row = y * state.surface_width;
        let dst_row = state.front_ptr.add(y * state.pitch_bytes);

        for x in x_start..x_end {
            let pixel = state.back[src_row + x];
            let dst = dst_row.add(x * bytes_per_pixel);
            write_pixel(dst, state.bpp, pixel);
        }
    }

    if state.mouse_cursor_visible && (refresh_cursor || !state.mouse_cursor_drawn) {
        draw_mouse_cursor_locked(state);
    }
}

#[inline]
fn rects_intersect(
    ax: usize,
    ay: usize,
    aw: usize,
    ah: usize,
    bx: usize,
    by: usize,
    bw: usize,
    bh: usize,
) -> bool {
    if aw == 0 || ah == 0 || bw == 0 || bh == 0 {
        return false;
    }

    let ax1 = ax.saturating_add(aw);
    let ay1 = ay.saturating_add(ah);
    let bx1 = bx.saturating_add(bw);
    let by1 = by.saturating_add(bh);

    ax < bx1 && ax1 > bx && ay < by1 && ay1 > by
}

unsafe fn refresh_mouse_cursor_locked(state: &mut FramebufferState) {
    if state.mouse_cursor_drawn {
        restore_mouse_cursor_locked(state);
    }

    if state.mouse_cursor_visible {
        draw_mouse_cursor_locked(state);
    }
}

unsafe fn restore_mouse_cursor_locked(state: &mut FramebufferState) {
    if !state.mouse_cursor_drawn || state.mouse_saved_w == 0 || state.mouse_saved_h == 0 {
        state.mouse_cursor_drawn = false;
        return;
    }

    let bytes_per_pixel = bytes_per_pixel_for_bpp(state.bpp);
    for row in 0..state.mouse_saved_h {
        let dst_row = state.front_ptr.add(
            (state.mouse_saved_y + row) * state.pitch_bytes + state.mouse_saved_x * bytes_per_pixel,
        );
        let saved_row = row * MOUSE_CURSOR_SAVE_WIDTH;
        for col in 0..state.mouse_saved_w {
            let pixel = state.mouse_saved_pixels[saved_row + col];
            write_pixel(dst_row.add(col * bytes_per_pixel), state.bpp, pixel);
        }
    }

    state.mouse_cursor_drawn = false;
    state.mouse_saved_w = 0;
    state.mouse_saved_h = 0;
}

unsafe fn draw_mouse_cursor_locked(state: &mut FramebufferState) {
    let sw = state.surface_width as i32;
    let sh = state.surface_height as i32;
    if sw <= 0 || sh <= 0 {
        state.mouse_cursor_drawn = false;
        return;
    }

    let x = state.mouse_cursor_x;
    let y = state.mouse_cursor_y;
    let shadow_x = x.saturating_add(MOUSE_CURSOR_SHADOW_OFFSET_X as i32);
    let shadow_y = y.saturating_add(MOUSE_CURSOR_SHADOW_OFFSET_Y as i32);

    let x0 = x.min(shadow_x).max(0);
    let y0 = y.min(shadow_y).max(0);
    let x1 = x
        .saturating_add(MOUSE_CURSOR_WIDTH as i32)
        .max(shadow_x.saturating_add(MOUSE_CURSOR_WIDTH as i32))
        .min(sw);
    let y1 = y
        .saturating_add(MOUSE_CURSOR_HEIGHT as i32)
        .max(shadow_y.saturating_add(MOUSE_CURSOR_HEIGHT as i32))
        .min(sh);
    if x0 >= x1 || y0 >= y1 {
        state.mouse_cursor_drawn = false;
        return;
    }

    let x0_usize = x0 as usize;
    let y0_usize = y0 as usize;
    let width = ((x1 - x0) as usize).min(MOUSE_CURSOR_SAVE_WIDTH);
    let height = ((y1 - y0) as usize).min(MOUSE_CURSOR_SAVE_HEIGHT);
    let bytes_per_pixel = bytes_per_pixel_for_bpp(state.bpp);

    state.mouse_saved_x = x0_usize;
    state.mouse_saved_y = y0_usize;
    state.mouse_saved_w = width;
    state.mouse_saved_h = height;

    for row in 0..height {
        let src_row = state
            .front_ptr
            .add((y0_usize + row) * state.pitch_bytes + x0_usize * bytes_per_pixel);
        let saved_row = row * MOUSE_CURSOR_SAVE_WIDTH;
        for col in 0..width {
            state.mouse_saved_pixels[saved_row + col] =
                read_pixel(src_row.add(col * bytes_per_pixel) as *const u8, state.bpp);
        }
    }

    for row in 0..height {
        let dst_row = state
            .front_ptr
            .add((y0_usize + row) * state.pitch_bytes + x0_usize * bytes_per_pixel);
        let py = y0 + row as i32;
        for col in 0..width {
            let px = x0 + col as i32;
            let sprite_x = px - x;
            let sprite_y = py - y;

            let color = if cursor_mask_at_signed(sprite_x, sprite_y) {
                if cursor_mask_border(sprite_x as usize, sprite_y as usize) {
                    MOUSE_CURSOR_BORDER_COLOR
                } else {
                    MOUSE_CURSOR_FILL_COLOR
                }
            } else if cursor_mask_at_signed(px - shadow_x, py - shadow_y) {
                MOUSE_CURSOR_SHADOW_COLOR
            } else {
                continue;
            };
            write_pixel(dst_row.add(col * bytes_per_pixel), state.bpp, color);
        }
    }

    state.mouse_cursor_drawn = true;
}

#[inline]
fn cursor_mask_at(x: usize, y: usize) -> bool {
    if x >= MOUSE_CURSOR_WIDTH || y >= MOUSE_CURSOR_HEIGHT {
        return false;
    }
    let row = MOUSE_CURSOR_BITMAP[y];
    let mask = 1u16 << (MOUSE_CURSOR_WIDTH - 1 - x);
    (row & mask) != 0
}

#[inline]
fn cursor_mask_at_signed(x: i32, y: i32) -> bool {
    if x < 0 || y < 0 {
        return false;
    }
    cursor_mask_at(x as usize, y as usize)
}

#[inline]
fn cursor_mask_border(x: usize, y: usize) -> bool {
    if !cursor_mask_at(x, y) {
        return false;
    }

    if x == 0 || y == 0 || x + 1 >= MOUSE_CURSOR_WIDTH || y + 1 >= MOUSE_CURSOR_HEIGHT {
        return true;
    }

    !cursor_mask_at(x - 1, y)
        || !cursor_mask_at(x + 1, y)
        || !cursor_mask_at(x, y - 1)
        || !cursor_mask_at(x, y + 1)
}

#[inline]
unsafe fn write_pixel(dst: *mut u8, bpp: usize, rgb: u32) {
    let r = ((rgb >> 16) & 0xFF) as u8;
    let g = ((rgb >> 8) & 0xFF) as u8;
    let b = (rgb & 0xFF) as u8;

    match bpp {
        0..=15 => {
            let value = (((r as u16 >> 3) & 0x1F) << 10)
                | (((g as u16 >> 3) & 0x1F) << 5)
                | ((b as u16 >> 3) & 0x1F);
            (dst as *mut u16).write_volatile(value);
        }
        16..=23 => {
            let value = (((r as u16 >> 3) & 0x1F) << 11)
                | (((g as u16 >> 2) & 0x3F) << 5)
                | ((b as u16 >> 3) & 0x1F);
            (dst as *mut u16).write_volatile(value);
        }
        24..=31 => {
            dst.write_volatile(b);
            dst.add(1).write_volatile(g);
            dst.add(2).write_volatile(r);
        }
        _ => {
            let value = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
            (dst as *mut u32).write_volatile(value);
        }
    }
}

#[inline]
unsafe fn read_pixel(src: *const u8, bpp: usize) -> u32 {
    match bpp {
        0..=15 => {
            let value = (src as *const u16).read_volatile();
            let r = ((value >> 10) & 0x1F) as u32;
            let g = ((value >> 5) & 0x1F) as u32;
            let b = (value & 0x1F) as u32;
            ((r * 255 / 31) << 16) | ((g * 255 / 31) << 8) | (b * 255 / 31)
        }
        16..=23 => {
            let value = (src as *const u16).read_volatile();
            let r = ((value >> 11) & 0x1F) as u32;
            let g = ((value >> 5) & 0x3F) as u32;
            let b = (value & 0x1F) as u32;
            ((r * 255 / 31) << 16) | ((g * 255 / 63) << 8) | (b * 255 / 31)
        }
        24..=31 => {
            let b = src.read_volatile() as u32;
            let g = src.add(1).read_volatile() as u32;
            let r = src.add(2).read_volatile() as u32;
            (r << 16) | (g << 8) | b
        }
        _ => {
            let value = (src as *const u32).read_volatile();
            value & 0x00FF_FFFF
        }
    }
}

unsafe fn install_console(cols: usize, rows: usize) -> bool {
    let cols = cols.clamp(1, MAX_COLS);
    let rows = rows.clamp(MIN_ROWS, MAX_ROWS);
    let terminal_rows = rows - 1;

    let terminal_len = match cols.checked_mul(terminal_rows) {
        Some(value) => value,
        None => return false,
    };
    let scrollback_len = match cols.checked_mul(SCROLLBACK_ROWS) {
        Some(value) => value,
        None => return false,
    };

    let mut terminal = Vec::new();
    if terminal.try_reserve_exact(terminal_len).is_err() {
        return false;
    }
    terminal.resize(terminal_len, BLANK_CELL);

    let mut status = Vec::new();
    if status.try_reserve_exact(cols).is_err() {
        return false;
    }
    status.resize(cols, BLANK_CELL);

    let mut scrollback = Vec::new();
    if scrollback.try_reserve_exact(scrollback_len).is_err() {
        return false;
    }
    scrollback.resize(scrollback_len, BLANK_CELL);

    CONSOLE = Some(ConsoleState {
        cols,
        rows,
        terminal,
        status,
        scrollback,
        scrollback_head: 0,
        scrollback_count: 0,
        view_offset_rows: 0,
    });

    true
}

fn try_init_framebuffer() -> Option<(FramebufferState, usize, usize)> {
    let mapping = paging::framebuffer_mapping()?;
    if mapping.virtual_base == 0 || mapping.width == 0 || mapping.height == 0 {
        return None;
    }

    let font_height = mapping.font_height.max(1).min(32);

    let mut cols = mapping.width / FONT_WIDTH;
    let mut rows = mapping.height / font_height;
    if cols == 0 || rows < MIN_ROWS {
        return None;
    }

    cols = cols.min(MAX_COLS);
    rows = rows.min(MAX_ROWS);

    let surface_width = cols * FONT_WIDTH;
    let surface_height = rows * font_height;
    let terminal_rows = rows - 1;

    let pixels = surface_width.checked_mul(surface_height)?;
    let mut back = Vec::new();
    if back.try_reserve_exact(pixels).is_err() {
        return None;
    }
    back.resize(pixels, 0);

    let terminal_len = cols.checked_mul(terminal_rows)?;
    let mut live_terminal_cache = Vec::new();
    if live_terminal_cache.try_reserve_exact(terminal_len).is_err() {
        return None;
    }
    live_terminal_cache.resize(terminal_len, BLANK_CELL);

    let mut live_status_cache = Vec::new();
    if live_status_cache.try_reserve_exact(cols).is_err() {
        return None;
    }
    live_status_cache.resize(cols, BLANK_CELL);

    let font_ptr = if mapping.font_phys != 0 && mapping.font_bytes >= font_height {
        mapping.font_phys as *const u8
    } else {
        core::ptr::null()
    };

    Some((
        FramebufferState {
            front_ptr: mapping.virtual_base as *mut u8,
            surface_width,
            surface_height,
            pitch_bytes: mapping.pitch,
            bpp: mapping.bpp,
            font_ptr,
            font_bytes: mapping.font_bytes,
            font_height,
            back,
            live_terminal_cache,
            live_status_cache,
            live_cache_valid: false,
            cursor_drawn: false,
            last_cursor_row: 0,
            last_cursor_col: 0,
            mouse_cursor_visible: false,
            mouse_cursor_x: 0,
            mouse_cursor_y: 0,
            mouse_cursor_drawn: false,
            mouse_saved_x: 0,
            mouse_saved_y: 0,
            mouse_saved_w: 0,
            mouse_saved_h: 0,
            mouse_saved_pixels: [0; MOUSE_CURSOR_SAVE_WIDTH * MOUSE_CURSOR_SAVE_HEIGHT],
        },
        cols,
        rows,
    ))
}

pub fn print_str(s: &str) {
    ensure_initialized();
    leave_scrollback_view_if_needed();

    unsafe {
        for byte in s.bytes() {
            match byte {
                0x20..=0x7E | b'\n' | b'\r' | b'\x08' => put_char_internal(byte as char),
                _ => put_char_internal('?'),
            }
        }
        render_active_view_locked();
    }
}

pub fn print_u32(mut value: u32) {
    ensure_initialized();
    leave_scrollback_view_if_needed();

    unsafe {
        if value == 0 {
            put_char_internal('0');
            render_active_view_locked();
            return;
        }

        let mut digits = [0u8; 10];
        let mut idx = 0usize;

        while value > 0 {
            digits[idx] = (value % 10) as u8;
            value /= 10;
            idx += 1;
        }

        while idx > 0 {
            idx -= 1;
            put_char_internal((digits[idx] + b'0') as char);
        }

        render_active_view_locked();
    }
}

struct VgaWriter;

impl Write for VgaWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        print_str(s);
        Ok(())
    }
}

pub fn _print(args: fmt::Arguments<'_>) {
    let _ = VgaWriter.write_fmt(args);
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::vga::_print(format_args!($($arg)*));
    };
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::vga::_print(format_args!("\n"));
    };
    ($($arg:tt)*) => {
        $crate::vga::_print(format_args!($($arg)*));
        $crate::vga::_print(format_args!("\n"));
    };
}
