// SPDX-License-Identifier: MIT

//! Header controls Icons and List panes share.

use std::rc::Rc;

use gtk::prelude::*;

use crate::{
    app::Browser,
    ui::{
        browser,
        keys::{
            Keys,
            bindings::{HISTORY_BACK, HISTORY_FORWARD, PARENT_FOLDER},
        },
        shortcut_reference,
    },
};

/// Back, Forward, and Parent folder buttons.
pub(super) fn list_navigation(browser: &Rc<Browser>) -> gtk::Box {
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("list-navigation");
    for (icon, tooltip, binding, action, available) in [
        (
            crate::assets::icons::ARROW_LEFT,
            "Back (%{keys})",
            HISTORY_BACK,
            Browser::back as fn(&Rc<Browser>),
            browser.can_go_back(),
        ),
        (
            crate::assets::icons::ARROW_RIGHT,
            "Forward (%{keys})",
            HISTORY_FORWARD,
            Browser::forward as fn(&Rc<Browser>),
            browser.can_go_forward(),
        ),
        (
            crate::assets::icons::ARROW_UP,
            "Parent folder (%{keys})",
            PARENT_FOLDER,
            Browser::parent as fn(&Rc<Browser>),
            browser.can_go_parent(),
        ),
    ] {
        let button = gtk::Button::builder().sensitive(available).build();
        let keys = Keys::One(binding.alternate(0).shown());
        shortcut_reference::follow_tooltip(&button, tooltip, keys);
        shortcut_reference::follow_label(&button, tooltip, keys);
        button.set_child(Some(&crate::assets::chrome_icon(icon)));
        button.add_css_class("list-navigation-button");
        button.set_cursor_from_name(Some("pointer"));
        let weak_browser = Rc::downgrade(browser);
        button.connect_clicked(move |_| {
            if let Some(browser) = weak_browser.upgrade() {
                action(&browser);
            }
        });
        actions.append(&button);
    }
    if let Some(back) = actions.first_child().and_downcast::<gtk::Button>() {
        browser::attach_back_history(browser, &back);
    }
    actions
}

/// The leftovers and Empty Trash header buttons, appended to `actions`.
pub(super) struct ContentActions {
    pub(super) leftovers: gtk::Button,
    /// Only a trash root keeps its Empty Trash button.
    pub(super) empty_trash: Option<gtk::Button>,
}

impl ContentActions {
    pub(super) fn append(actions: &gtk::Box, browser: &Rc<Browser>, depth: usize) -> Self {
        let leftovers = browser::leftovers_button(browser, depth);
        actions.append(&leftovers);
        let empty_trash = browser::empty_trash_button(browser);
        let is_trash = browser
            .location_at(depth)
            .is_some_and(|location| browser::is_trash_root(&location));
        empty_trash.set_visible(is_trash);
        empty_trash.set_sensitive(false);
        actions.append(&empty_trash);
        Self {
            leftovers,
            empty_trash: is_trash.then_some(empty_trash),
        }
    }
}
