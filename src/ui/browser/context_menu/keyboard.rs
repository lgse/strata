// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc};

use gtk::{gdk::Key, glib, prelude::*};

pub(super) fn install_menu_edges(popover: &gtk::PopoverMenu) {
    // Extend GTK's native menu navigation without replacing generated rows.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.set_propagation_limit(gtk::PropagationLimit::None);
    let weak = popover.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if !modifiers.is_empty() {
            return glib::Propagation::Proceed;
        }
        let Some(popover) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if !matches!(key, Key::Home | Key::End) {
            return glib::Propagation::Proceed;
        }
        let active = popover
            .root()
            .and_then(|root| root.focus())
            .and_then(|focus| focus.ancestor(gtk::Popover::static_type()))
            .and_downcast::<gtk::Popover>()
            .unwrap_or_else(|| popover.clone().upcast());
        focus_first_or_last_menu_item(&active, key == Key::Home);
        glib::Propagation::Stop
    });
    popover.add_controller(keys);
}

pub(super) fn install_submenu_return(submenu: &gtk::PopoverMenu, owner: &gtk::Widget) {
    let returned = Rc::new(Cell::new(false));
    let weak_submenu = submenu.downgrade();
    let weak_owner = owner.downgrade();
    let returned_for_submenu = returned.clone();
    let submenu_keys = gtk::EventControllerKey::new();
    submenu_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    submenu_keys.connect_key_pressed(move |_, key, _, modifiers| {
        if !modifiers.is_empty() {
            return glib::Propagation::Proceed;
        }
        let (Some(submenu), Some(owner)) = (weak_submenu.upgrade(), weak_owner.upgrade()) else {
            return glib::Propagation::Proceed;
        };
        match key {
            Key::Left => {
                submenu.set_visible(false);
                owner.grab_focus();
                returned_for_submenu.set(true);
                glib::Propagation::Stop
            }
            Key::Right if returned_for_submenu.replace(false) => {
                owner.activate();
                let submenu = submenu.downgrade();
                glib::idle_add_local_once(move || {
                    if let Some(submenu) = submenu.upgrade() {
                        focus_first_or_last_menu_item(submenu.upcast_ref(), true);
                    }
                });
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    submenu.add_controller(submenu_keys);

    let weak_submenu = submenu.downgrade();
    let weak_owner = owner.downgrade();
    let owner_keys = gtk::EventControllerKey::new();
    owner_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    owner_keys.connect_key_pressed(move |_, key, _, modifiers| {
        if key != Key::Right || !modifiers.is_empty() || !returned.replace(false) {
            return glib::Propagation::Proceed;
        }
        let (Some(submenu), Some(owner)) = (weak_submenu.upgrade(), weak_owner.upgrade()) else {
            return glib::Propagation::Proceed;
        };
        owner.activate();
        glib::idle_add_local_once(move || {
            focus_first_or_last_menu_item(submenu.upcast_ref(), true);
        });
        glib::Propagation::Stop
    });
    owner.add_controller(owner_keys);
}

pub(super) fn install(popover: &gtk::Popover) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = popover.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(popover) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if modifiers.intersects(
            gtk::accelerator_get_default_mod_mask() & !gtk::gdk::ModifierType::SHIFT_MASK,
        ) {
            return glib::Propagation::Stop;
        }
        // Captured menu keys bypass GTK's focus-indicator timeout refresh.
        if let Some(window) = popover.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(true);
        }
        match key {
            Key::Escape => popover.popdown(),
            Key::Home => focus_first_or_last_menu_item(&popover, true),
            Key::End => focus_first_or_last_menu_item(&popover, false),
            Key::Up | Key::Down | Key::Tab | Key::ISO_Left_Tab => {
                let forward = key == Key::Down
                    || (key == Key::Tab && !modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK));
                let buttons = menu_buttons(&popover);
                if !buttons.is_empty() {
                    let current = buttons.iter().position(|button| button.has_focus());
                    let next = match (current, forward) {
                        (Some(index), true) => (index + 1) % buttons.len(),
                        (Some(index), false) => (index + buttons.len() - 1) % buttons.len(),
                        (None, true) => 0,
                        (None, false) => buttons.len() - 1,
                    };
                    buttons[next].grab_focus();
                }
            }
            Key::Return | Key::KP_Enter | Key::space => {
                if let Some(button) = menu_buttons(&popover)
                    .iter()
                    .find(|button| button.has_focus())
                {
                    // activate() waits for a key release that this controller consumes.
                    if let Some(button) = button.downcast_ref::<gtk::Button>() {
                        button.emit_clicked();
                    } else {
                        button.activate();
                    }
                }
            }
            _ => {}
        }
        glib::Propagation::Stop
    });
    popover.add_controller(keys);
}

pub(super) fn focus_first_or_last_menu_item(popover: &gtk::Popover, first: bool) {
    let buttons = menu_buttons(popover);
    if let Some(button) = if first {
        buttons.first()
    } else {
        buttons.last()
    } {
        button.grab_focus();
    }
}

fn menu_buttons(popover: &gtk::Popover) -> Vec<gtk::Widget> {
    let mut buttons = Vec::new();
    collect_buttons(popover.upcast_ref(), popover, &mut buttons);
    buttons
}

fn collect_buttons(widget: &gtk::Widget, popover: &gtk::Popover, buttons: &mut Vec<gtk::Widget>) {
    if !widget.is_visible()
        || !widget.is_sensitive()
        || (widget.is::<gtk::Popover>() && widget != popover.upcast_ref::<gtk::Widget>())
    {
        return;
    }
    if widget.is::<gtk::Button>()
        || (widget.is_focusable() && widget.accessible_role() == gtk::AccessibleRole::MenuItem)
    {
        buttons.push(widget.clone());
        return;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        collect_buttons(&widget, popover, buttons);
        child = widget.next_sibling();
    }
}
