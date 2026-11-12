// SPDX-License-Identifier: MIT

//! View-state behavior shared by every mode: pointer ownership, the keyboard
//! destination and its styling, and global activity.

use super::*;

fn set_css_class(widget: &impl IsA<gtk::Widget>, class: &str, enabled: bool) {
    if enabled {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

fn mark_keyboard_cursor(column: &ColumnView, cursor: Option<u32>) {
    column.bound_rows.borrow_mut().retain(|bound| {
        let (Some(item), Some(row)) = (bound.item.upgrade(), bound.row.upgrade()) else {
            return false;
        };
        set_css_class(&row, "keyboard-cursor", cursor == Some(item.position()));
        true
    });
}

impl ViewState {
    /// Returns focus to the listing after an overlay closes: the cursor row, or the
    /// pane surface of an empty or loading directory.
    pub(in crate::ui) fn restore_listing_focus(&self) {
        if self.mode.get() == BrowserMode::Columns {
            self.focus_rebuilt_active_column();
            return;
        }
        if self.mode_views.borrow().focus_search_results() {
            return;
        }
        self.browser.focus_active();
        let focused = self.overlay.root().and_then(|root| root.focus());
        if !focused.is_some_and(|focused| focused.is_mapped()) {
            self.mode_views.borrow().focus_pane_surface();
        }
    }

    pub(in crate::ui) fn notify_search_selection_changed(&self) {
        let handlers = self.search_selection_handlers.borrow().clone();
        for handler in handlers {
            handler();
        }
    }

    pub(in crate::ui::browser) fn stop_drag_autoscroll(&self) {
        if let Some(tracker) = self.drag_autoscroll.borrow().as_ref() {
            tracker.stop();
        }
    }

    pub(in crate::ui) fn cancel_peek(&self) {
        cancel_source(&self.pending_peek);
        self.browser.close_peek();
    }

    pub(super) fn begin_global_activity(
        self: &Rc<Self>,
        label: impl Into<String>,
    ) -> GlobalActivity {
        let label = label.into();
        let id = self.global_activity.borrow_mut().begin(label.clone());
        crate::ui::accessibility::set_description(&self.global_activity_spinner, Some(&label));
        self.global_activity_spinner.set_visible(true);
        self.global_activity_spinner.start();
        GlobalActivity {
            state: Rc::downgrade(self),
            id,
        }
    }

    pub(super) fn finish_global_activity(&self, id: u64) {
        let current = {
            let mut activity = self.global_activity.borrow_mut();
            activity.finish(id);
            activity.current_label().map(str::to_owned)
        };
        if let Some(label) = current {
            crate::ui::accessibility::set_description(&self.global_activity_spinner, Some(&label));
        } else {
            self.global_activity_spinner.stop();
            self.global_activity_spinner.set_visible(false);
            crate::ui::accessibility::set_description(
                &self.global_activity_spinner,
                Some(&crate::i18n::tr("Working…")),
            );
        }
    }

    pub(super) fn enter_mode(self: &Rc<Self>, mode: BrowserMode) {
        self.mode.set(mode);
        let scope = match mode {
            BrowserMode::Columns => VisibleDepths::All,
            BrowserMode::Icons | BrowserMode::List | BrowserMode::Compact => VisibleDepths::Active,
        };
        self.browser.set_freshness(FreshnessInput {
            scope,
            ..self.browser.freshness_input()
        });
    }

    /// A reload rebinds the rows an inline edit leases, which would discard the typed
    /// name, so window commands and background re-lists wait while this holds.
    pub(in crate::ui) fn inline_edit_is_open(&self) -> bool {
        self.pending_new_entry.borrow().is_some()
            || self.active_rename.borrow().is_some()
            || self.mode_views.borrow().rename_is_active()
    }

    pub(super) fn refresh_browser(&self) {
        if self.mode.get() == BrowserMode::Columns {
            self.browser.refresh_all();
        } else {
            self.browser.reload_active();
        }
    }

    pub(super) fn sync_mode_selection(&self) {
        if self.mode.get() == BrowserMode::Columns {
            if let Some(depth) = self.focused_column_depth() {
                self.browser.set_active_column(depth);
            }
            return;
        }
        let Some((depth, positions)) = self.mode_views.borrow().selected_positions() else {
            return;
        };
        let focused = self
            .mode_views
            .borrow()
            .focused_position()
            .filter(|(focused_depth, _)| *focused_depth == depth)
            .map(|(_, position)| position)
            .or_else(|| positions.last().copied());
        self.browser.set_selection(depth, &positions, focused);
    }

    pub(super) fn install_input_ownership(self: &Rc<Self>) {
        self.overlay.add_css_class("keyboard-navigation");
        let motion = gtk::EventControllerMotion::new();
        motion.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        motion.connect_motion(move |controller, x, y| {
            let Some(state) = weak.upgrade() else { return };
            let Some(event) = controller
                .current_event()
                .filter(|event| event.event_type() == gtk::gdk::EventType::MotionNotify)
            else {
                return;
            };
            let Some(position) = event.position() else {
                return;
            };
            // Only GTK's synthesized motion lacks an event time.
            if event.time() == gtk::gdk::CURRENT_TIME {
                state
                    .input_ownership
                    .borrow_mut()
                    .pointer_resynced(position);
                return;
            }
            let hovered = state.column_depth_at(x, y);
            BrowserView { state }.record_pointer_hover(position, hovered);
        });
        self.overlay.add_controller(motion);
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |_, _, x, y| {
            if let Some(state) = weak.upgrade() {
                state.hovered_column.set(state.column_depth_at(x, y));
                state.pointer_navigation();
            }
        });
        self.overlay.add_controller(click);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        scroll.connect_scroll(move |_, _, _| {
            if let Some(state) = weak.upgrade() {
                state.pointer_navigation();
            }
            glib::Propagation::Proceed
        });
        self.overlay.add_controller(scroll);
    }

    pub(super) fn column_depth_at(&self, x: f64, y: f64) -> Option<usize> {
        let picked = self.overlay.pick(x, y, gtk::PickFlags::DEFAULT)?;
        self.columns.borrow().iter().position(|column| {
            picked == column.shell.upcast_ref::<gtk::Widget>().clone()
                || picked.is_ancestor(&column.shell)
        })
    }

    pub(super) fn adopt_pointer(&self, hovered: Option<usize>) {
        self.hovered_column.set(hovered);
        self.overlay.remove_css_class("keyboard-navigation");
        self.refresh_destination_style();
    }

    pub(super) fn pointer_navigation(&self) {
        self.input_ownership.borrow_mut().pointer_action();
        self.adopt_pointer(self.hovered_column.get());
    }

    pub(super) fn destination_depth(&self) -> Option<usize> {
        if self.mode.get() != BrowserMode::Columns {
            return self.browser.active_depth();
        }
        if let Some(depth) = self.context_menu_column.get()
            && depth < self.columns.borrow().len()
        {
            return Some(depth);
        }
        self.input_ownership.borrow().destination(
            self.hovered_column.get(),
            self.focused_column_depth(),
            self.browser.active_depth(),
            self.columns.borrow().len(),
        )
    }

    pub(super) fn refresh_destination_style(&self) {
        let destination = if self.preview_owns_keys.get() {
            None
        } else {
            self.destination_depth()
        };
        let focused_column = self.focused_column_depth();
        let focused_item = self
            .browser
            .focused_item()
            .map(|(depth, position, _)| (depth, position));
        for (depth, column) in self.columns.borrow().iter().enumerate() {
            let show_actions = destination == Some(depth);
            self.show_column_actions(column, show_actions);
            let cursor = focused_item
                .filter(|(item_depth, _)| *item_depth == depth)
                .filter(|_| focused_column == Some(depth))
                .and_then(|(_, position)| column.map.view_position(position));
            mark_keyboard_cursor(column, cursor);
            let active = show_actions && self.accepts_drops_at(depth);
            set_css_class(&column.shell, "destination-column", active);
        }
    }

    fn show_column_actions(&self, column: &ColumnView, show_actions: bool) {
        set_css_class(&column.shell, "active-column", show_actions);
        let header_focused = self
            .overlay
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focused| {
                focused == *column.header_actions.upcast_ref::<gtk::Widget>()
                    || focused.is_ancestor(&column.header_actions)
            });
        if !show_actions && header_focused {
            // Do not leave keyboard focus inside controls hidden by pointer navigation.
            column.focus_surface();
        }
        column
            .header_actions_stack
            .set_visible_child_name(if show_actions { "actions" } else { "hidden" });
    }

    /// Trash and Recent take no drops, so they never look like a destination.
    fn accepts_drops_at(&self, depth: usize) -> bool {
        self.browser
            .location_at(depth)
            .is_some_and(|location| !is_trash_location(&location) && !location.is_recent_location())
    }

    pub(super) fn focused_column_depth(&self) -> Option<usize> {
        let focused = self.overlay.root()?.focus()?;
        self.columns.borrow().iter().position(|column| {
            focused == column.shell.clone().upcast::<gtk::Widget>()
                || focused.is_ancestor(&column.shell)
        })
    }

    /// Like `ModeViews::listing_may_take_focus`, for any column.
    pub(super) fn column_may_take_focus(&self) -> bool {
        self.overlay
            .root()
            .and_then(|root| root.focus())
            .is_none_or(|focused| focused.root().is_none())
            || self.focused_column_depth().is_some()
    }

    pub(super) fn select_all(&self, depth: usize) {
        if self.mode.get() != BrowserMode::Columns {
            self.browser.select_all(depth);
            return;
        }
        if let Some(column) = self.columns.borrow().get(depth) {
            column.selection.select_all();
            column.focus_surface();
        }
    }
}
