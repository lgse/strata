// SPDX-License-Identifier: MIT

use crate::model::Location;
use crate::ui::browser::clipboard::{
    PreparedFileDrop, arm_spring_load_navigation, copy_path_text, prepare_file_drop_target,
};
use crate::ui::browser::{
    DropHandler, ViewState, attach_drop_target, commit_dropped_value, file_drop_action,
};
use crate::ui::browser_modes::BrowserMode;
use crate::ui::collection_interaction::{MiddleClickAction, middle_click_action};
use gtk::glib;
use gtk::prelude::*;
use std::cell::Cell;
use std::path::Path;
use std::rc::{Rc, Weak};
use std::time::Duration;

// Long crumbs middle-elide past this cap; the scroller handles deeper paths.
const BREADCRUMB_LABEL_MAX_CHARS: i32 = 32;

fn ellipsize_crumb_label(label: &gtk::Label) {
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_max_width_chars(BREADCRUMB_LABEL_MAX_CHARS);
    label.set_width_chars(
        label
            .text()
            .chars()
            .take(BREADCRUMB_LABEL_MAX_CHARS as usize)
            .count() as i32,
    );
}

pub(in crate::ui::browser) fn is_breadcrumb_button_target(mut target: gtk::Widget) -> bool {
    loop {
        if target.is::<gtk::Button>() {
            return true;
        }
        let Some(parent) = target.parent() else {
            return false;
        };
        if parent.has_css_class("breadcrumbs") {
            return false;
        }
        target = parent;
    }
}

fn is_filesystem_root(location: &Location) -> bool {
    location
        .native_path()
        .is_some_and(|path| path == Path::new("/"))
}

/// The crumbs shown for `location`, starting at the home folder when it lies inside it.
fn visible_crumbs(location: &Location, home: &Location) -> Vec<Location> {
    let mut locations = location.breadcrumbs();
    if let Some(home_index) = locations.iter().position(|crumb| crumb == home) {
        locations.drain(..home_index);
    }
    locations
}

fn crumb_label(crumb: &Location, home: &Location) -> String {
    if crumb == home {
        "~".to_owned()
    } else {
        crumb.display_name()
    }
}

fn set_copy_path_feedback(button: &gtk::Button, icon: &gtk::Image, copied: bool) {
    let (icon_name, label) = if copied {
        (crate::assets::icons::CHECK, "Path copied")
    } else {
        (crate::assets::icons::COPY, "Copy path")
    };
    crate::assets::set_primary_icon(icon, icon_name);
    crate::ui::accessibility::set_tooltip_and_label(button, &crate::i18n::tr(label));
}

fn copy_path_button(location: &Location) -> gtk::Button {
    let copy = gtk::Button::new();
    crate::ui::accessibility::set_tooltip_and_label(&copy, &crate::i18n::tr("Copy path"));
    let copy_icon = crate::assets::primary_icon(crate::assets::icons::COPY, 16);
    copy.set_child(Some(&copy_icon));
    copy.add_css_class("copy-path");
    copy.set_has_frame(false);
    copy.set_cursor_from_name(Some("pointer"));
    let copied_path = copy_path_text(location, true);
    let feedback_generation = Rc::new(Cell::new(0_u64));
    copy.connect_clicked(move |button| {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&copied_path);
        }
        let generation = feedback_generation.get().saturating_add(1);
        feedback_generation.set(generation);
        set_copy_path_feedback(button, &copy_icon, true);
        let button = button.clone();
        let copy_icon = copy_icon.clone();
        let feedback_generation = feedback_generation.clone();
        glib::timeout_add_local_once(Duration::from_secs(2), move || {
            if feedback_generation.get() == generation {
                set_copy_path_feedback(&button, &copy_icon, false);
            }
        });
    });
    copy
}

fn current_crumb(crumb: &Location, label: &str, location: &Location) -> gtk::Box {
    let current = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    current.add_css_class("current-breadcrumb");
    let current_label = gtk::Label::new(Some(label));
    current_label.add_css_class("breadcrumb");
    current_label.add_css_class("current");
    ellipsize_crumb_label(&current_label);
    crate::ui::accessibility::set_description(&current_label, Some(&crumb.display_path()));
    current.append(&current_label);
    current.append(&copy_path_button(location));
    current
}

fn ancestor_crumb_button(crumb: &Location, label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    if let Some(label) = button.child().and_downcast::<gtk::Label>() {
        ellipsize_crumb_label(&label);
    }
    if crumb.is_administrator() && crumb.parent().is_none() {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        content.append(&crate::assets::primary_icon(crate::assets::icons::LOCK, 14));
        content.append(&gtk::Label::new(Some(label)));
        button.set_child(Some(&content));
        crate::ui::accessibility::set_label(&button, label);
    }
    button.add_css_class("breadcrumb");
    if is_filesystem_root(crumb) {
        button.add_css_class("breadcrumb-root");
    }
    button.set_has_frame(false);
    crate::ui::accessibility::set_description(&button, Some(&crumb.display_path()));
    button.set_cursor_from_name(Some("pointer"));
    button
}

