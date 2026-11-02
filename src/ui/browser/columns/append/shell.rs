// SPDX-License-Identifier: MIT

//! A column's outer shell: width, filter blur, drop veil, resize line and peek target.

use super::super::resize::{cap_column_width, column_width_cap, initial_column_width};
use crate::model::Location;
use crate::ui::browser::ViewState;
use gtk::glib;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::{Rc, Weak};

/// The shell and the width state that travels with its column.
pub(super) struct ColumnShell {
    pub(super) shell: gtk::Box,
    pub(super) resize_handle: gtk::Box,
    pub(super) reveal_button: gtk::Button,
    pub(super) animation_generation: Rc<Cell<u64>>,
    pub(super) unscaled_width: Rc<Cell<f64>>,
}

/// Leaving the column for another column, not a popover or dialog, closes its filter.
fn close_filter_after_blur(
    widget: &gtk::Widget,
    shell: &gtk::Box,
    filter_button: &gtk::ToggleButton,
    browser: &Weak<crate::app::Browser>,
) {
    let Some(root) = widget.root() else {
        return;
    };
    // Result dialogs must retain the query for restoration after dismissal.
    if root
        .downcast_ref::<gtk::Window>()
        .is_some_and(|window| crate::ui::window::visible_modal_layer(window).is_some())
    {
        return;
    }
    let left_column = root.focus().is_some_and(|focused| {
        !focused.is_ancestor(shell)
            && focused != shell.clone().upcast::<gtk::Widget>()
            && focused.ancestor(gtk::Popover::static_type()).is_none()
    });
    // 10xer hides the filter with its state kept for leaving, so a focus
    // move while it is hidden must not switch the filter off.
    let keeps_filter = crate::ui::tenxer_mode::chrome_suppressed()
        || browser
            .upgrade()
            .is_none_or(|browser| browser.is_chooser_mode());
    if left_column && !keeps_filter {
        filter_button.set_active(false);
    }
}

/// Focus may be unset during transfer; inspect its destination at idle.
fn install_filter_blur(state: &ViewState, shell: &gtk::Box, filter_button: &gtk::ToggleButton) {
    let filter_button_for_blur = filter_button.clone();
    let shell_for_blur = shell.downgrade();
    let weak_browser_for_blur = Rc::downgrade(&state.browser);
    let filter_focus = gtk::EventControllerFocus::new();
    filter_focus.connect_leave(move |controller| {
        let Some(widget) = controller.widget() else {
            return;
        };
        let shell_for_blur = shell_for_blur.clone();
        let filter_button_for_blur = filter_button_for_blur.clone();
        let weak_browser = weak_browser_for_blur.clone();
        glib::idle_add_local_once(move || {
            let Some(shell) = shell_for_blur.upgrade() else {
                return;
            };
            close_filter_after_blur(&widget, &shell, &filter_button_for_blur, &weak_browser);
        });
    });
    shell.add_controller(filter_focus);
}

/// Also called for unrelated changes; those must not narrow a column
/// after the window shrank.
fn rescale_column(state: &ViewState, shell: &gtk::Widget, width: i32) {
    if state.column_width_limit.get().is_some() {
        // The sidebar scales too, so the published area is stale until the
        // next frame; that limit narrows the column once.
        shell.set_width_request(width);
        state.clamp_to_next_column_limit.set(true);
    } else {
        shell.set_width_request(cap_column_width(width, column_width_cap(state)));
    }
}

fn column_drop_veil() -> gtk::Box {
    let drop_veil = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    drop_veil.add_css_class("column-drop-veil");
    drop_veil.set_can_target(false);
    drop_veil
}

fn column_resize_handle() -> gtk::Box {
    let resize_handle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    resize_handle.add_css_class("column-resize-handle");
    resize_handle.set_width_request(1);
    resize_handle.set_can_target(false);
    resize_handle.set_halign(gtk::Align::End);
    resize_handle.set_valign(gtk::Align::Fill);
    resize_handle
}

