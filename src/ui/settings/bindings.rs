// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn bind_number(
    manager: &Rc<ThemeManager>,
    control: &gtk::SpinButton,
    read: fn(&ThemeManager) -> f64,
    write: fn(&ThemeManager, f64),
) {
    manager.bind_preference(control, read, |widget, value| {
        if let Some(control) = widget.downcast_ref::<gtk::SpinButton>() {
            control.set_value(value);
        }
    });
    let manager = manager.clone();
    control.connect_value_changed(move |control| {
        let value = control.value();
        if read(&manager) != value {
            write(&manager, value);
        }
    });
}

pub(super) fn bind_switch(
    manager: &Rc<ThemeManager>,
    toggle: &gtk::Switch,
    read: fn(&ThemeManager) -> bool,
    write: fn(&ThemeManager, bool),
) {
    manager.bind_preference(toggle, read, |widget, value| {
        if let Some(toggle) = widget.downcast_ref::<gtk::Switch>() {
            toggle.set_active(value);
        }
    });
    let manager = manager.clone();
    toggle.connect_active_notify(move |toggle| {
        let value = toggle.is_active();
        if read(&manager) != value {
            write(&manager, value);
        }
    });
}

pub(super) fn bind_choice<T: Copy + PartialEq + 'static>(
    manager: &Rc<ThemeManager>,
    button: &gtk::ToggleButton,
    value: T,
    read: impl Fn(&ThemeManager) -> T + 'static,
    write: impl Fn(&ThemeManager, T) + 'static,
) {
    let read = Rc::new(read);
    let read_for_binding = read.clone();
    manager.bind_preference(
        button,
        move |manager| read_for_binding(manager) == value,
        |widget, active| {
            if let Some(button) = widget.downcast_ref::<gtk::ToggleButton>() {
                button.set_active(active);
            }
        },
    );
    let manager = manager.clone();
    button.connect_toggled(move |button| {
        if button.is_active() && read(&manager) != value {
            write(&manager, value);
        }
    });
}
