// SPDX-License-Identifier: MIT

//! The one reader of key events. Handlers install through [`on_press`] and
//! [`on_release`], which normalize each event once into a [`KeyPress`]. See
//! `docs/keyboard-shortcuts.md`.
//!
//! The catalog, the tables and `KeyPress` itself are `strata_desktop::keys`, re-exported
//! here at the names they always had. What stays is what reads a GDK event or a widget:
//! this reader, the native aliases, shortcut capture, composition, the conflict check, and
//! the backend that answers the shared code's key questions with GDK and GTK.

use gtk::{
    gdk,
    glib::{self, Propagation, translate::IntoGlib},
    prelude::*,
};
pub(crate) use strata_desktop::keys::{
    Binding, CUSTOM_ACTIONS, Command, CommandKind, DIRECT_IN_TENXER, Digits, Group, Key, KeyPress,
    Keymap, Keys, Map, Match, Matcher, Modifiers, SHORTCUT_MODIFIERS, Shown, TEXT_EDITING,
    bindings, custom_action_slot, digit_label, digit_value, direct_commands, label, label_in,
    label_in_map, modifiers_label, modifiers_label_in, registry, search_text, shown,
};
use strata_desktop::keys::{Context, Event, normalize, rewrite};
#[cfg(test)]
pub(crate) use strata_desktop::keys::{DIRECT_IN_DEFAULT, Letters, main_key, use_keymap};

mod alias;
mod capture;
mod composition;
pub(crate) mod conflicts;
mod gdk_backend;

/// The command registry's tests read key names and accelerators, so they stay with the
/// library that answers until the shared crate has one of its own.
#[cfg(test)]
mod command {
    use strata_desktop::keys::{
        Command, CommandKind as Kind, Group, Keymap, Map, Rejected, registry, use_keymap,
    };

    mod tests;
}

#[cfg(test)]
mod tests;

pub(crate) use alias::install_native as install_native_aliases;
pub(crate) use capture::{Capture, capture};
pub(crate) use gdk_backend::install as install_backend;

impl strata_desktop::keys::Active for crate::ui::preferences::PreferenceManager {
    fn tenxer_mode(&self) -> bool {
        Self::tenxer_mode(self)
    }

    fn type_to_search_active(&self) -> bool {
        Self::type_to_search_active(self)
    }

    fn keymap(&self) -> std::rc::Rc<Keymap> {
        Self::keymap(self)
    }
}

/// Makes the shared preference manager what presses and labels resolve through. A weak
/// reference to the same manager dies exactly when `PreferenceManager::existing()` would
/// find none, and the shipped defaults apply then, as they always did.
pub(crate) fn follow(manager: &std::rc::Rc<crate::ui::preferences::PreferenceManager>) {
    let manager = std::rc::Rc::downgrade(manager);
    let source: std::rc::Weak<dyn strata_desktop::keys::Active> = manager;
    strata_desktop::keys::set_active(source);
}

/// The GDK key of a shared one, for a GTK call.
#[expect(
    unsafe_code,
    reason = "gdk4 has no safe way to make a key from its number"
)]
pub(crate) fn gdk_key(key: Key) -> gdk::Key {
    use glib::translate::FromGlib;
    // SAFETY: a `gdk::Key` is its keysym, and every `u32` is one.
    unsafe { gdk::Key::from_glib(key.keysym()) }
}

pub(crate) fn key_from_gdk(key: gdk::Key) -> Key {
    Key::from_keysym(key.into_glib())
}

/// The GDK flags of shared modifiers, for a GTK call.
pub(crate) fn gdk_modifiers(modifiers: Modifiers) -> gdk::ModifierType {
    gdk::ModifierType::from_bits_truncate(modifiers.bits())
}

/// The held keyboard modifiers of a GDK state. Pointer buttons and the rest are dropped.
pub(crate) fn modifiers_from_gdk(modifiers: gdk::ModifierType) -> Modifiers {
    Modifiers::from_bits_truncate(modifiers.bits())
}

