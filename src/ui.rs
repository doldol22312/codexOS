extern crate alloc;

use alloc::vec::Vec;

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

pub enum Widget {
    Panel(Panel),
    Label(Label),
    Button(Button),
}

impl Widget {
    pub fn id(&self) -> WidgetId {
        match self {
            Widget::Panel(panel) => panel.id,
            Widget::Label(label) => label.id,
            Widget::Button(button) => button.id,
        }
    }

    pub fn rect(&self) -> Rect {
        match self {
            Widget::Panel(panel) => panel.rect,
            Widget::Label(label) => label.rect,
            Widget::Button(button) => button.rect,
        }
    }

    pub fn is_focusable(&self) -> bool {
        matches!(self, Widget::Button(_))
    }

    pub fn draw(&self, focused: bool) {
        match self {
            Widget::Panel(panel) => panel.draw(focused),
            Widget::Label(label) => label.draw(focused),
            Widget::Button(button) => button.draw(focused),
        }
    }

    pub fn handle_event(&mut self, event: &InputEvent, focused: bool) -> WidgetResponse {
        match self {
            Widget::Panel(panel) => panel.handle_event(event, focused),
            Widget::Label(label) => label.handle_event(event, focused),
            Widget::Button(button) => button.handle_event(event, focused),
        }
    }

    pub fn update_hover_state(&mut self, x: i32, y: i32) -> bool {
        match self {
            Widget::Button(button) => button.set_hovered(button.rect.contains(x, y)),
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
}

impl EventDispatcher {
    pub fn new() -> Self {
        Self {
            widgets: Vec::new(),
            hit_regions: [EMPTY_REGION; MAX_WIDGETS],
            hit_count: 0,
            focused_id: None,
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

    pub fn draw(&self) {
        for widget in self.widgets.iter() {
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

        match event {
            InputEvent::MouseMove { x, y, .. } => {
                if self.update_hover_states(x, y) {
                    batch.redraw = true;
                }
                if let Some(target_id) = self.hit_widget_id(x, y) {
                    let response = self.dispatch_to_widget(target_id, event);
                    if response.redraw {
                        batch.redraw = true;
                    }
                    if response.clicked {
                        batch.clicked = Some(target_id);
                    }
                }
            }
            InputEvent::MouseDown { button, x, y } => {
                let target_id = self.hit_widget_id(x, y);
                if button == MouseButton::Left {
                    let next_focus = target_id.filter(|id| self.is_focusable(*id));
                    if self.set_focus(next_focus) {
                        batch.redraw = true;
                    }
                }

                if let Some(target_id) = target_id {
                    let response = self.dispatch_to_widget(target_id, event);
                    if response.redraw {
                        batch.redraw = true;
                    }
                    if response.clicked {
                        batch.clicked = Some(target_id);
                    }
                }
            }
            InputEvent::MouseUp { x, y, .. } | InputEvent::MouseClick { x, y, .. } => {
                if let Some(target_id) = self.hit_widget_id(x, y) {
                    let response = self.dispatch_to_widget(target_id, event);
                    if response.redraw {
                        batch.redraw = true;
                    }
                    if response.clicked {
                        batch.clicked = Some(target_id);
                    }
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
                    if response.redraw {
                        batch.redraw = true;
                    }
                    if response.clicked {
                        batch.clicked = Some(focused_id);
                    }
                }
            }
        }

        batch
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
            if self.hit_count >= MAX_WIDGETS {
                break;
            }
            self.hit_regions[self.hit_count] = widget.rect().as_hit_region(widget.id());
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

    fn is_focusable(&self, id: WidgetId) -> bool {
        self.widget_index_by_id(id)
            .is_some_and(|index| self.widgets[index].is_focusable())
    }
}
