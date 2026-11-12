// SPDX-License-Identifier: MIT

//! The table of registered parts, and what the shell asks of it.

use std::any::TypeId;

use rune::{El, Tx};

use super::{Frame, Listener, Part, Setup, Slot};
use crate::App;

/// One registered part: its name, its slot, and its hooks.
pub(crate) struct Entry<S: 'static> {
    /// The field of the application state that holds the part.
    pub(crate) name: &'static str,
    pub(crate) slot: Slot,
    pub(crate) focus: Option<&'static str>,
    pub(crate) kind: fn() -> TypeId,
    pub(crate) shows: fn(&str) -> bool,
    pub(crate) listen: fn(&Setup<'_>) -> Option<Listener<S>>,
    pub(crate) build: fn(&S, &mut Frame<'_, S>) -> Option<El>,
    pub(crate) wrap: fn(&S, El, &mut Frame<'_, S>) -> El,
    pub(crate) publish: fn(&mut S),
    pub(crate) caught_up: fn(&mut S),
    pub(crate) poll: fn(&mut S, f64, &mut Tx) -> bool,
    pub(crate) pointer_at: fn(&mut S, (f32, f32)),
    pub(crate) holds_keyboard: fn(&S) -> bool,
}

impl Entry<App> {
    /// The entry of part `P`, registered as the field `name`.
    pub(crate) const fn of<P: Part>(name: &'static str) -> Self {
        Self {
            name,
            slot: P::SLOT,
            focus: P::FOCUS,
            kind: TypeId::of::<P>,
            shows: P::shows,
            listen: P::listen,
            build: P::build,
            wrap: P::wrap,
            publish: P::publish,
            caught_up: P::caught_up,
            poll: P::poll,
            pointer_at: P::pointer_at,
            holds_keyboard: P::holds_keyboard,
        }
    }
}

/// Why a table of parts cannot be a window.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    /// Two lines register this field.
    NamedTwice(&'static str),
    /// These two fields hold the same part: its element keys would be in the tree twice.
    SamePart(&'static str, &'static str),
    /// These two parts are in a slot that holds one.
    Crowded(Slot, &'static str, &'static str),
}

/// The registered parts, and which of the views is shown.
pub(crate) struct Registry<'a, S: 'static> {
    entries: &'a [Entry<S>],
    shown: Option<usize>,
}

impl<S> Clone for Registry<'_, S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Registry<'_, S> {}

impl<'a, S> Registry<'a, S> {
    /// The table with the view the saved `mode` names shown, or the first registered view
    /// when none answers to it.
    pub(crate) fn new(entries: &'a [Entry<S>], mode: &str) -> Self {
        let views = || {
            let entries = entries.iter().enumerate();
            entries.filter(|(_, entry)| entry.slot == Slot::View)
        };
        let named = views().find(|(_, entry)| (entry.shows)(mode));
        let shown = named.or_else(|| views().next()).map(|(index, _)| index);
        Self { entries, shown }
    }

    /// What is wrong with `entries`, if anything: the first fault in registration order.
    pub(crate) fn check(entries: &[Entry<S>]) -> Result<(), Fault> {
        for (index, entry) in entries.iter().enumerate() {
            for earlier in &entries[..index] {
                if earlier.name == entry.name {
                    return Err(Fault::NamedTwice(entry.name));
                }
                if (earlier.kind)() == (entry.kind)() {
                    return Err(Fault::SamePart(earlier.name, entry.name));
                }
                if earlier.slot == entry.slot && entry.slot.holds_one() {
                    return Err(Fault::Crowded(entry.slot, earlier.name, entry.name));
                }
            }
        }
        Ok(())
    }

    /// Every part whose hooks run, in registration order: all but the views not shown.
    pub(crate) fn active(self) -> impl Iterator<Item = &'a Entry<S>> {
        let entries = self.entries.iter().enumerate();
        entries
            .filter(move |(index, entry)| entry.slot != Slot::View || self.shown == Some(*index))
            .map(|(_, entry)| entry)
    }

    /// The active parts of `slot`, in registration order. None for a slot nothing is
    /// registered in.
    pub(crate) fn of(self, slot: Slot) -> impl Iterator<Item = &'a Entry<S>> {
        self.active().filter(move |entry| entry.slot == slot)
    }

    /// The key of the element that takes the keyboard for `slot`.
    pub(crate) fn focus(self, slot: Slot) -> Option<&'static str> {
        self.of(slot).find_map(|entry| entry.focus)
    }

    /// The active parts' listeners, in registration order.
    pub(crate) fn listeners(self, setup: &Setup<'_>) -> Vec<Listener<S>> {
        self.active()
            .filter_map(|entry| (entry.listen)(setup))
            .collect()
    }
}
