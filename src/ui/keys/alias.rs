// SPDX-License-Identifier: MIT

//! The GTK half of the aliases of structural commands: what owns typing where a
//! press lands, and GTK's own effect for an alias nothing claimed. The rewrite
//! itself is `strata_desktop::keys::rewrite`. See `docs/keyboard-shortcuts.md`.

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib,
    prelude::*,
};

pub(super) use strata_desktop::keys::FALLBACK;

use super::{Context, Map, gdk_key, gdk_modifiers, rewrite};

/// What owns typing where a press lands, from the focused widget.
pub(super) fn context_of(focused: Option<&gtk::Widget>, type_to_search: bool) -> Context {
    let tenxer = Map::active() == Map::Tenxer;
    let owns_typing = |widget: &gtk::Widget| {
        (if tenxer {
            crate::ui::focus_navigation::accepts_typing(widget)
        } else {
            crate::ui::focus_navigation::editable(widget)
        }) || crate::ui::focus_navigation::in_popover(widget)
    };
    Context {
        typing: type_to_search || focused.is_some_and(owns_typing),
    }
}

pub(super) fn context_of_controller(controller: &impl IsA<gtk::EventController>) -> Context {
    let focused = controller
        .widget()
        .and_then(|widget| widget.root())
        .and_then(|root| root.focus());
    let type_to_search = crate::ui::preferences::PreferenceManager::existing()
        .is_some_and(|preferences| preferences.type_to_search_active());
    context_of(focused.as_ref(), type_to_search)
}

/// GTK's own bindings see an alias as the key pressed, so an alias that
/// nothing claimed by the end of propagation gets the canonical key's GTK
/// effect from here. Plain printable aliases act only where Strata reads them.
pub(crate) fn install_native(window: &impl IsA<gtk::Window>) -> gtk::EventControllerKey {
    let keys = gtk::EventControllerKey::new();
    let window_ref = window.as_ref().downgrade();
    // Not `on_press`, which would forward a key the composing field already
    // had back to it.
    super::signals::pressed(&keys, move |controller, keyval, keycode, state| {
        if super::composition::composing_field(controller).is_some() {
            return glib::Propagation::Proceed;
        }
        let press = rewrite(
            super::read(controller, keyval, keycode, state),
            context_of_controller(controller),
        );
        let handled = press.unclaimed_alias().is_some_and(|(key, modifiers)| {
            window_ref
                .upgrade()
                .and_then(|window| gtk::prelude::RootExt::focus(&window))
                .is_some_and(|focused| {
                    activate_native(&focused, gdk_key(key), gdk_modifiers(modifiers))
                })
        });
        if handled {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.as_ref().add_controller(keys.clone());
    keys
}

/// Runs what GTK runs for `key` from `focused` outward: a focused widget's
/// activation for Enter, the first matching widget binding, and a popover's
/// own close on Escape.
fn activate_native(focused: &gtk::Widget, key: Key, modifiers: Modifiers) -> bool {
    if key == Key::Return && focused.activate() {
        return true;
    }
    let mut current = Some(focused.clone());
    while let Some(widget) = current {
        if activate_binding(&widget, key, modifiers) {
            return true;
        }
        if key == Key::Escape
            && let Some(popover) = widget.downcast_ref::<gtk::Popover>()
        {
            popover.popdown();
            return true;
        }
        current = widget.parent();
    }
    false
}

fn activate_binding(widget: &gtk::Widget, key: Key, modifiers: Modifiers) -> bool {
    let controllers = widget.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::ShortcutController>()
        })
        .filter(|controller| controller.scope() == gtk::ShortcutScope::Local)
        .any(|controller| {
            (0..controller.n_items())
                .filter_map(|index| controller.item(index).and_downcast::<gtk::Shortcut>())
                .any(|shortcut| {
                    shortcut
                        .trigger()
                        .is_some_and(|trigger| triggered_by(&trigger, key, modifiers))
                        && shortcut.action().is_some_and(|action| {
                            action.activate(
                                gtk::ShortcutActionFlags::EXCLUSIVE,
                                widget,
                                shortcut.arguments().as_ref(),
                            )
                        })
                })
        })
}

fn triggered_by(trigger: &gtk::ShortcutTrigger, key: Key, modifiers: Modifiers) -> bool {
    if let Some(keyval) = trigger.downcast_ref::<gtk::KeyvalTrigger>() {
        return keyval.keyval() == key && keyval.modifiers() == modifiers;
    }
    trigger
        .downcast_ref::<gtk::AlternativeTrigger>()
        .is_some_and(|either| {
            triggered_by(&either.first(), key, modifiers)
                || triggered_by(&either.second(), key, modifiers)
        })
}

#[cfg(test)]
mod tests;
