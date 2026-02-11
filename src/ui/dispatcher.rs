use super::*;

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
