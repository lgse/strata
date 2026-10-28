// SPDX-License-Identifier: MIT

//! Escape, Enter, Tab and the arrows are structural commands: a press of one
//! of their other bindings is read as the canonical key before any handler
//! sees it, so every surface that matches the command follows it. See
//! `docs/keyboard-shortcuts.md`.

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib,
    prelude::*,
};

use super::{
    Binding, Command, KeyPress, Map, Match, bindings,
    command::{active_keymap, registry},
};

/// Escape always matches `escape`, even where a table leaves it out.
pub(super) const FALLBACK: Command = bindings::ESCAPE;

/// What owns typing where a press lands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Context {
    /// A focused text field or popover, or type-to-search, reads plain
    /// printable keys as its own. In 10xer a read-only text view, such as a
    /// text preview, types nothing; the default map keeps treating it as a
    /// field.
    pub(crate) typing: bool,
}

impl Context {
    pub(crate) fn of(focused: Option<&gtk::Widget>, type_to_search: bool) -> Self {
        let tenxer = Map::active() == Map::Tenxer;
        let owns_typing = |widget: &gtk::Widget| {
            (if tenxer {
                crate::ui::focus_navigation::accepts_typing(widget)
            } else {
                crate::ui::focus_navigation::editable(widget)
            }) || crate::ui::focus_navigation::in_popover(widget)
        };
        Self {
            typing: type_to_search || focused.is_some_and(owns_typing),
        }
    }

    pub(super) fn of_controller(controller: &impl IsA<gtk::EventController>) -> Self {
        let focused = controller
            .widget()
            .and_then(|widget| widget.root())
            .and_then(|root| root.focus());
        let type_to_search = crate::ui::preferences::PreferenceManager::existing()
            .is_some_and(|preferences| preferences.type_to_search_active());
        Self::of(focused.as_ref(), type_to_search)
    }

    fn aliasing(self) -> Aliasing {
        if self.typing {
            Aliasing::Untyped
        } else {
            Aliasing::All
        }
    }
}

/// Which alternates a press's context lets through. `Off` is a press that
/// did not come through `on_press`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Aliasing {
    Off,
    Untyped,
    All,
}

impl Aliasing {
    pub(super) fn allows(self, typed: bool) -> bool {
        match self {
            Self::Off => false,
            Self::Untyped => !typed,
            Self::All => true,
        }
    }
}

/// The key a rewritten press came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Alias {
    pub(super) key: Key,
    pub(super) modifiers: Modifiers,
    /// A plain printable alternate, such as **j** for ↓.
    pub(super) typed: bool,
}

fn structural() -> impl Iterator<Item = (Command, Binding)> {
    registry().filter_map(|command| command.canonical().map(|canonical| (command, canonical)))
}

/// Reads `press` as the canonical key of the structural command whose
/// alternate it matches exactly in the active map, where `context` lets that
/// alternate through. A canonical key, with any modifiers, is never rewritten.
pub(crate) fn rewrite(press: KeyPress, context: Context) -> KeyPress {
    let aliasing = context.aliasing();
    let press = KeyPress { aliasing, ..press };
    if press.is_modifier() || press.aliased() {
        return press;
    }
    if structural().any(|(_, canonical)| canonical.key() == press.key) {
        return press;
    }
    let keymap = active_keymap();
    let map = Map::active();
    for (command, canonical) in structural() {
        let alternate = keymap.bindings(map, command).iter().find(|binding| {
            **binding != canonical && aliasing.allows(binding.is_typed()) && binding.matches(&press)
        });
        if let Some(alternate) = alternate {
            return press.read_as(canonical, alternate.is_typed());
        }
    }
    press
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
            Context::of_controller(controller),
        );
        let handled = press.alias.is_some_and(|alias| !alias.typed)
            && window_ref
                .upgrade()
                .and_then(|window| gtk::prelude::RootExt::focus(&window))
                .is_some_and(|focused| {
                    activate_native(&focused, press.key, press.shortcut_modifiers)
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
