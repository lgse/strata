// SPDX-License-Identifier: MIT

use gtk::{gdk::Key, glib, prelude::*};

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
                    button.emit_clicked();
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

fn menu_buttons(popover: &gtk::Popover) -> Vec<gtk::Button> {
    let mut buttons = Vec::new();
    collect_buttons(popover.upcast_ref(), &mut buttons);
    buttons
}

fn collect_buttons(widget: &gtk::Widget, buttons: &mut Vec<gtk::Button>) {
    if !widget.is_visible() || !widget.is_sensitive() {
        return;
    }
    if let Some(button) = widget.downcast_ref::<gtk::Button>() {
        buttons.push(button.clone());
        return;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        collect_buttons(&widget, buttons);
        child = widget.next_sibling();
    }
}
