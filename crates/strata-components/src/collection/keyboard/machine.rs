//! The keyboard state machine: keys in, selection operations and [`Outcome`]s out. Plain data,
//! no Rune elements.

use rune::{Key, KeyEvent};

use super::command::{Command, RangeKind, Target, Vertical};
use super::settings::{KeyHandle, KeySettings, Letters};
use super::type_ahead::TypeAhead;
use crate::collection::element::KeyInput;
use crate::collection::selection::{AnchorRule, Changed, Extend, Selection};
use crate::contract::{CollectionGeometry, Direction, EntryId, EntrySource};

/// What the machine asks for besides selection changes. What each means is the host's policy.
///
/// Variants may be added.
///
/// Used by: the C1e adapter, C2, C18, hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    /// Scroll the entry at this index into view: the cursor after a move.
    Reveal(usize),
    /// Open the entry (Enter, 10xer `o`).
    Activate {
        /// The entry.
        id: EntryId,
    },
    /// The cursor can't move this way inside the collection. The host decides: the sidebar or
    /// the pane header (unless Keep arrows in file list is on), the next Columns column (C2),
    /// the parent folder or the folder under the cursor (10xer List and Columns), or nothing
    /// (List's Right). While a key repeats, an edge is reported once on arrival, not at every
    /// repeat; a new press reports it again.
    Edge(Direction),
    /// The key was keyboard navigation that moved no cursor (Ctrl+A, Escape): the cursor
    /// outline shows, as Strata's `keyboard_navigation` turns it on.
    Keyboard,
    /// A 10xer selection key in an empty folder: the host flashes `Nothing to select` (C18).
    NothingToSelect,
}

/// What the machine reads and changes: the source, its geometry, its settings, the selection,
/// and where its outcomes go. The adapter ([`Keyboard`](super::Keyboard)) makes one of C1e's
/// `Ctx`; tests use a plain one.
///
/// Used by: the adapter, and views that drive the machine themselves.
pub trait Surface {
    /// The settings: the host's collection setting, else the defaults.
    fn settings(&self) -> &KeySettings;

    /// The entries, in display order.
    fn source(&self) -> &dyn EntrySource;

    /// Where they sit, mounted or not.
    fn geometry(&self) -> &dyn CollectionGeometry;

    /// The selection, to read.
    fn selection(&self) -> &Selection;

    /// Runs one selection operation, and returns what it changed.
    fn apply(&mut self, op: &mut dyn FnMut(&mut Selection, &dyn EntrySource) -> Changed)
    -> Changed;

    /// Hands on an outcome.
    fn emit(&mut self, outcome: Outcome);
}

/// Where a target lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Landing {
    At(usize),
    Edge(Direction),
    Nowhere,
}

/// The keyboard state machine of one collection.
///
/// Feed it each key with [`KeyMachine::key`], and run [`KeyMachine::tick`] when the host has
/// queued commands on its [`KeyHandle`]. It changes the selection through the [`Surface`] and
/// emits [`Outcome`]s for the rest. The [module docs](super) give the rules.
///
/// Used by: the adapter ([`Keyboard`](super::Keyboard)).
#[derive(Debug)]
pub struct KeyMachine {
    handle: KeyHandle,
    type_ahead: TypeAhead,
    /// The selection's revision after the machine's own last change during a 10xer range. A
    /// different revision means something else (a click) changed the selection, which ends
    /// the range, as a pointer change ends one in Strata.
    range_revision: u64,
    /// Whether the key being handled is a repeat.
    repeating: bool,
    /// The edge the last command reported, and the cursor it was reported at.
    last_edge: Option<(Direction, Option<EntryId>)>,
}

impl Default for KeyMachine {
    /// A machine with a handle of its own.
    fn default() -> Self {
        Self::new(KeyHandle::new())
    }
}

impl KeyMachine {
    /// A machine sharing `handle` with the host.
    pub fn new(handle: KeyHandle) -> Self {
        Self {
            handle,
            type_ahead: TypeAhead::default(),
            range_revision: 0,
            repeating: false,
            last_edge: None,
        }
    }

    /// The handle it reads.
    pub fn handle(&self) -> &KeyHandle {
        &self.handle
    }

