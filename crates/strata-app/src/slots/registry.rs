// SPDX-License-Identifier: MIT

//! The table of registered parts, and what the shell asks of it.

use std::{any::TypeId, cell::Cell, rc::Rc};

use rune::Tx;

use strata_fs::app::{Browser, BrowserEvent};

use super::{Heard, Listener, Part, Place, Setup, Sidebar, Slot, View};
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
    pub(crate) switched: fn(&mut S, &mut Tx),
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
            switched: P::switched,
            commands: P::COMMANDS,
            keyboard: P::keyboard,
        }
    }
}

impl<S> Entry<S> {
    pub(crate) const fn slot(&self) -> Slot {
        self.place.slot()
    }

    pub(crate) fn view(&self) -> Option<&View<S>> {
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

    /// Shows the view that answers to `mode` in place of the one shown. False when none
    /// answers to it, or when it is shown already.
    pub(crate) fn show(&mut self, mode: &str) -> bool {
        let named = self
            .entries
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.view().is_some_and(|view| (view.shows)(mode)));
        match named {
            Some((index, _)) if self.shown != Some(index) => {
                self.shown = Some(index);
                true
            }
            _ => false,
        }
    }

    /// Which view is shown, as the parts that read it later share it: the view's listener
    /// ([`Registry::listeners`]) and a part's timer ([`ShownView::is`]).
    pub(crate) fn shown_now(self) -> ShownView {
        let shown = ShownView(Rc::new(Cell::new(None)));
        shown.follow(self);
        shown
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

    /// Every part's listener, in registration order. A view's hears only while it is the one
    /// [`Setup::shown`] names: a view that is not shown misses the browser's events, and is
    /// rebuilt from the browser when it is shown.
    pub(crate) fn listeners(self, setup: &Setup<'_>) -> Vec<Listener<S>>
    where
        S: 'static,
    {
        let entries = self.entries.iter().enumerate();
        entries
            .filter_map(|(index, entry)| {
                let listener = (entry.listen)(setup)?;
                if entry.view().is_none() {
                    return Some(listener);
                }
                Some(setup.shown.gate(index, listener))
            })
            .collect()
    }
}

/// Which registered view is shown, for what reads it after the start: a view's listener, which
/// hears the browser only while its view is shown, and a part's timer, which asks at the time
/// it fires, as GTK's `child_mirror_blocked` asks for the mode. The shell keeps it with the
/// registry and moves both together ([`ShownView::follow`]).
#[derive(Clone)]
pub(crate) struct ShownView(Rc<Cell<Option<(usize, TypeId)>>>);

impl ShownView {
    /// `listener`, heard only while the view registered at `index` is the one shown.
    fn gate<S: 'static>(&self, index: usize, listener: Listener<S>) -> Listener<S> {
        let shown = self.0.clone();
        Box::new(
            move |event: &BrowserEvent, browser: &Browser, heard: &mut Heard<'_, S>| {
                if shown.get().is_some_and(|(shown, _)| shown == index) {
                    listener(event, browser, heard);
                }
            },
        )
    }

    /// Whether the view shown now is part `P`.
    pub(crate) fn is<P: 'static>(&self) -> bool {
        self.0
            .get()
            .is_some_and(|(_, kind)| kind == TypeId::of::<P>())
    }

    /// From now on the view `registry` shows is the one shown: it hears the browser, and no
    /// other does.
    pub(crate) fn follow<S>(&self, registry: Registry<'_, S>) {
        let shown = registry
            .shown
            .and_then(|index| Some((index, (registry.entries.get(index)?.kind)())));
        self.0.set(shown);
    }
}
