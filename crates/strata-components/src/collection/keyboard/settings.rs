// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use super::command::{Command, RangeKind};
use super::keymap::KeyMap;

/// What plain printable keys (no Ctrl, Alt or Super) do when the map doesn't bind them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Letters {
    /// They go on to the host: Strata's default, where Type to search is on and the typed character starts a search.
    #[default]
    Pass,
    /// The map's printable aliases act as the arrows: `h j k l` with Type to search off.
    Arrows,
    /// They find an entry by its name's prefix (see the module docs).
    TypeAhead,
}

/// The keyboard machine's settings.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct KeySettings {
    /// Which chords run which commands.
    pub map: KeyMap,
    /// What unbound plain printable keys do.
    pub letters: Letters,
    /// How long after a typed character the next one still extends the type-ahead prefix.
    pub type_ahead_timeout: Duration,
    /// Whether Left and Right cross row ends, to the previous or next entry, when the geometry has no neighbour that way.
    pub wrap_rows: bool,
    /// Whether a Select key that cannot move the cursor still selects the cursor's entry alone.
    pub reselects_in_place: bool,
    /// Whether Ctrl+A moves the cursor to the last entry, with the anchor left where it was.
    pub select_all_to_last: bool,
    /// Whether Left with no cursor reports its edge, where it otherwise lands on the last entry.
    pub left_leaves_without_cursor: bool,
    /// Whether multi-selection keys (Shift ranges, Ctrl+Space, Space, `v`/`V`, Ctrl+A, Ctrl+R) may select more than one entry.
    pub multiple: bool,
}

impl Default for KeySettings {
    /// The default map, letters passed on, no wrapping.
    fn default() -> Self {
        Self {
            map: KeyMap::default_map(),
            letters: Letters::Pass,
            type_ahead_timeout: Duration::from_secs(1),
            wrap_rows: false,
            reselects_in_place: false,
            select_all_to_last: false,
            left_leaves_without_cursor: false,
            multiple: true,
        }
    }
}

impl KeySettings {
    /// The 10xer map, with its letters as moves (10xer doesn't use type-to-search).
    pub fn tenxer() -> Self {
        Self {
            map: KeyMap::tenxer(),
            letters: Letters::Arrows,
            ..Self::default()
        }
    }

    /// The same settings with `letters`.
    pub fn letters(mut self, letters: Letters) -> Self {
        self.letters = letters;
        self
    }

    /// The same settings, wrapping Left and Right at row ends or not.
    pub fn wrap_rows(mut self, wrap: bool) -> Self {
        self.wrap_rows = wrap;
        self
    }

    /// The same settings with multiple selection on or off.
    pub fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }
}

#[derive(Debug, Default)]
struct Shared {
    range: Cell<Option<RangeKind>>,
    queue: RefCell<VecDeque<Command>>,
}

/// One keyboard machine's 10xer range, and commands for it to run, shared with the host.
#[derive(Clone, Debug, Default)]
pub struct KeyHandle(Rc<Shared>);

impl KeyHandle {
    /// A handle with no range and nothing queued.
    pub fn new() -> Self {
        Self::default()
    }

    /// The active 10xer range, for the `VISUAL` and `UNSET` footer and Columns' mirroring.
    pub fn range(&self) -> Option<RangeKind> {
        self.0.range.get()
    }

    /// Ends a 10xer range and keeps what it selected: a step of the host's Escape ladder, or leaving 10xer mode.
    pub fn end_range(&self) -> bool {
        self.0.range.take().is_some()
    }

    pub(super) fn set_range(&self, range: Option<RangeKind>) {
        self.0.range.set(range);
    }

    /// Queues `command` for the machine to run at its next `tick`, as a key would run it.
    pub fn run(&self, command: Command) {
        self.0.queue.borrow_mut().push_back(command);
    }

    /// Whether commands wait for the next `tick`.
    pub fn pending(&self) -> bool {
        !self.0.queue.borrow().is_empty()
    }

    pub(super) fn next(&self) -> Option<Command> {
        self.0.queue.borrow_mut().pop_front()
    }
}