    /// Handles one key. True when the machine took it; false lets the page and Rune hear it.
    pub fn key(&mut self, input: &KeyInput<'_>, s: &mut dyn Surface) -> bool {
        let settings = s.settings();
        let command = settings.map.lookup(input, settings.letters);
        let (letters, timeout) = (settings.letters, settings.type_ahead_timeout);
        if let Some(command) = command {
            self.type_ahead.reset();
            self.repeating = input.repeat;
            let taken = self.run(command, s);
            self.repeating = false;
            return taken;
        }
        if letters != Letters::TypeAhead {
            return false;
        }
        let Some(c) = printable(input.event) else {
            return false;
        };
        if c == ' ' && !self.type_ahead.open(input.now, timeout) {
            return false;
        }
        let cursor = s.selection().cursor_index(s.source());
        if let Some(i) = self
            .type_ahead
            .push(c, input.now, timeout, s.source(), cursor)
        {
            op(s, |sel, src| sel.select_only(src, i));
            s.emit(Outcome::Reveal(i));
        }
        true
    }

    /// A primary press on the collection: it ends a 10xer range, as a click does in Strata, at
    /// once, so the footer is right. (Any other change to the selection the machine didn't
    /// make ends the range at its next key, by the selection's revision.)
    pub fn pressed(&mut self) {
        self.handle.set_range(None);
    }

    /// Drops what is in progress: the type-ahead prefix, a 10xer range, and commands still
    /// queued on the handle. The collection shows another folder.
    pub fn reset(&mut self) {
        self.type_ahead.reset();
        self.handle.set_range(None);
        while self.handle.next().is_some() {}
        self.last_edge = None;
    }

    /// Runs the commands the host queued on the handle ([`KeyHandle::run`]). True when one
    /// of them did something.
    pub fn tick(&mut self, s: &mut dyn Surface) -> bool {
        let mut handled = false;
        while let Some(command) = self.handle.next() {
            handled |= self.run(command, s);
        }
        handled
    }

    /// Runs `command` as a key bound to it would. True when it took the key.
    pub fn run(&mut self, command: Command, s: &mut dyn Surface) -> bool {
        if self.handle.range().is_some() && s.selection().revision() != self.range_revision {
            self.handle.set_range(None);
        }
        if !keeps_range(command) {
            self.handle.set_range(None);
        }
        let edge_before = self.last_edge.take();
        let taken = self.dispatch(command, edge_before, s);
        self.range_revision = s.selection().revision();
        taken
    }

    /// Reports `d` as an edge, unless the key is a repeat that reached the same edge at the
    /// same cursor last time.
    fn edge(
        &mut self,
        d: Direction,
        before: Option<(Direction, Option<EntryId>)>,
        s: &mut dyn Surface,
    ) {
        let at = (d, s.selection().cursor());
        if !(self.repeating && before == Some(at)) {
            s.emit(Outcome::Edge(d));
        }
        self.last_edge = Some(at);
    }

    /// Moves the cursor to `i`, keeping the selection, or walks an active 10xer range there.
    fn cursor_to(&self, i: usize, s: &mut dyn Surface) {
        match self.handle.range() {
            Some(kind) => op(s, |sel, src| sel.extend_to(src, i, kind.extend())),
            None => op(s, |sel, src| sel.move_cursor(src, i, AnchorRule::Keep)),
        };
        s.emit(Outcome::Reveal(i));
    }

