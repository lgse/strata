// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc};

use gtk::{gdk, prelude::*};

use crate::app::Presence;

type Update = Rc<dyn Fn(&gtk::Window)>;

pub(in crate::ui) fn follow(
    window: &impl IsA<gtk::Window>,
    apply: impl Fn(Presence, bool) + 'static,
) {
    let window = window.upcast_ref::<gtk::Window>();
    let power_saver = Rc::new(Cell::new(false));
    let saving = power_saver.clone();
    let update: Update = Rc::new(move |window: &gtk::Window| {
        apply(presence_of(window), saving.get());
    });
    let changed = update.clone();
    window.connect_is_active_notify(move |window| changed(window));
    let changed = update.clone();
    window.connect_map(move |window| changed(window));
    let changed = update.clone();
    window.connect_unmap(move |window| changed(window));
    let changed = update.clone();
    window.connect_realize(move |window| {
        let Some(toplevel) = window.surface().and_downcast::<gdk::Toplevel>() else {
            return;
        };
        let weak = window.downgrade();
        let changed = changed.clone();
        toplevel.connect_state_notify(move |_| {
            if let Some(window) = weak.upgrade() {
                changed(&window);
            }
        });
    });
    follow_power_saver(window, power_saver, update);
}

fn presence_of(window: &gtk::Window) -> Presence {
    let state = window
        .surface()
        .and_downcast::<gdk::Toplevel>()
        .map_or(gdk::ToplevelState::empty(), |toplevel| toplevel.state());
    presence(window.is_mapped(), window.is_active(), state)
}

fn presence(mapped: bool, active: bool, state: gdk::ToplevelState) -> Presence {
    if !mapped || state.intersects(gdk::ToplevelState::SUSPENDED | gdk::ToplevelState::MINIMIZED) {
        Presence::Hidden
    } else if active {
        Presence::Focused
    } else {
        Presence::Visible
    }
}

#[cfg(test)]
fn follow_power_saver(_: &gtk::Window, _: Rc<Cell<bool>>, _: Update) {}

#[cfg(not(test))]
fn follow_power_saver(window: &gtk::Window, power_saver: Rc<Cell<bool>>, update: Update) {
    use gtk::{gio, glib};
    let armed = Cell::new(false);
    window.connect_map(move |window| {
        if armed.replace(true) {
            return;
        }
        let weak = window.downgrade();
        let power_saver = power_saver.clone();
        let update = update.clone();
        glib::idle_add_local_once(move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let monitor = gio::PowerProfileMonitor::get_default();
            let refresh = move |monitor: &gio::PowerProfileMonitor, window: &gtk::Window| {
                power_saver.set(monitor.is_power_saver_enabled());
                update(window);
            };
            refresh(&monitor, &window);
            let weak = window.downgrade();
            let handler = monitor.connect_power_saver_enabled_notify(move |monitor| {
                if let Some(window) = weak.upgrade() {
                    refresh(monitor, &window);
                }
            });
            let handler = std::cell::RefCell::new(Some(handler));
            window.connect_destroy(move |_| {
                if let Some(handler) = handler.take() {
                    monitor.disconnect(handler);
                }
            });
        });
    });
}

#[cfg(test)]
mod tests;
