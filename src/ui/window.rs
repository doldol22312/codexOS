extern crate alloc;

use alloc::vec::Vec;
use core::cmp::min;

use super::Rect;
use crate::input::{InputEvent, MouseButton};
use crate::vga;

pub type WindowId = u16;

pub const MAX_WINDOWS: usize = 16;

const WINDOW_BORDER: i32 = 2;
const TITLE_BAR_HEIGHT: i32 = 24;
const MINIMIZED_HEIGHT: i32 = TITLE_BAR_HEIGHT + WINDOW_BORDER;
const RESIZE_GRAB_SIZE: i32 = 6;
const MIN_VISIBLE_TITLE_WIDTH: i32 = 40;
const MAX_RESIZE_FACTOR: i32 = 2;
const TITLE_TEXT_X_PAD: i32 = 8;
const BUTTON_SIZE: i32 = 14;
const BUTTON_GAP: i32 = 4;
const MAX_WINDOW_TITLE_CHARS: usize = 48;

const COLOR_FRAME: u32 = 0x3A4D66;
const COLOR_FRAME_FOCUSED: u32 = 0x39FF14;
const COLOR_TITLE: u32 = 0x233146;
const COLOR_TITLE_FOCUSED: u32 = 0x2D3F5D;
const COLOR_CLIENT_BACKGROUND: u32 = 0x101829;
const COLOR_TITLE_TEXT: u32 = 0xE8F1FF;
const COLOR_BUTTON_CLOSE: u32 = 0xB33A3A;
const COLOR_BUTTON_MINIMIZE: u32 = 0xAF8B2B;
const COLOR_BUTTON_MAXIMIZE: u32 = 0x2E8D5E;
const COLOR_BUTTON_PRESSED: u32 = 0x1B2638;
const COLOR_RESIZE_HANDLE: u32 = 0x5A708E;

#[derive(Clone, Copy, Debug)]
pub struct WindowSpec {
    pub title: &'static str,
    pub rect: Rect,
    pub min_width: i32,
    pub min_height: i32,
    pub background: u32,
    pub accent: u32,
}