    fn dispatch(
        &mut self,
        command: Command,
        edge_before: Option<(Direction, Option<EntryId>)>,
        s: &mut dyn Surface,
    ) -> bool {
        let len = s.source().len();
        match command {
            Command::Select(target) => {
                match self.land(target, s) {
                    Landing::At(i) => {
                        op(s, |sel, src| sel.select_only(src, i));
                        s.emit(Outcome::Reveal(i));
                    }
                    Landing::Edge(d) => self.edge(d, edge_before, s),
                    Landing::Nowhere => {}
                }
                true
            }
            Command::Extend(target) => self.extend(target, s),
            Command::Cursor(target) => {
                match self.land(target, s) {
                    Landing::At(i) => self.cursor_to(i, s),
                    Landing::Edge(d) => self.edge(d, edge_before, s),
                    Landing::Nowhere => {}
                }
                true
            }
            Command::Nudge(d) => match self.land(Target::Step(d), s) {
                Landing::At(i) => {
                    self.cursor_to(i, s);
                    true
                }
                // Not taken at an edge, so a Columns column hands Ctrl+Left and Ctrl+Right to
                // the view, as Strata's directory_navigation proceeds with Ctrl held.
                Landing::Edge(_) | Landing::Nowhere => false,
            },
            Command::Toggle => {
                if len > 0 {
                    let i = cursor_or_first(s);
                    op(s, |sel, src| sel.toggle(src, i));
                    s.emit(Outcome::Reveal(i));
                }
                true
            }
            Command::ToggleAdvance => {
                if len == 0 {
                    s.emit(Outcome::NothingToSelect);
                } else if self.handle.range().is_some() {
                    // Inside a `v`/`V` range, Space flips the cursor's entry against the range
                    // and stays put; later steps keep the flip (Strata's
                    // `toggle_visual_cursor`).
                    let i = cursor_or_first(s);
                    op(s, |sel, src| sel.toggle_in_run(src, i));
                    s.emit(Outcome::Reveal(i));
                } else {
                    toggle_advance(s);
                }
                true
            }
            Command::SelectAll => {
                if len == 0 {
                    s.emit(Outcome::NothingToSelect);
                } else {
                    if op(s, |sel, src| sel.select_all(src)).cursor {
                        s.emit(Outcome::Reveal(0));
                    }
                    s.emit(Outcome::Keyboard);
                }
                true
            }
            Command::Invert => {
                if len == 0 {
                    s.emit(Outcome::NothingToSelect);
                } else {
                    op(s, |sel, src| sel.invert(src));
                }
                true
            }
            Command::Range(kind) => {
                self.range(kind, s);
                true
            }
            Command::Clear => {
                if s.selection().is_empty() {
                    return false;
                }
                op(s, |sel, _| sel.clear());
                s.emit(Outcome::Keyboard);
                true
            }
            Command::Activate => activate(s),
            Command::Swallow => true,
        }
    }

    /// Shift: the range from the anchor, as Strata's `extend_selection` and
    /// `extend_page_selection` do. Where there is no anchor (or no cursor), the anchor moves to
    /// the cursor, or to the end the move starts from. With nothing selected, it moves there
    /// too, and a one-entry step then selects that entry alone without moving, while a page or
    /// a jump extends from it at once.
    ///
    /// A sideways step with no neighbour isn't taken (false), so the view decides: Columns'
    /// Shift+Right enters the folder and Shift+Left opens the parent (C2), and List does
    /// nothing, as Strata keeps a Shift-held sideways arrow there.
    fn extend(&mut self, target: Target, s: &mut dyn Surface) -> bool {
        let len = s.source().len();
        if len == 0 {
            return !matches!(target, Target::Step(Direction::Left | Direction::Right));
        }
        let cursor = s.selection().cursor_index(s.source());
        let origin = cursor.unwrap_or(if forward(target) { 0 } else { len - 1 });
        if let Target::Step(d @ (Direction::Left | Direction::Right)) = target
            && !matches!(self.step(origin, d, s), Landing::At(_))
        {
            return false;
        }
        let empty = s.selection().is_empty();
        let anchored = s.selection().anchor_index(s.source()).is_some();
        if empty || !anchored || cursor.is_none() {
            op(s, |sel, src| sel.move_cursor(src, origin, AnchorRule::Move));
        }
        let to = match (target, empty) {
            (Target::Step(_), true) => origin,
            _ => match self.land(target, s) {
                Landing::At(i) => i,
                Landing::Edge(_) | Landing::Nowhere => origin,
            },
        };
        op(s, |sel, src| sel.extend_to(src, to, Extend::Replace));
        s.emit(Outcome::Reveal(to));
        true
    }

    /// `v` or `V`: starts a range at the cursor, switches its kind, or ends one of this kind.
    fn range(&mut self, kind: RangeKind, s: &mut dyn Surface) {
        if self.handle.range() == Some(kind) {
            self.handle.set_range(None);
            return;
        }
        if s.source().is_empty() {
            self.handle.set_range(None);
            s.emit(Outcome::NothingToSelect);
            return;
        }
        // A range starts at the cursor; with none there is nothing to select (Strata's
        // `start_visual`). A load's automatic pick isn't part of the range's base.
        let Some(i) = s.selection().cursor_index(s.source()) else {
            self.handle.set_range(None);
            s.emit(Outcome::NothingToSelect);
            return;
        };
        op(s, |sel, _| sel.discard_provisional());
        op(s, |sel, src| sel.move_cursor(src, i, AnchorRule::Move));
        op(s, |sel, src| sel.extend_to(src, i, kind.extend()));
        self.handle.set_range(Some(kind));
        s.emit(Outcome::Reveal(i));
    }

