// SPDX-License-Identifier: MIT

//! A column's header: heading, status indicators, pane actions and the filter toggle.

use crate::model::Location;
use crate::ui::browser::ViewState;
use crate::ui::browser::pane_header::{
    column_sort_direction_toggle, column_sort_menu, empty_trash_button, leftovers_button,
    pane_new_folder_button, pane_refresh_button,
};
use crate::ui::browser::paths::is_trash_root;
use crate::ui::enter_target::EnterTarget;
use gtk::prelude::*;
use std::rc::Rc;

pub(super) struct ColumnHeader {
    pub(super) header: gtk::Box,
    pub(super) truncated_hint: gtk::Image,
    pub(super) spinner: gtk::Spinner,
    pub(super) header_actions: gtk::Box,
    pub(super) leftovers: gtk::Button,
    pub(super) empty_trash: gtk::Button,
    pub(super) is_trash: bool,
    pub(super) sort_direction_button: gtk::Button,
    pub(super) header_actions_stack: gtk::Stack,
    pub(super) filter_entry: gtk::Entry,
    pub(super) filter_revealer: gtk::Revealer,
    pub(super) filter_button: gtk::ToggleButton,
    pub(super) enter_target: EnterTarget,
}

fn column_heading(location: &Location) -> gtk::Label {
    let heading = gtk::Label::new(Some(&location.display_name()));
    heading.add_css_class("column-heading");
    heading.set_xalign(0.0);
    heading.set_yalign(0.5);
    heading.set_valign(gtk::Align::Center);
    heading.set_hexpand(true);
    heading.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    heading.set_max_width_chars(1);
    crate::ui::accessibility::set_description(&heading, Some(&location.display_path()));
    heading
}

fn truncated_hint() -> gtk::Image {
    let truncated_hint = crate::assets::primary_icon(crate::assets::icons::TRIANGLE_ALERT, 16);
    truncated_hint.add_css_class("column-truncated-hint");
    crate::ui::accessibility::set_description(
        &truncated_hint,
        Some(&crate::i18n::tr(
            "This directory has more entries than could be loaded; showing a partial listing.",
        )),
    );
    truncated_hint.set_visible(false);
    truncated_hint
}

/// Opening one column's filter closes the others.
fn close_other_filters(state: &ViewState, depth: usize) {
    for (other_depth, other) in state.columns.borrow().iter().enumerate() {
        if other_depth != depth && other.filter_button.is_active() {
            other.filter_button.set_active(false);
        }
    }
}

fn close_pane_button(state: &ViewState, depth: usize) -> gtk::Button {
    let close_label = crate::i18n::tr("Close this pane");
    let close = gtk::Button::builder().tooltip_text(&close_label).build();
    close.set_child(Some(&crate::assets::chrome_icon(crate::assets::icons::X)));
    crate::ui::accessibility::set_label(&close, &close_label);
    crate::ui::controls::pane_header_action(&close);
    let weak_browser = Rc::downgrade(&state.browser);
    close.connect_clicked(move |_| {
        if crate::ui::tenxer_mode::chrome_suppressed() {
            return;
        }
        if let Some(browser) = weak_browser.upgrade() {
            browser.close_column(depth);
        }
    });
    crate::ui::tenxer_mode::hide_while_enabled(&close);
    close
}

/// Homogeneous pages keep column geometry stable as the action target changes.
fn header_actions_stack(header_actions: &gtk::Box) -> gtk::Stack {
    let header_actions_stack = gtk::Stack::new();
    header_actions_stack.set_valign(gtk::Align::Center);
    header_actions_stack.add_named(header_actions, Some("actions"));
    header_actions_stack.add_named(
        &gtk::Box::new(gtk::Orientation::Horizontal, 0),
        Some("hidden"),
    );
    header_actions_stack
}

impl ViewState {
    pub(super) fn column_header(
        self: &Rc<Self>,
        depth: usize,
        location: &Location,
    ) -> ColumnHeader {
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("column-header");
        let heading_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        heading_box.set_hexpand(true);
        heading_box.set_valign(gtk::Align::Center);
        let heading = column_heading(location);
        let truncated_hint = truncated_hint();
        heading_box.append(&heading);
        heading_box.append(&truncated_hint);
        let spinner = gtk::Spinner::new();
        spinner.set_valign(gtk::Align::Center);
        spinner.set_visible(false);
        header.append(&heading_box);
        header.append(&spinner);
        let header_actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header_actions.add_css_class("column-header-actions");
        let leftovers = leftovers_button(&self.browser, depth);
        header_actions.append(&leftovers);
        let empty_trash = empty_trash_button(&self.browser);
        let is_trash = is_trash_root(location);
        empty_trash.set_visible(is_trash);
        empty_trash.set_sensitive(false);
        header_actions.append(&empty_trash);
        if !self.interactive {
            header_actions.append(&pane_new_folder_button(Rc::downgrade(self), depth));
        }
        header_actions.append(&pane_refresh_button(&self.browser, depth));
        let sort_direction_button = column_sort_direction_toggle(&self.browser, depth);
        header_actions.append(&sort_direction_button);
        header_actions.append(&column_sort_menu(&self.browser, depth));

        let (filter_entry, filter_revealer, filter_button) =
            crate::ui::browser_modes::filter_controls("Filter this pane (%{keys})");
        crate::ui::browser::bind_filter_descent(&filter_entry, Rc::downgrade(self));
        let enter_target = EnterTarget::new(&filter_entry);
        let weak_self = Rc::downgrade(self);
        filter_button.connect_toggled(move |button| {
            if button.is_active()
                && let Some(state) = weak_self.upgrade()
            {
                close_other_filters(&state, depth);
            }
        });
        header_actions.append(&filter_button);
        if depth > 0 {
            header_actions.append(&close_pane_button(self, depth));
        }
        let header_actions_stack = header_actions_stack(&header_actions);
        header.append(&header_actions_stack);
        ColumnHeader {
            header,
            truncated_hint,
            spinner,
            header_actions,
            leftovers,
            empty_trash,
            is_trash,
            sort_direction_button,
            header_actions_stack,
            filter_entry,
            filter_revealer,
            filter_button,
            enter_target,
        }
    }
}