impl WindowSpec {
    pub const fn new(title: &'static str, rect: Rect) -> Self {
        Self {
            title,
            rect,
            min_width: 160,
            min_height: 120,
            background: COLOR_CLIENT_BACKGROUND,
            accent: COLOR_FRAME_FOCUSED,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowEventResult {
    pub redraw: bool,
    pub closed: Option<WindowId>,
    pub focused: Option<WindowId>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowDebugSnapshot {
    pub cursor_window_id: Option<WindowId>,
    pub cursor_window_frame: Option<Rect>,
    pub cursor_window_client: Option<Rect>,
    pub focused_window_id: Option<WindowId>,
    pub focused_window_frame: Option<Rect>,
    pub resizing_window_id: Option<WindowId>,
    pub resizing_window_frame: Option<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DecorationButton {
    Close,
    Minimize,
    Maximize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ResizeEdges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

impl ResizeEdges {
    #[inline]
    fn any(self) -> bool {
        self.left || self.right || self.top || self.bottom
    }
}

#[derive(Clone, Copy, Debug)]
struct DragState {
    window_id: WindowId,
    offset_x: i32,
    offset_y: i32,
}

#[derive(Clone, Copy, Debug)]
struct ResizeState {
    window_id: WindowId,
    start_x: i32,
    start_y: i32,
    virtual_x: i32,
    virtual_y: i32,
    start_rect: Rect,
    edges: ResizeEdges,
}

#[derive(Clone, Copy, Debug)]
struct PressedDecoration {
    window_id: WindowId,
    button: DecorationButton,
}

#[derive(Clone, Copy, Debug)]
struct DecorationRects {
    close: Rect,
    minimize: Rect,
    maximize: Rect,
}

pub struct Window {
    id: WindowId,
    pub title: &'static str,
    pub rect: Rect,
    pub min_width: i32,
    pub min_height: i32,
    pub background: u32,
    pub accent: u32,
    minimized: bool,
    maximized: bool,
    restore_rect: Option<Rect>,
    client_width: usize,
    client_height: usize,
    client_pixels: Vec<u32>,
}

impl Window {
    fn new(id: WindowId, spec: WindowSpec) -> Self {
        let mut rect = spec.rect;
        rect.width = rect.width.max(spec.min_width.max(64));
        rect.height = rect.height.max(spec.min_height.max(MINIMIZED_HEIGHT + 1));

        let mut window = Self {
            id,
            title: spec.title,
            rect,
            min_width: spec.min_width.max(64),
            min_height: spec.min_height.max(MINIMIZED_HEIGHT + 1),
            background: spec.background,
            accent: spec.accent,
            minimized: false,
            maximized: false,
            restore_rect: None,
            client_width: 0,
            client_height: 0,
            client_pixels: Vec::new(),
        };
        let _ = window.refresh_client_buffer();
        window
    }

    #[inline]
    pub fn id(&self) -> WindowId {
        self.id
    }

    #[inline]
    pub fn minimized(&self) -> bool {
        self.minimized
    }

    #[inline]
    pub fn maximized(&self) -> bool {
        self.maximized
    }

    fn frame_rect(&self) -> Rect {
        let height = if self.minimized {
            MINIMIZED_HEIGHT
        } else {
            self.rect.height
        };
        Rect::new(self.rect.x, self.rect.y, self.rect.width, height.max(1))
    }

    fn title_rect(&self) -> Rect {
        Rect::new(
            self.rect.x + WINDOW_BORDER,
            self.rect.y + WINDOW_BORDER,
            (self.rect.width - WINDOW_BORDER * 2).max(0),
            (TITLE_BAR_HEIGHT - WINDOW_BORDER).max(0),
        )
    }

    fn client_rect(&self) -> Rect {
        if self.minimized {
            return Rect::new(0, 0, 0, 0);
        }
        Rect::new(
            self.rect.x + WINDOW_BORDER,
            self.rect.y + TITLE_BAR_HEIGHT,
            (self.rect.width - WINDOW_BORDER * 2).max(0),
            (self.rect.height - TITLE_BAR_HEIGHT - WINDOW_BORDER).max(0),
        )
    }

    fn decoration_rects(&self) -> DecorationRects {
        let title = self.title_rect();
        if title.width <= 0 || title.height <= 0 {
            return DecorationRects {
                close: Rect::new(0, 0, 0, 0),
                minimize: Rect::new(0, 0, 0, 0),
                maximize: Rect::new(0, 0, 0, 0),
            };
        }

        let y = title
            .y
            .saturating_add(((title.height - BUTTON_SIZE) / 2).max(0));
        let mut right = title
            .x
            .saturating_add(title.width)
            .saturating_sub(BUTTON_SIZE)
            .saturating_sub(2);

        let close = Rect::new(right, y, BUTTON_SIZE, BUTTON_SIZE);
        right = right.saturating_sub(BUTTON_SIZE + BUTTON_GAP);
        let maximize = Rect::new(right, y, BUTTON_SIZE, BUTTON_SIZE);
        right = right.saturating_sub(BUTTON_SIZE + BUTTON_GAP);
        let minimize = Rect::new(right, y, BUTTON_SIZE, BUTTON_SIZE);

        DecorationRects {
            close,
            minimize,
            maximize,
        }
    }

    fn hit_decoration_button(&self, x: i32, y: i32) -> Option<DecorationButton> {
        let buttons = self.decoration_rects();
        if buttons.close.contains(x, y) {
            Some(DecorationButton::Close)
        } else if buttons.minimize.contains(x, y) {
            Some(DecorationButton::Minimize)
        } else if buttons.maximize.contains(x, y) {
            Some(DecorationButton::Maximize)
        } else {
            None
        }
    }

    fn is_title_drag_hit(&self, x: i32, y: i32) -> bool {
        self.title_rect().contains(x, y) && self.hit_decoration_button(x, y).is_none()
    }

    fn resize_edges_at(&self, x: i32, y: i32) -> ResizeEdges {
        if self.minimized || self.maximized {
            return ResizeEdges::default();
        }

        let frame = self.frame_rect();
        if !frame.contains(x, y) {
            return ResizeEdges::default();
        }

        let left = x < frame.x.saturating_add(RESIZE_GRAB_SIZE);
        let right =
            x >= frame
                .x
                .saturating_add(frame.width)
                .saturating_sub(RESIZE_GRAB_SIZE);
        let top = y < frame.y.saturating_add(RESIZE_GRAB_SIZE);
        let bottom =
            y >= frame
                .y
                .saturating_add(frame.height)
                .saturating_sub(RESIZE_GRAB_SIZE);

        ResizeEdges {
            left,
            right,
            top,
            bottom,
        }
    }

    fn set_rect_without_refresh(&mut self, rect: Rect) -> bool {
        if self.rect == rect {
            return false;
        }
        self.rect = rect;
        true
    }

    fn set_rect(&mut self, rect: Rect) -> bool {
        if !self.set_rect_without_refresh(rect) {
            return false;
        }
        let _ = self.refresh_client_buffer();
        true
    }

    fn refresh_client_buffer(&mut self) -> bool {
        let client = self.client_rect();
        let new_w = client.width.max(0) as usize;
        let new_h = client.height.max(0) as usize;

        if new_w == self.client_width && new_h == self.client_height {
            return true;
        }

        let new_len = new_w.saturating_mul(new_h);
        let mut new_pixels = Vec::new();
        if new_pixels.try_reserve_exact(new_len).is_err() {
            return false;
        }
        new_pixels.resize(new_len, self.background);

        if self.client_width != 0 && self.client_height != 0 && !self.client_pixels.is_empty() {
            let copy_w = min(self.client_width, new_w);
            let copy_h = min(self.client_height, new_h);
            for row in 0..copy_h {
                let old_row = row * self.client_width;
                let new_row = row * new_w;
                new_pixels[new_row..new_row + copy_w]
                    .copy_from_slice(&self.client_pixels[old_row..old_row + copy_w]);
            }
        }

        self.client_width = new_w;
        self.client_height = new_h;
        self.client_pixels = new_pixels;
        true
    }
}

pub struct WindowManager {
    windows: Vec<Window>,
    focus_id: Option<WindowId>,
    dragging: Option<DragState>,
    resizing: Option<ResizeState>,
    pressed_decoration: Option<PressedDecoration>,
    desktop_color: u32,
    next_id: WindowId,
}

impl WindowManager {
    pub fn new(desktop_color: u32) -> Self {
        Self {
            windows: Vec::new(),
            focus_id: None,
            dragging: None,
            resizing: None,
            pressed_decoration: None,
            desktop_color,
            next_id: 1,
        }
    }

    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    pub fn focused_window(&self) -> Option<WindowId> {
        self.focus_id
    }

    pub fn debug_snapshot(&self, cursor_x: i32, cursor_y: i32) -> WindowDebugSnapshot {
        let cursor_window_id = self.top_window_at(cursor_x, cursor_y);
        let cursor_window_frame =
            cursor_window_id.and_then(|id| self.window_index(id).map(|index| self.windows[index].frame_rect()));
        let cursor_window_client =
            cursor_window_id.and_then(|id| self.window_index(id).map(|index| self.windows[index].client_rect()));

        let focused_window_id = self.focus_id;
        let focused_window_frame =
            focused_window_id.and_then(|id| self.window_index(id).map(|index| self.windows[index].frame_rect()));

        let resizing_window_id = self.resizing.map(|state| state.window_id);
        let resizing_window_frame =
            resizing_window_id.and_then(|id| self.window_index(id).map(|index| self.windows[index].frame_rect()));

        WindowDebugSnapshot {
            cursor_window_id,
            cursor_window_frame,
            cursor_window_client,
            focused_window_id,
            focused_window_frame,
            resizing_window_id,
            resizing_window_frame,
        }
    }

    pub fn add_window(&mut self, spec: WindowSpec) -> Result<WindowId, &'static str> {
        if self.windows.len() >= MAX_WINDOWS {
            return Err("window capacity reached");
        }

        let id = self.allocate_id().ok_or("window id space exhausted")?;
        let window = Window::new(id, spec);
        self.windows.push(window);
        self.focus_id = Some(id);
        Ok(id)
    }

    pub fn with_window_buffer_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut [u32], usize, usize) -> R,
    ) -> Option<R> {
        let index = self.window_index(id)?;
        let window = &mut self.windows[index];
        let width = window.client_width;
        let height = window.client_height;
        Some(f(window.client_pixels.as_mut_slice(), width, height))
    }

    pub fn clear_window_buffer(&mut self, id: WindowId, color: u32) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        let window = &mut self.windows[index];
        for pixel in window.client_pixels.iter_mut() {
            *pixel = color;
        }
        true
    }

    pub fn compose(&self, desktop_bounds: Rect) {
        if desktop_bounds.width <= 0 || desktop_bounds.height <= 0 {
            return;
        }

        vga::begin_draw_batch();
        let _ = vga::draw_filled_rect(
            desktop_bounds.x,
            desktop_bounds.y,
            desktop_bounds.width,
            desktop_bounds.height,
            self.desktop_color,
        );

        for window in self.windows.iter() {
            self.draw_window(window);
        }
        vga::end_draw_batch();
    }

    pub fn handle_event(&mut self, event: InputEvent, desktop_bounds: Rect) -> WindowEventResult {
        let mut result = WindowEventResult::default();

        match event {
            InputEvent::MouseDown {
                button: MouseButton::Left,
                x,
                y,
            } => {
                self.dragging = None;
                self.resizing = None;
                self.pressed_decoration = None;

                let hit_id = self.top_window_at(x, y);
                if let Some(window_id) = hit_id {
                    if self.bring_to_front(window_id) {
                        result.redraw = true;
                    }
                    if self.set_focus(Some(window_id)) {
                        result.redraw = true;
                    }

                    let Some(index) = self.window_index(window_id) else {
                        result.focused = self.focus_id;
                        return result;
                    };
                    let window = &self.windows[index];

                    if let Some(button) = window.hit_decoration_button(x, y) {
                        self.pressed_decoration = Some(PressedDecoration { window_id, button });
                        result.redraw = true;
                        result.focused = self.focus_id;
                        return result;
                    }

                    let resize_edges = window.resize_edges_at(x, y);
                    if resize_edges.any() {
                        self.resizing = Some(ResizeState {
                            window_id,
                            start_x: x,
                            start_y: y,
                            virtual_x: x,
                            virtual_y: y,
                            start_rect: window.rect,
                            edges: resize_edges,
                        });
                        result.focused = self.focus_id;
                        return result;
                    }

                    if window.is_title_drag_hit(x, y) {
                        self.dragging = Some(DragState {
                            window_id,
                            offset_x: x.saturating_sub(window.rect.x),
                            offset_y: y.saturating_sub(window.rect.y),
                        });
                    }
                } else if self.set_focus(None) {
                    result.redraw = true;
                }
            }
            InputEvent::MouseMove { x, y, dx, dy } => {
                if let Some(drag) = self.dragging {
                    if self.apply_drag(drag, x, y, desktop_bounds) {
                        result.redraw = true;
                    }
                } else if let Some(mut resize) = self.resizing {
                    resize.virtual_x = resize.virtual_x.saturating_add(dx);
                    resize.virtual_y = resize.virtual_y.saturating_add(dy);
                    if self.apply_resize(resize, resize.virtual_x, resize.virtual_y, desktop_bounds) {
                        result.redraw = true;
                    }
                    self.resizing = Some(resize);
                }
            }
            InputEvent::MouseUp {
                button: MouseButton::Left,
                x,
                y,
            } => {
                if self.dragging.take().is_some() {
                    result.redraw = true;
                }
                if let Some(resize) = self.resizing.take() {
                    self.finalize_resize(resize);
                    result.redraw = true;
                }

                if let Some(pressed) = self.pressed_decoration.take() {
                    result.redraw = true;
                    if self.decoration_still_hit(pressed, x, y) {
                        self.activate_decoration(pressed, desktop_bounds, &mut result);
                    }
                }
            }
            _ => {}
        }

        result.focused = self.focus_id;
        result
    }

    fn draw_window(&self, window: &Window) {
        let focused = self.focus_id == Some(window.id);
        let frame = window.frame_rect();
        if frame.width <= 0 || frame.height <= 0 {
            return;
        }

        let frame_color = if focused { window.accent } else { COLOR_FRAME };
        let _ = vga::draw_filled_rect(frame.x, frame.y, frame.width, frame.height, frame_color);

        let title = window.title_rect();
        let title_color = if focused {
            COLOR_TITLE_FOCUSED
        } else {
            COLOR_TITLE
        };
        let _ = vga::draw_filled_rect(title.x, title.y, title.width, title.height, title_color);

        let decorations = window.decoration_rects();
        let pressed_button = self
            .pressed_decoration
            .filter(|state| state.window_id == window.id)
            .map(|state| state.button);

        self.draw_title_button(
            decorations.close,
            COLOR_BUTTON_CLOSE,
            pressed_button == Some(DecorationButton::Close),
        );
        self.draw_title_button(
            decorations.minimize,
            COLOR_BUTTON_MINIMIZE,
            pressed_button == Some(DecorationButton::Minimize),
        );
        self.draw_title_button(
            decorations.maximize,
            COLOR_BUTTON_MAXIMIZE,
            pressed_button == Some(DecorationButton::Maximize),
        );

        self.draw_close_icon(decorations.close);
        self.draw_minimize_icon(decorations.minimize);
        self.draw_maximize_icon(decorations.maximize);

        let mut title_buffer = [0u8; MAX_WINDOW_TITLE_CHARS];
        let mut title_len = 0usize;
        for byte in window.title.bytes() {
            if title_len >= title_buffer.len() {
                break;
            }
            title_buffer[title_len] = byte;
            title_len += 1;
        }
        if let Ok(title_text) = core::str::from_utf8(&title_buffer[..title_len]) {
            let text_x = title.x.saturating_add(TITLE_TEXT_X_PAD);
            let text_y = title.y.saturating_add(4);
            let _ = vga::draw_text(text_x, text_y, title_text, COLOR_TITLE_TEXT, title_color);
        }

        if !window.minimized {
            let client = window.client_rect();
            let _ = vga::draw_filled_rect(
                client.x,
                client.y,
                client.width,
                client.height,
                window.background,
            );
            let copy_w = min(window.client_width, client.width.max(0) as usize);
            let copy_h = min(window.client_height, client.height.max(0) as usize);
            if client.width > 0
                && client.height > 0
                && copy_w > 0
                && copy_h > 0
            {
                let _ = vga::blit_bitmap(
                    client.x,
                    client.y,
                    &window.client_pixels,
                    copy_w,
                    copy_h,
                    window.client_width,
                );
            }

            if !window.maximized {
                self.draw_resize_handles(frame);
            }
        }

        let _ = vga::draw_horizontal_line(frame.x, frame.y, frame.width, frame_color);
        let _ = vga::draw_horizontal_line(
            frame.x,
            frame.y.saturating_add(frame.height).saturating_sub(1),
            frame.width,
            frame_color,
        );
        let _ = vga::draw_vertical_line(frame.x, frame.y, frame.height, frame_color);
        let _ = vga::draw_vertical_line(
            frame.x.saturating_add(frame.width).saturating_sub(1),
            frame.y,
            frame.height,
            frame_color,
        );
    }

    fn draw_resize_handles(&self, frame: Rect) {
        let span = 10;
        let right = frame.x.saturating_add(frame.width).saturating_sub(1);
        let bottom = frame.y.saturating_add(frame.height).saturating_sub(1);

        let _ = vga::draw_horizontal_line(frame.x + 2, frame.y + 2, span, COLOR_RESIZE_HANDLE);
        let _ = vga::draw_vertical_line(frame.x + 2, frame.y + 2, span, COLOR_RESIZE_HANDLE);

        let _ = vga::draw_horizontal_line(
            right.saturating_sub(span).saturating_sub(1),
            frame.y + 2,
            span,
            COLOR_RESIZE_HANDLE,
        );
        let _ = vga::draw_vertical_line(right - 2, frame.y + 2, span, COLOR_RESIZE_HANDLE);

        let _ = vga::draw_horizontal_line(
            frame.x + 2,
            bottom.saturating_sub(2),
            span,
            COLOR_RESIZE_HANDLE,
        );
        let _ = vga::draw_vertical_line(
            frame.x + 2,
            bottom.saturating_sub(span).saturating_sub(1),
            span,
            COLOR_RESIZE_HANDLE,
        );

        let _ = vga::draw_horizontal_line(
            right.saturating_sub(span).saturating_sub(1),
            bottom.saturating_sub(2),
            span,
            COLOR_RESIZE_HANDLE,
        );
        let _ = vga::draw_vertical_line(
            right - 2,
            bottom.saturating_sub(span).saturating_sub(1),
            span,
            COLOR_RESIZE_HANDLE,
        );
    }

    fn draw_title_button(&self, rect: Rect, color: u32, pressed: bool) {
        if rect.width <= 0 || rect.height <= 0 {
            return;
        }
        let fill = if pressed { COLOR_BUTTON_PRESSED } else { color };
        let _ = vga::draw_filled_rect(rect.x, rect.y, rect.width, rect.height, fill);
        let border = if pressed { color } else { COLOR_FRAME };
        let _ = vga::draw_horizontal_line(rect.x, rect.y, rect.width, border);
        let _ = vga::draw_horizontal_line(
            rect.x,
            rect.y.saturating_add(rect.height).saturating_sub(1),
            rect.width,
            border,
        );
        let _ = vga::draw_vertical_line(rect.x, rect.y, rect.height, border);
        let _ = vga::draw_vertical_line(
            rect.x.saturating_add(rect.width).saturating_sub(1),
            rect.y,
            rect.height,
            border,
        );
    }

    fn draw_close_icon(&self, rect: Rect) {
        if rect.width < 6 || rect.height < 6 {
            return;
        }
        let x0 = rect.x + 3;
        let y0 = rect.y + 3;
        let x1 = rect.x + rect.width - 4;
        let y1 = rect.y + rect.height - 4;
        let _ = vga::draw_line(x0, y0, x1, y1, COLOR_TITLE_TEXT);
        let _ = vga::draw_line(x0, y1, x1, y0, COLOR_TITLE_TEXT);
    }

    fn draw_minimize_icon(&self, rect: Rect) {
        if rect.width < 6 || rect.height < 6 {
            return;
        }
        let y = rect.y + rect.height - 4;
        let _ = vga::draw_horizontal_line(rect.x + 3, y, rect.width - 6, COLOR_TITLE_TEXT);
    }

    fn draw_maximize_icon(&self, rect: Rect) {
        if rect.width < 8 || rect.height < 8 {
            return;
        }
        let x = rect.x + 3;
        let y = rect.y + 3;
        let w = rect.width - 6;
        let h = rect.height - 6;
        let _ = vga::draw_horizontal_line(x, y, w, COLOR_TITLE_TEXT);
        let _ = vga::draw_horizontal_line(x, y + h - 1, w, COLOR_TITLE_TEXT);
        let _ = vga::draw_vertical_line(x, y, h, COLOR_TITLE_TEXT);
        let _ = vga::draw_vertical_line(x + w - 1, y, h, COLOR_TITLE_TEXT);
    }

    fn set_focus(&mut self, focus: Option<WindowId>) -> bool {
        if self.focus_id == focus {
            return false;
        }
        self.focus_id = focus;
        true
    }

    fn allocate_id(&mut self) -> Option<WindowId> {
        for _ in 0..u16::MAX {
            let candidate = self.next_id;
            self.next_id = self.next_id.wrapping_add(1).max(1);
            if self.window_index(candidate).is_none() {
                return Some(candidate);
            }
        }
        None
    }

    fn top_window_at(&self, x: i32, y: i32) -> Option<WindowId> {
        for window in self.windows.iter().rev() {
            if window.frame_rect().contains(x, y) {
                return Some(window.id());
            }
        }
        None
    }

    fn bring_to_front(&mut self, id: WindowId) -> bool {
        let Some(index) = self.window_index(id) else {
            return false;
        };
        if index + 1 == self.windows.len() {
            return false;
        }

        let window = self.windows.remove(index);
        self.windows.push(window);
        true
    }

    fn window_index(&self, id: WindowId) -> Option<usize> {
        for (index, window) in self.windows.iter().enumerate() {
            if window.id == id {
                return Some(index);
            }
        }
        None
    }

    fn apply_drag(&mut self, drag: DragState, x: i32, y: i32, desktop_bounds: Rect) -> bool {
        let Some(index) = self.window_index(drag.window_id) else {
            return false;
        };
        if self.windows[index].maximized {
            return false;
        }

        let window_width = self.windows[index].rect.width.max(1);
        let title_height = TITLE_BAR_HEIGHT.max(1);
        let mut next_x = x.saturating_sub(drag.offset_x);
        let mut next_y = y.saturating_sub(drag.offset_y);

        let min_x = desktop_bounds
            .x
            .saturating_sub(window_width)
            .saturating_add(40);
        let max_x = desktop_bounds
            .x
            .saturating_add(desktop_bounds.width)
            .saturating_sub(40);
        let min_y = desktop_bounds.y;
        let max_y = desktop_bounds
            .y
            .saturating_add(desktop_bounds.height)
            .saturating_sub(title_height);

        next_x = next_x.clamp(min_x, max_x);
        next_y = next_y.clamp(min_y, max_y);

        let mut next = self.windows[index].rect;
        next.x = next_x;
        next.y = next_y;
        self.windows[index].set_rect(next)
    }

    fn apply_resize(
        &mut self,
        resize: ResizeState,
        x: i32,
        y: i32,
        desktop_bounds: Rect,
    ) -> bool {
        let Some(index) = self.window_index(resize.window_id) else {
            return false;
        };
        let window = &self.windows[index];
        if window.maximized || window.minimized {
            return false;
        }

        let dx = x.saturating_sub(resize.start_x);
        let dy = y.saturating_sub(resize.start_y);
        let mut rect = resize.start_rect;

        if resize.edges.left {
            rect.x = rect.x.saturating_add(dx);
            rect.width = rect.width.saturating_sub(dx);
        }
        if resize.edges.right {
            rect.width = rect.width.saturating_add(dx);
        }
        if resize.edges.top {
            rect.y = rect.y.saturating_add(dy);
            rect.height = rect.height.saturating_sub(dy);
        }
        if resize.edges.bottom {
            rect.height = rect.height.saturating_add(dy);
        }

        let min_w = window.min_width.max(64);
        let min_h = window.min_height.max(MINIMIZED_HEIGHT + 1);
        if rect.width < min_w {
            if resize.edges.left {
                rect.x = rect.x.saturating_sub(min_w - rect.width);
            }
            rect.width = min_w;
        }
        if rect.height < min_h {
            if resize.edges.top {
                rect.y = rect.y.saturating_sub(min_h - rect.height);
            }
            rect.height = min_h;
        }

        let max_w = desktop_bounds
            .width
            .saturating_mul(MAX_RESIZE_FACTOR)
            .max(min_w);
        let max_h = desktop_bounds
            .height
            .saturating_mul(MAX_RESIZE_FACTOR)
            .max(min_h);
        if rect.width > max_w {
            if resize.edges.left {
                rect.x = rect.x.saturating_add(rect.width - max_w);
            }
            rect.width = max_w;
        }
        if rect.height > max_h {
            if resize.edges.top {
                rect.y = rect.y.saturating_add(rect.height - max_h);
            }
            rect.height = max_h;
        }

        let desktop_right = desktop_bounds.x.saturating_add(desktop_bounds.width);
        let desktop_bottom = desktop_bounds.y.saturating_add(desktop_bounds.height);
        let min_x = desktop_bounds
            .x
            .saturating_sub(rect.width.saturating_sub(MIN_VISIBLE_TITLE_WIDTH));
        let max_x = desktop_right.saturating_sub(MIN_VISIBLE_TITLE_WIDTH);
        let min_y = desktop_bounds.y;
        let max_y = desktop_bottom.saturating_sub(TITLE_BAR_HEIGHT.max(1));

        rect.x = rect.x.clamp(min_x, max_x);
        rect.y = rect.y.clamp(min_y, max_y);
        rect.width = rect.width.max(min_w).min(max_w);
        rect.height = rect.height.max(min_h).min(max_h);
        self.windows[index].set_rect_without_refresh(rect)
    }

    fn finalize_resize(&mut self, resize: ResizeState) {
        let Some(index) = self.window_index(resize.window_id) else {
            return;
        };

        if self.windows[index].refresh_client_buffer() {
            return;
        }

        // Retry once after explicitly releasing the previous backing store.
        // This avoids transient old+new peak memory pressure during large resizes.
        self.windows[index].client_pixels.clear();
        self.windows[index].client_pixels.shrink_to(0);
        if self.windows[index].refresh_client_buffer() {
            return;
        }

        // If final resize allocation fails, revert to the last known-good rect
        // (the geometry at resize start) so the window does not remain expanded
        // with an unbacked client area.
        self.windows[index].set_rect_without_refresh(resize.start_rect);
    }

    fn decoration_still_hit(&self, pressed: PressedDecoration, x: i32, y: i32) -> bool {
        let Some(index) = self.window_index(pressed.window_id) else {
            return false;
        };
        self.windows[index].hit_decoration_button(x, y) == Some(pressed.button)
    }

    fn activate_decoration(
        &mut self,
        pressed: PressedDecoration,
        desktop_bounds: Rect,
        result: &mut WindowEventResult,
    ) {
        let Some(index) = self.window_index(pressed.window_id) else {
            return;
        };

        match pressed.button {
            DecorationButton::Close => {
                let removed = self.windows.remove(index);
                result.closed = Some(removed.id);

                if self.focus_id == Some(removed.id) {
                    self.focus_id = self.windows.last().map(|window| window.id);
                }
                self.dragging = None;
                self.resizing = None;
                self.pressed_decoration = None;
            }
            DecorationButton::Minimize => {
                let window = &mut self.windows[index];
                window.minimized = !window.minimized;
                if window.minimized {
                    self.dragging = None;
                    self.resizing = None;
                }
            }
            DecorationButton::Maximize => {
                let window = &mut self.windows[index];
                if window.maximized {
                    if let Some(previous) = window.restore_rect.take() {
                        window.maximized = false;
                        window.set_rect(previous);
                    }
                } else {
                    window.restore_rect = Some(window.rect);
                    window.maximized = true;
                    window.minimized = false;
                    let max_rect = maximized_rect(desktop_bounds, window.min_width, window.min_height);
                    window.set_rect(max_rect);
                }
            }
        }
    }
}

fn maximized_rect(desktop: Rect, min_width: i32, min_height: i32) -> Rect {
    let margin = 8;
    let x = desktop.x.saturating_add(margin);
    let y = desktop.y.saturating_add(margin);
    let width = desktop
        .width
        .saturating_sub(margin * 2)
        .max(min_width.max(64));
    let height = desktop
        .height
        .saturating_sub(margin * 2)
        .max(min_height.max(MINIMIZED_HEIGHT + 1));

    Rect::new(x, y, width, height)
}
