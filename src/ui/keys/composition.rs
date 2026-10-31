// SPDX-License-Identifier: MIT

//! GTK filters keys through the input method after every capture-phase
//! handler, so key handlers forward keys to a field with a preedit first.

use std::cell::{Cell, RefCell};

use gtk::{gdk, gio, glib, prelude::*};

thread_local! {
    static FOLLOWING: Cell<bool> = const { Cell::new(false) };
    static FOLLOWED: RefCell<Vec<glib::WeakRef<gtk::Widget>>> = const { RefCell::new(Vec::new()) };
    static COMPOSING: RefCell<Vec<glib::WeakRef<gtk::Widget>>> = const { RefCell::new(Vec::new()) };
}

/// GTK withholds the release of a handled press from later widgets, so a
/// forwarded press's release is forwarded too.
#[derive(Default)]
pub(super) struct Forwarded(RefCell<Vec<(gdk::Key, glib::WeakRef<gtk::Widget>)>>);

impl Forwarded {
    pub(super) fn press(&self, key: gdk::Key, field: &gtk::Widget) {
        let mut presses = self.0.borrow_mut();
        presses.retain(|(pressed, _)| *pressed != key);
        presses.push((key, field.downgrade()));
    }

    pub(super) fn release(&self, controller: &gtk::EventControllerKey, key: gdk::Key) {
        let field = {
            let mut presses = self.0.borrow_mut();
            let Some(index) = presses.iter().position(|(pressed, _)| *pressed == key) else {
                return;
            };
            presses.swap_remove(index).1.upgrade()
        };
        if let Some(field) = field {
            controller.forward(&field);
        }
    }
}

pub(super) fn composing_field(controller: &gtk::EventControllerKey) -> Option<gtk::Widget> {
    COMPOSING.with_borrow(|composing| {
        if composing.is_empty() {
            return None;
        }
        let focus = controller.widget()?.root()?.focus()?;
        composing
            .iter()
            .any(|field| field.upgrade().as_ref() == Some(&focus))
            .then_some(focus)
    })
}

pub(super) fn follow() {
    if FOLLOWING.replace(true) {
        return;
    }
    let windows = gtk::Window::toplevels();
    follow_windows(&windows, 0, windows.n_items());
    windows.connect_items_changed(|windows, position, _, added| {
        follow_windows(windows, position, added);
    });
}

fn follow_windows(windows: &gio::ListModel, position: u32, count: u32) {
    for window in (position..position + count)
        .filter_map(|index| windows.item(index).and_downcast::<gtk::Window>())
    {
        follow_focus(&window);
        window.connect_focus_widget_notify(follow_focus);
    }
}

fn follow_focus(window: &gtk::Window) {
    let Some(focus) = GtkWindowExt::focus(window) else {
        return;
    };
    let text = focus.downcast_ref::<gtk::Text>();
    let view = focus.downcast_ref::<gtk::TextView>();
    if text.is_none() && view.is_none() {
        return;
    }
    FOLLOWED.with_borrow_mut(|followed| {
        followed.retain(|field| field.upgrade().is_some());
        if followed
            .iter()
            .any(|field| field.upgrade().as_ref() == Some(&focus))
        {
            return;
        }
        if let Some(text) = text {
            text.connect_preedit_changed(|text, preedit| {
                composition_changed(text.upcast_ref(), preedit);
            });
        }
        if let Some(view) = view {
            view.connect_preedit_changed(|view, preedit| {
                composition_changed(view.upcast_ref(), preedit);
            });
        }
        followed.push(focus.downgrade());
    });
}

fn composition_changed(field: &gtk::Widget, preedit: &str) {
    COMPOSING.with_borrow_mut(|composing| {
        composing.retain(|other| other.upgrade().is_some_and(|other| &other != field));
        if !preedit.is_empty() {
            composing.push(field.downgrade());
        }
    });
}
