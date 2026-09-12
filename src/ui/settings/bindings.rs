// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn choice_menu<T: Copy + PartialEq + 'static>(
    manager: &Rc<ThemeManager>,
    title: &str,
    choices: &[(&'static str, T)],
    read: fn(&ThemeManager) -> T,
    write: fn(&ThemeManager, T),
) -> gtk::MenuButton {
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
    menu.add_css_class("column-menu");
    let popover = gtk::Popover::builder()
        .child(&menu)
        .has_arrow(false)
        .build();
    popover.add_css_class("column-popover");
    let button = gtk::MenuButton::builder()
        .popover(&popover)
        .always_show_arrow(true)
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("form-control");
    button.add_css_class("settings-choice");
    button.set_tooltip_text(Some(title));
    super::super::accessibility::set_label(&button, title);
    let labels = choices.to_vec();
    manager.bind_preference(&button, read, move |widget, value| {
        if let Some(button) = widget.downcast_ref::<gtk::MenuButton>() {
            button.set_label(
                labels
                    .iter()
                    .find(|(_, candidate)| *candidate == value)
                    .map_or(labels[0].0, |(label, _)| *label),
            );
        }
    });
    for &(label, value) in choices {
        let (option, check) = super::super::controls::menu_option(label, read(manager) == value);
        manager.bind_preference(&check, read, move |widget, selected| {
            widget.set_visible(selected == value)
        });
        let weak_button = button.downgrade();
        let manager = manager.clone();
        option.connect_clicked(move |_| {
            if read(&manager) != value {
                write(&manager, value);
            }
            if let Some(button) = weak_button.upgrade() {
                button.popdown();
            }
        });
        menu.append(&option);
    }
    button
}

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
