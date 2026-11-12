// SPDX-License-Identifier: MIT

//! The table of registered parts, and what the shell asks of it.

use std::any::TypeId;

use rune::Tx;

use super::{Listener, Part, Place, Setup, Sidebar, Slot, View};
use crate::{
    App,
    commands::{Action, Holds},
};

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
    pub(crate) commands: &'static [Action<S>],
    pub(crate) keyboard: fn(&S) -> Option<Holds>,
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
            commands: P::COMMANDS,
            keyboard: P::keyboard,
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

/// Whoever registers commands: a part, or the window itself.
struct Holder<'a, S: 'static> {
    name: &'static str,
    /// Views are shown one at a time, so two of them may each register a command.
    view: bool,
    commands: &'a [Action<S>],
}

impl<'a, S> Holder<'a, S> {
    fn of(entry: &'a Entry<S>) -> Self {
        Self {
            name: entry.name,
            view: entry.view().is_some(),
            commands: entry.commands,
        }
    }

    fn window(own: &'a [Action<S>]) -> Self {
        Self {
            name: "window",
            view: false,
            commands: own,
        }
    }

    fn has(&self, id: &str) -> bool {
        self.commands.iter().any(|action| action.id == id)
    }

    /// A command this one registers twice.
    fn repeats_itself(&self) -> Option<Fault> {
        let mut commands = self.commands.iter().enumerate();
        let twice = commands.find(|(at, action)| {
            let earlier = &self.commands[..*at];
            earlier.iter().any(|earlier| earlier.id == action.id)
        });
        twice.map(|(_, action)| Fault::CommandTwice(action.id, self.name, self.name))
    }

    /// A command this one registers that `earlier` has, where both can be active at once.
    fn repeats(&self, earlier: &Self) -> Option<Fault> {
        if self.view && earlier.view {
            return None;
        }
        let twice = self.commands.iter().find(|action| earlier.has(action.id));
        twice.map(|action| Fault::CommandTwice(action.id, earlier.name, self.name))
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
    /// This command is registered twice, by these two (the window's own table is `window`),
    /// where both can be active at once: a menu row or a button would run only the first.
    CommandTwice(&'static str, &'static str, &'static str),
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

    /// What is wrong with `entries`, if anything: the first fault in registration order, and
    /// then the first command registered twice, among the parts and the window's `own`.
    pub(crate) fn check(entries: &[Entry<S>], own: &[Action<S>]) -> Result<(), Fault> {
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
        let parts = entries.iter().map(Holder::of);
        let holders: Vec<Holder<'_, S>> = parts.chain([Holder::window(own)]).collect();
        match holders.iter().enumerate().find_map(|(index, holder)| {
            holder.repeats_itself().or_else(|| {
                holders[..index]
                    .iter()
                    .find_map(|earlier| holder.repeats(earlier))
            })
        }) {
            Some(fault) => Err(fault),
            None => Ok(()),
        }
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

    /// The sidebar, when one is registered.
    pub(crate) fn sidebar(self) -> Option<&'a Sidebar<S>> {
        self.active().find_map(|entry| match &entry.place {
            Place::Sidebar(sidebar) => Some(sidebar),
            _ => None,
        })
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