/// A middle click opens the crumb in a new tab, or a new window with Ctrl or Shift.
fn install_crumb_middle_click(button: &gtk::Button, crumb: Location) {
    let click = gtk::GestureClick::new();
    click.set_button(gtk::gdk::BUTTON_MIDDLE);
    click.connect_released(move |gesture, count, _, _| {
        if count != 1 {
            return;
        }
        let Some(widget) = gesture.widget() else {
            return;
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        let modifiers = gesture.current_event_state();
        let action = match middle_click_action(
            true,
            modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK),
            modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK),
        ) {
            MiddleClickAction::OpenWindow => "win.open-window-at",
            MiddleClickAction::OpenTab | MiddleClickAction::RevealParent => "win.open-tab-at",
        };
        let uri = crate::adapters::gio_file_for_location(&crumb)
            .uri()
            .to_variant();
        if let Err(error) = widget.activate_action(action, Some(&uri)) {
            tracing::warn!(%error, action, "open-in-new location action unavailable");
        }
    });
    button.add_controller(click);
}

/// Hovering a drag over the crumb springs into it; dropping transfers into it.
fn attach_crumb_drop_target(button: &gtk::Button, crumb: Location, weak: &Weak<ViewState>) {
    let spring_navigate: Rc<dyn Fn(Location)> = {
        let weak = weak.clone();
        Rc::new(move |location| {
            if let Some(state) = weak.upgrade() {
                state.browser.navigate_location(location, false);
            }
        })
    };
    let PreparedFileDrop {
        target: drop,
        state: drop_state,
    } = prepare_file_drop_target(move || Some(crumb.clone()));
    let state_for_enter = drop_state.clone();
    let navigate_for_enter = spring_navigate.clone();
    drop.connect_enter(move |target, _, _| {
        let action = file_drop_action(target, &state_for_enter);
        arm_spring_load_navigation(&state_for_enter, target, &navigate_for_enter);
        action
    });
    let state_for_motion = drop_state.clone();
    let navigate_for_motion = spring_navigate.clone();
    drop.connect_motion(move |target, _, _| {
        let action = file_drop_action(target, &state_for_motion);
        arm_spring_load_navigation(&state_for_motion, target, &navigate_for_motion);
        action
    });
    let state_for_value = drop_state.clone();
    drop.connect_value_notify(move |target| {
        if target.current_drop().is_none() {
            return;
        }
        arm_spring_load_navigation(&state_for_value, target, &spring_navigate);
    });
    let state_for_leave = drop_state.clone();
    drop.connect_leave(move |_| {
        state_for_leave.cancel_spring_load_navigation();
    });
    let handler: DropHandler = {
        let weak = weak.clone();
        Rc::new(move |destination, request| {
            if let Some(state) = weak.upgrade() {
                state.commit_drop(destination, request);
            }
        })
    };
    let handler_for_drop = handler.clone();
    let state_for_drop = drop_state.clone();
    drop.connect_drop(move |target, value, _, _| {
        state_for_drop.cancel_spring_load_navigation();
        let Some(destination) = state_for_drop.destination() else {
            return false;
        };
        let handler = handler_for_drop.clone();
        commit_dropped_value(
            target,
            value,
            destination,
            &state_for_drop,
            move |destination, request| handler(destination, request),
        )
    });
    attach_drop_target(button, &drop, &drop_state, handler);
}

fn crumb_icon(crumb: &Location, home: &Location) -> &'static str {
    if crumb == home {
        crate::assets::icons::HOME
    } else if is_filesystem_root(crumb) {
        crate::assets::icons::HARD_DRIVE
    } else if crumb.uri_value().is_some_and(|u| u.starts_with("trash://")) {
        crate::assets::icons::TRASH
    } else if crumb.uri_value().is_some() {
        crate::assets::icons::NETWORK
    } else {
        crate::assets::icons::FOLDER
    }
}

fn hierarchy_item_row(crumb: &Location, home: &Location) -> gtk::Box {
    let item_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon = crate::assets::primary_icon(crumb_icon(crumb, home), 16);
    let label = gtk::Label::new(Some(&crumb_label(crumb, home)));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    ellipsize_crumb_label(&label);
    item_row.append(&icon);
    item_row.append(&label);
    item_row
}