/// While the focused field is composing, the input method sees each key first;
/// the handler gets only keys it passes back, like Escape after a Korean commit.
/// An armed [`capture`] in the controller's window takes every other press.
pub(crate) fn on_press(
    controller: &gtk::EventControllerKey,
    handler: impl Fn(&gtk::EventControllerKey, &KeyPress) -> Propagation + 'static,
) -> glib::SignalHandlerId {
    composition::follow();
    let forwarded = std::rc::Rc::new(composition::Forwarded::default());
    let releases = forwarded.clone();
    signals::released(controller, move |controller, keyval, _, _| {
        releases.release(controller, keyval);
    });
    signals::pressed(controller, move |controller, keyval, keycode, state| {
        if let Some(field) = composition::composing_field(controller) {
            // Forwarding needs a real event, and forwarding from the field's own
            // controller would run that controller again.
            if controller.current_event().is_none() || controller.widget().as_ref() == Some(&field)
            {
                return Propagation::Proceed;
            }
            if controller.forward(&field) {
                forwarded.press(keyval, &field);
                return Propagation::Stop;
            }
        }
        let press = read(controller, keyval, keycode, state);
        if let Some(captured) = capture::deliver(controller, &press) {
            return captured;
        }
        handler(
            controller,
            &rewrite(press, alias::context_of_controller(controller)),
        )
    })
}

/// Releases report what is held, so they are never read through aliases.
pub(crate) fn on_release(
    controller: &gtk::EventControllerKey,
    handler: impl Fn(&gtk::EventControllerKey, &KeyPress) + 'static,
) -> glib::SignalHandlerId {
    signals::released(controller, move |controller, keyval, keycode, state| {
        handler(controller, &read(controller, keyval, keycode, state));
    })
}

/// Reads a key event that reached a non-key controller, such as
/// `EventControllerLegacy`, as `on_press` reads it. Other events give `None`.
pub(crate) fn read_event(
    controller: &impl IsA<gtk::EventController>,
    event: &gdk::Event,
) -> Option<KeyPress> {
    let event = event.downcast_ref::<gdk::KeyEvent>()?;
    Some(rewrite(
        normalize_event(event),
        alias::context_of_controller(controller),
    ))
}

#[expect(
    clippy::disallowed_methods,
    reason = "ui::keys is the only module that reads raw key signals"
)]
mod signals {
    use gtk::{
        gdk::{Key, ModifierType},
        glib::{Propagation, SignalHandlerId},
    };

    pub(super) fn pressed(
        controller: &gtk::EventControllerKey,
        handler: impl Fn(&gtk::EventControllerKey, Key, u32, ModifierType) -> Propagation + 'static,
    ) -> SignalHandlerId {
        controller.connect_key_pressed(handler)
    }

    pub(super) fn released(
        controller: &gtk::EventControllerKey,
        handler: impl Fn(&gtk::EventControllerKey, Key, u32, ModifierType) + 'static,
    ) -> SignalHandlerId {
        controller.connect_key_released(handler)
    }
}

fn read(
    controller: &gtk::EventControllerKey,
    keyval: gdk::Key,
    keycode: u32,
    state: gdk::ModifierType,
) -> KeyPress {
    match controller
        .current_event()
        .and_then(|event| event.downcast::<gdk::KeyEvent>().ok())
    {
        Some(event) => normalize_event(&event),
        None => KeyPress::from_signal(key_from_gdk(keyval), keycode, modifiers_from_gdk(state)),
    }
}

fn normalize_event(event: &gdk::KeyEvent) -> KeyPress {
    normalize(&read_gdk_event(event), |keycode| {
        event
            .display()
            .and_then(|display| display.map_keycode(keycode))
            .into_iter()
            .flatten()
            .map(|(entry, key)| {
                (
                    entry.group() as u32,
                    entry.level() as u32,
                    key_from_gdk(key),
                )
            })
    })
}

fn read_gdk_event(event: &gdk::KeyEvent) -> Event {
    let typed = event.keyval();
    let state = event.modifier_state();
    let (key, modifiers) = event.match_().unwrap_or((typed.to_lower(), state));
    Event {
        matched: (key_from_gdk(key), modifiers_from_gdk(modifiers)),
        typed: key_from_gdk(typed),
        state: modifiers_from_gdk(state),
        keycode: event.keycode(),
        layout: event.layout(),
        level: event.level(),
        modifier: event.is_modifier(),
    }
}
