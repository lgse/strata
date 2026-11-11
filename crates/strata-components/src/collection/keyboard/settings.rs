//! What the keyboard machine reads on every key ([`KeySettings`], which the host passes as a
//! collection setting), and the [`KeyHandle`] it shares with the host: the 10xer range state
//! and the commands the host queues.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use super::command::{Command, RangeKind};
use super::keymap::KeyMap;

/// What plain printable keys (no Ctrl, Alt or Super) do when the map doesn't bind them.
///
/// Used by: C2, C3, C4, and hosts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Letters {
    /// They go on to the host: Strata's default, where Type to search is on and the typed
    /// character starts a search (C18). `h j k l` don't move.
    #[default]
    Pass,
    /// The map's printable aliases act as the arrows: `h j k l` with Type to search off.
    Arrows,
    /// They find an entry by its name's prefix (see the [module docs](super#type-ahead)).
    /// Strata has no counterpart; it is off unless a host asks for it.
    TypeAhead,
}

/// The keyboard machine's settings. The host passes them as a collection setting,
/// `Collection::settings(settings)`, with its current values each time it builds the view, and
/// the machine reads them with `Ctx::settings`; with none, it uses [`KeySettings::default`].
///
/// Fields may be added: start from [`KeySettings::default`] or [`KeySettings::tenxer`].
///
/// Used by: C2, C3, C4, C18, and hosts.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct KeySettings {
    /// Which chords run which commands.
    pub map: KeyMap,
    /// What unbound plain printable keys do.
    pub letters: Letters,
    /// How long after a typed character the next one still extends the type-ahead prefix.
    /// 1 s by default. It is measured on the keys' times, which come from the collection's
    /// clock (`Collection::clock`): without one, every key comes at 0 and a prefix never times
    /// out.
    pub type_ahead_timeout: Duration,
    /// Whether Left and Right cross row ends, to the previous or next entry, when the geometry
    /// has no neighbour that way. On for the icons grid (C3), as Strata's Icons does
    /// (lgse/strata#2044); W1's geometries never wrap themselves. Off for lists and columns,
    /// where Left and Right then reach the view's edge.
    pub wrap_rows: bool,
}

impl Default for KeySettings {
    /// The default map, letters passed on, no wrapping.
    fn default() -> Self {
        Self {
            map: KeyMap::default_map(),
            letters: Letters::Pass,
            type_ahead_timeout: Duration::from_secs(1),
            wrap_rows: false,
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
}

#[derive(Debug, Default)]
struct Shared {
    range: Cell<Option<RangeKind>>,
    queue: RefCell<VecDeque<Command>>,
}

/// One keyboard machine's 10xer range, and commands for it to run, shared with the host.
///
/// Clones share one value. The element keeps its controllers boxed in its `CollectionState`,
/// where the host can't reach them, so the host keeps a clone: it reads the range for the
/// footer, ends it from its Escape ladder, and queues commands that start outside the
/// collection, which run at the collection's next `tick` (C1e's `KeyController::tick`).
/// Settings don't live here: they come through `Ctx::settings`.
///
/// Used by: C2, C3, C4, C18, and hosts.
#[derive(Clone, Debug, Default)]
pub struct KeyHandle(Rc<Shared>);

impl KeyHandle {
    /// A handle with no range and nothing queued.
    pub fn new() -> Self {
        Self::default()
    }

    /// The active 10xer range, for the `VISUAL` and `UNSET` footer (C18) and Columns' mirroring
    /// (C2).
    pub fn range(&self) -> Option<RangeKind> {
        self.0.range.get()
    }

    /// Ends a 10xer range and keeps what it selected: a step of the host's Escape ladder, or
    /// leaving 10xer mode. False if none was active.
    pub fn end_range(&self) -> bool {
        self.0.range.take().is_some()
    }

    pub(super) fn set_range(&self, range: Option<RangeKind>) {
        self.0.range.set(range);
    }

    /// Queues `command` for the machine to run at its next `tick`, as a key would run it: a
    /// 10xer chord step (`g g` is `Command::Cursor(Target::First)`), a window-level Ctrl+A, the
    /// host's Escape ladder. The host then calls the collection's `tick`.
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