impl ViewState {
    /// The folder the view shows: Columns ends at its deepest column, while Icons
    /// and List show only the active folder, however many columns stay open.
    pub(in crate::ui::browser) fn displayed_location(&self) -> Option<Location> {
        match self.mode.get() {
            BrowserMode::Columns => (0..)
                .map_while(|depth| self.browser.location_at(depth))
                .last(),
            BrowserMode::Icons | BrowserMode::List | BrowserMode::Compact => {
                self.browser.active_location()
            }
        }
    }

    pub(in crate::ui::browser) fn sync_displayed_location(self: &Rc<Self>) {
        if let Some(location) = self.displayed_location() {
            self.set_location(&location);
        }
    }

    fn append_ancestor_crumb(self: &Rc<Self>, crumb: Location, label: &str) {
        let button = ancestor_crumb_button(&crumb, label);
        let weak = Rc::downgrade(self);
        let clicked_weak = weak.clone();
        let clicked_crumb = crumb.clone();
        button.connect_clicked(move |_| {
            if let Some(state) = clicked_weak.upgrade() {
                state.browser.navigate_to_ancestor(clicked_crumb.clone());
            }
        });
        if !self.browser.is_chooser_mode() {
            install_crumb_middle_click(&button, crumb.clone());
        }
        attach_crumb_drop_target(&button, crumb, &weak);
        self.breadcrumbs.append(&button);
    }

    pub(in crate::ui::browser) fn set_location(self: &Rc<Self>, location: &Location) {
        while let Some(child) = self.breadcrumbs.first_child() {
            self.breadcrumbs.remove(&child);
        }

        let home = Location::local(glib::home_dir());
        let locations = visible_crumbs(location, &home);
        let starts_at_root = locations.first().is_some_and(is_filesystem_root);
        let last = locations.len().saturating_sub(1);
        for (index, crumb) in locations.into_iter().enumerate() {
            let joins_root = starts_at_root && index == 1;
            if index > 0 && !joins_root {
                let separator = gtk::Label::new(Some("/"));
                separator.add_css_class("breadcrumb-separator");
                self.breadcrumbs.append(&separator);
            }
            let label = crumb_label(&crumb, &home);
            if index == last {
                self.breadcrumbs
                    .append(&current_crumb(&crumb, &label, location));
            } else {
                self.append_ancestor_crumb(crumb, &label);
            }
        }
        // A folder that arrives while the user types a location, such as a new
        // tab's first load, must not close the entry or replace the typed text.
        let editing = self.location_stack.visible_child_name().as_deref() == Some("entry")
            && self.location_entry_has_focus();
        if !editing {
            self.show_breadcrumbs();
            self.location_entry.set_text(&location.display_path());
        }
        self.breadcrumb_reveal_pending.set(true);
    }

    fn hierarchy_item(
        self: &Rc<Self>,
        (index, crumb): (usize, &Location),
        home: &Location,
        popover: &gtk::Popover,
    ) -> gtk::Button {
        let button = gtk::Button::builder()
            .child(&hierarchy_item_row(crumb, home))
            .has_frame(false)
            .build();
        button.set_cursor_from_name(Some("pointer"));
        button.add_css_class("breadcrumb-hierarchy-item");
        if index == 0 {
            button.add_css_class("current");
        }
        let weak_self = Rc::downgrade(self);
        let weak_popover = popover.downgrade();
        let target_crumb = crumb.clone();
        button.connect_clicked(move |_| {
            if let Some(popover) = weak_popover.upgrade() {
                popover.popdown();
            }
            if let Some(state) = weak_self.upgrade() {
                state.browser.navigate_to_ancestor(target_crumb.clone());
            }
        });
        button
    }

    pub(in crate::ui::browser) fn show_breadcrumb_hierarchy_menu(
        self: &Rc<Self>,
        anchor: &gtk::Widget,
        x: f64,
    ) {
        let Some(location) = self.displayed_location() else {
            return;
        };
        if location.breadcrumbs().is_empty() {
            return;
        }
        let home = Location::local(glib::home_dir());
        let mut locations = visible_crumbs(&location, &home);
        locations.reverse();

        let menu_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        menu_box.add_css_class("breadcrumb-hierarchy-menu");

        let popover = gtk::Popover::builder()
            .child(&menu_box)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            // The whole bar's height keeps the menu clear of the crumbs it lists.
            .pointing_to(&gtk::gdk::Rectangle::new(x as i32, 0, 1, anchor.height()))
            .build();
        popover.add_css_class("breadcrumb-popover");
        popover.set_parent(anchor);

        for (index, crumb) in locations.iter().enumerate() {
            menu_box.append(&self.hierarchy_item((index, crumb), &home, &popover));
        }

        popover.connect_closed(move |popover| {
            popover.unparent();
        });

        popover.popup();
    }
}
