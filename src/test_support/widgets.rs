// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use gtk::gdk::{Key, ModifierType};
use gtk::{glib, prelude::*};

/// A context-menu key pressed with the pane-header Back button focused.
pub(crate) struct BackHistoryKey<'a> {
    pub(crate) window: &'a gtk::Widget,
    pub(crate) browser: &'a crate::app::Browser,
    pub(crate) case: &'a str,
    pub(crate) key: Key,
    pub(crate) modifiers: ModifierType,
}

impl BackHistoryKey<'_> {
    /// `press_window` runs the window's capture-phase key handler, which sees
    /// the key first and must leave it to Back. Back then lists its history,
    /// alone and without going back.
    pub(crate) fn assert_lists_history(&self, press_window: impl Fn(Key, ModifierType) -> bool) {
        let case = self.case;
        let back = wait_for(|| mapped_back_button(self.window))
            .unwrap_or_else(|| panic!("{case}: no Back button"));
        let history =
            history_popover(&back).unwrap_or_else(|| panic!("{case}: Back has no history menu"));
        let location = self.browser.active_location();
        assert!(back.grab_focus(), "{case}: Back took no focus");
        assert!(
            !press_window(self.key, self.modifiers),
            "{case}: the window claimed the key from Back"
        );
        press_on_widget(&back, self.key, self.modifiers);
        assert!(
            wait_for(|| history.is_visible().then_some(())).is_some(),
            "{case}: the history menu did not open"
        );
        let others = visible_popovers(self.window)
            .into_iter()
            .filter(|popover| popover != &history)
            .count();
        assert_eq!(others, 0, "{case}: another menu opened with the history");
        assert_eq!(
            self.browser.active_location(),
            location,
            "{case}: the key went back"
        );
        history.popdown();
        assert!(
            wait_for(|| (!history.is_visible()).then_some(())).is_some(),
            "{case}: the history menu did not close"
        );
    }
}

fn wait_for<T>(found: impl Fn() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(value) = found() {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A view-mode switch leaves the old header in the tree until it rebuilds,
/// and until the pane is allocated it has no context menu to open instead.
fn mapped_back_button(widget: &gtk::Widget) -> Option<gtk::Widget> {
    if !widget.is_mapped() {
        return None;
    }
    if widget.has_css_class("list-navigation") {
        return widget
            .first_child()
            .filter(|back| back.is_mapped() && back.width() > 0);
    }
    children(widget).find_map(|child| mapped_back_button(&child))
}

fn history_popover(back: &gtk::Widget) -> Option<gtk::Popover> {
    children(back)
        .filter(|child| child.has_css_class("back-history-popover"))
        .find_map(|child| child.downcast::<gtk::Popover>().ok())
}

fn visible_popovers(widget: &gtk::Widget) -> Vec<gtk::Popover> {
    let mut found: Vec<gtk::Popover> = widget
        .downcast_ref::<gtk::Popover>()
        .filter(|popover| popover.is_visible())
        .into_iter()
        .cloned()
        .collect();
    for child in children(widget) {
        found.extend(visible_popovers(&child));
    }
    found
}

/// Presses a key on `widget`'s own key controllers, as GTK does once the
/// window's capture-phase controller declines it.
fn press_on_widget(widget: &gtk::Widget, key: Key, modifiers: ModifierType) {
    let controllers = widget.observe_controllers();
    for index in 0..controllers.n_items() {
        if let Some(keys) = controllers
            .item(index)
            .and_downcast::<gtk::EventControllerKey>()
        {
            keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]);
        }
    }
}

fn children(widget: &gtk::Widget) -> impl Iterator<Item = gtk::Widget> {
    std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
}