    /// Where `target` lands from the cursor, through the geometry.
    fn land(&self, target: Target, s: &dyn Surface) -> Landing {
        let len = s.source().len();
        if len == 0 {
            return match target {
                Target::Step(d) => Landing::Edge(d),
                _ => Landing::Nowhere,
            };
        }
        let last = len - 1;
        let cursor = s.selection().cursor_index(s.source());
        let page = s.geometry().page_step().max(1);
        let step = |v: Vertical, by: usize| match (cursor, v) {
            (None, Vertical::Down) => 0,
            (None, Vertical::Up) => last,
            (Some(i), Vertical::Down) => i.saturating_add(by).min(last),
            (Some(i), Vertical::Up) => i.saturating_sub(by),
        };
        match target {
            Target::First => Landing::At(0),
            Target::Last => Landing::At(last),
            Target::Page(v) => Landing::At(step(v, page)),
            Target::HalfPage(v) => Landing::At(step(v, (page / 2).max(1))),
            Target::Step(d) => {
                let Some(i) = cursor else {
                    return Landing::At(match d {
                        Direction::Down | Direction::Right => 0,
                        Direction::Up | Direction::Left => last,
                    });
                };
                self.step(i, d, s)
            }
        }
    }

    /// The entry next to `i` in direction `d`: the geometry's neighbour, else across a row
    /// end with `wrap_rows`, else the edge.
    fn step(&self, i: usize, d: Direction, s: &dyn Surface) -> Landing {
        if let Some(next) = s.geometry().neighbor(i, d) {
            return Landing::At(next);
        }
        let wrap = s.settings().wrap_rows;
        let last = s.source().len().saturating_sub(1);
        match d {
            Direction::Left if wrap && i > 0 => Landing::At(i - 1),
            Direction::Right if wrap && i < last => Landing::At(i + 1),
            _ => Landing::Edge(d),
        }
    }
}

/// Runs one selection operation on the surface.
fn op(
    s: &mut dyn Surface,
    mut f: impl FnMut(&mut Selection, &dyn EntrySource) -> Changed,
) -> Changed {
    s.apply(&mut f)
}

/// Whether `command` leaves an active 10xer range going: moves walk it, Space flips inside
/// it, `v` and `V` end or switch it themselves.
fn keeps_range(command: Command) -> bool {
    matches!(
        command,
        Command::Cursor(_)
            | Command::ToggleAdvance
            | Command::Range(_)
            | Command::Activate
            | Command::Swallow
            | Command::Nudge(_)
    )
}

/// Whether a target moves towards the last entry, so a range with no cursor starts at the
/// first.
fn forward(target: Target) -> bool {
    match target {
        Target::Step(d) => matches!(d, Direction::Down | Direction::Right),
        Target::Page(v) | Target::HalfPage(v) => v == Vertical::Down,
        Target::First => false,
        Target::Last => true,
    }
}

fn cursor_or_first(s: &dyn Surface) -> usize {
    s.selection().cursor_index(s.source()).unwrap_or(0)
}

/// 10xer Space outside a range: the first Space adds the cursor's entry even over a load's
/// automatic pick, then the cursor moves to the next entry in display order
/// (`toggle_cursor_and_advance`: `move_displayed_cursor(1, 1)`).
fn toggle_advance(s: &mut dyn Surface) {
    let len = s.source().len();
    let i = cursor_or_first(s);
    op(s, |sel, _| sel.discard_provisional());
    op(s, |sel, src| sel.toggle(src, i));
    let next = if i + 1 < len { i + 1 } else { i };
    if next != i {
        op(s, |sel, src| sel.move_cursor(src, next, AnchorRule::Keep));
    }
    s.emit(Outcome::Reveal(next));
}

/// Enter: opens the cursor's entry. With no cursor, the first entry takes the cursor and the
/// selection and opens (a recorded decision, from Strata's grouped-List rule that Return
/// starts at the first row shown). An empty collection leaves the key to the host.
fn activate(s: &mut dyn Surface) -> bool {
    let i = match s.selection().cursor_index(s.source()) {
        Some(i) => i,
        None if s.source().is_empty() => return false,
        None => {
            op(s, |sel, src| sel.select_only(src, 0));
            s.emit(Outcome::Reveal(0));
            0
        }
    };
    let id = s.source().id_at(i);
    s.emit(Outcome::Activate { id });
    true
}

/// The character a plain printable key types: no Ctrl, Alt or Super, Shift allowed.
fn printable(event: &KeyEvent) -> Option<char> {
    let m = event.mods;
    if m.cmd || m.ctrl || m.alt || m.logo {
        return None;
    }
    let Key::Text(text) = &event.key else {
        return None;
    };
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_control() => Some(c),
        _ => None,
    }
}
