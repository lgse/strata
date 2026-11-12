// SPDX-License-Identifier: MIT

//! The table of registered parts, and what the shell asks of it.

use std::any::TypeId;

use rune::Tx;

use super::{Listener, Part, Place, Setup, Slot, View};
use crate::App;

/// One registered part: its name, its place, and its hooks.
pub(crate) struct Entry<S: 'static> {
    /// The field of the application state that holds the part.
    pub(crate) name: &'static str,
    pub(crate) kind: fn() -> TypeId,
    pub(crate) place: Place<S>,
    pub(crate) listen: fn(&Setup<'_>) -> Option<Listener<S>>,
    pub(crate) publish: fn(&mut S),
    pub(crate) caught_up: fn(&mut S),
    pub(crate) poll: fn(&mut S, f64, &mut Tx) -> bool,
    pub(crate) holds_keyboard: fn(&S) -> bool,
}

impl Entry<App> {
    /// The entry of part `P`, registered as the field `name`.
    pub(crate) const fn of<P: Part>(name: &'static str) -> Self {
        Self {
            name,
            kind: TypeId::of::<P>,
            place: P::PLACE,
            listen: P::listen,
            publish: P::publish,
            caught_up: P::caught_up,
            poll: P::poll,
            holds_keyboard: P::holds_keyboard,
        }
    }
}

impl<S> Entry<S> {
    pub(crate) const fn slot(&self) -> Slot {
        self.place.slot()
    }

    fn view(&self) -> Option<&View<S>> {
        match &self.place {
            Place::View(view) => Some(view),
            _ => None,
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
            entries.filter_map(|(index, entry)| Some((index, entry.view()?)))
        };
        let named = views().find(|(_, view)| (view.shows)(mode));
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
                if earlier.slot() == entry.slot() && entry.slot().holds_one() {
                    return Err(Fault::Crowded(entry.slot(), earlier.name, entry.name));
                }
            }
        }
        Ok(())
    }

    /// Every part whose hooks run, in registration order: all but the views not shown.
    pub(crate) fn active(self) -> impl Iterator<Item = &'a Entry<S>> {
        let entries = self.entries.iter().enumerate();
        entries
            .filter(move |(index, entry)| entry.view().is_none() || self.shown == Some(*index))
            .map(|(_, entry)| entry)
    }

    /// The active parts of `slot`, in registration order. None for a slot nothing is
    /// registered in.
    pub(crate) fn of(self, slot: Slot) -> impl Iterator<Item = &'a Entry<S>> {
        self.active().filter(move |entry| entry.slot() == slot)
    }

    /// The view that is shown. None when no view is registered.
    pub(crate) fn shown(self) -> Option<&'a View<S>> {
        self.entries.get(self.shown?)?.view()
    }

    /// The drawer's wrap around the files, when a drawer is registered.
    pub(crate) fn drawer(self) -> Option<super::Wrap<S>> {
        self.active().find_map(|entry| match entry.place {
            Place::Drawer(wrap) => Some(wrap),
            _ => None,
        })
    }

    /// The active parts' listeners, in registration order.
    pub(crate) fn listeners(self, setup: &Setup<'_>) -> Vec<Listener<S>> {
        self.active()
            .filter_map(|entry| (entry.listen)(setup))
            .collect()
    }
}