fn column_reveal_button(location: &Location) -> gtk::Button {
    let reveal_button = gtk::Button::new();
    reveal_button.add_css_class("column-peek-target");
    reveal_button.set_focusable(false);
    reveal_button.set_focus_on_click(false);
    // Let row drag sources receive presses through the peek overlay.
    reveal_button.set_can_target(false);
    reveal_button.set_cursor_from_name(Some("pointer"));
    reveal_button.set_visible(false);
    crate::ui::accessibility::set_label(
        &reveal_button,
        &rust_i18n::t!("Reveal %{value1} column", value1 = location.display_name()),
    );
    reveal_button.set_tooltip_text(Some(&rust_i18n::t!(
        "Reveal %{value1}",
        value1 = location.display_path()
    )));
    reveal_button
}

impl ViewState {
    /// Tracks the pointer over `widget` as hovering the column at `depth`.
    pub(super) fn install_column_hover(
        self: &Rc<Self>,
        widget: &impl IsA<gtk::Widget>,
        depth: usize,
    ) {
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        motion.connect_enter(move |_, _, _| {
            if let Some(state) = weak.upgrade() {
                state.hovered_column.set(Some(depth));
                state.refresh_destination_style();
            }
        });
        let weak = Rc::downgrade(self);
        motion.connect_leave(move |_| {
            if let Some(state) = weak.upgrade()
                && state.hovered_column.get() == Some(depth)
            {
                state.hovered_column.set(None);
                state.refresh_destination_style();
            }
        });
        widget.add_controller(motion);
    }

    fn size_column_shell(self: &Rc<Self>, shell: &gtk::Box, shell_state: &ColumnShell) {
        let preferences = crate::ui::preferences::PreferenceManager::shared();
        let unscaled_width = &shell_state.unscaled_width;
        let scale = preferences.interface_scale();
        shell.set_size_request(
            cap_column_width(
                (unscaled_width.get() * scale).round() as i32,
                column_width_cap(self),
            ),
            -1,
        );
        let previous_scale = Cell::new(scale);
        let weak_state = Rc::downgrade(self);
        let unscaled_for_scale = unscaled_width.clone();
        let generation_for_scale = shell_state.animation_generation.clone();
        preferences.bind_interface_scale(shell, move |shell, scale| {
            if previous_scale.replace(scale) == scale {
                return;
            }
            let Some(state) = weak_state.upgrade() else {
                return;
            };
            if !state.columns.borrow().iter().any(|c| c.shell == *shell) {
                return;
            }
            generation_for_scale.set(generation_for_scale.get().saturating_add(1));
            let width = (unscaled_for_scale.get() * scale).round() as i32;
            rescale_column(&state, shell, width);
        });
    }

    pub(super) fn column_shell(
        self: &Rc<Self>,
        depth: usize,
        location: &Location,
        column: &gtk::Box,
        filter_button: &gtk::ToggleButton,
    ) -> ColumnShell {
        let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        install_filter_blur(self, &shell, filter_button);
        let parts = ColumnShell {
            shell: shell.clone(),
            resize_handle: column_resize_handle(),
            reveal_button: column_reveal_button(location),
            animation_generation: Rc::new(Cell::new(0u64)),
            unscaled_width: Rc::new(Cell::new(f64::from(initial_column_width(self)))),
        };
        self.size_column_shell(&shell, &parts);
        shell.set_vexpand(true);
        shell.set_overflow(gtk::Overflow::Hidden);
        let column_overlay = gtk::Overlay::new();
        column_overlay.set_child(Some(column));
        column_overlay.set_hexpand(true);
        column_overlay.set_vexpand(true);
        // The column paints an opaque background, so a drop highlight needs an
        // overlay on top of it; picking falls through to the column's targets.
        column_overlay.add_overlay(&column_drop_veil());
        shell.append(&column_overlay);
        column_overlay.add_overlay(&parts.resize_handle);
        let weak = Rc::downgrade(self);
        let revealed_location = location.clone();
        parts.reveal_button.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.reveal_column_only(depth, &revealed_location);
            }
        });
        self.install_column_hover(&parts.reveal_button, depth);
        self.install_column_drop_targets(
            location,
            &[
                parts.reveal_button.upcast_ref(),
                parts.resize_handle.upcast_ref(),
            ],
        );
        column_overlay.add_overlay(&parts.reveal_button);
        parts
    }
}
