// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent};

use super::command::{Command, RangeKind, Target, Vertical};
use super::settings::{KeyHandle, KeySettings, Letters};
use super::type_ahead::TypeAhead;
use crate::collection::element::KeyInput;
use crate::collection::selection::{AnchorRule, Changed, Extend, Selection};
use crate::contract::{CollectionGeometry, Direction, EntryId, EntrySource};

/// What the machine asks for besides selection changes.
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
    /// The cursor can't move this way inside the collection.
    Edge(Direction),
    /// The key was keyboard navigation that moved no cursor (Ctrl+A, Escape).
    Keyboard,
    /// A 10xer selection key in an empty folder: the host flashes `Nothing to select`.
    NothingToSelect,
    /// A Select key ran and the cursor is where it was.
    Rested,
}

/// What the machine reads and changes: the source, its geometry, its settings, the selection, and where its outcomes go.
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
#[derive(Debug)]
pub struct KeyMachine {
    handle: KeyHandle,
    type_ahead: TypeAhead,
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

    /// A primary press on the collection: it ends a 10xer range, as a click does in Strata, at once, so the footer is right.
    pub fn pressed(&mut self) {
        self.handle.set_range(None);
    }

    /// Drops what is in progress: the type-ahead prefix, a 10xer range, and commands still queued on the handle.
    pub fn reset(&mut self) {
        self.type_ahead.reset();
        self.handle.set_range(None);
        while self.handle.next().is_some() {}
        self.last_edge = None;
    }

    /// Runs the commands the host queued on the handle (`KeyHandle::run`).
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
        let taken = self.route(command, edge_before, s);
        self.range_revision = s.selection().revision();
        taken
    }

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

    fn select(
        &mut self,
        target: Target,
        edge_before: Option<(Direction, Option<EntryId>)>,
        s: &mut dyn Surface,
    ) {
        let reselects = s.settings().reselects_in_place;
        let cursor = s.selection().cursor_index(s.source());
        match self.land(target, s) {
            Landing::At(i) => {
                op(s, |sel, src| sel.select_only(src, i));
                s.emit(Outcome::Reveal(i));
                if reselects && cursor == Some(i) {
                    s.emit(Outcome::Rested);
                }
            }
            Landing::Edge(d) => {
                let vertical = matches!(d, Direction::Up | Direction::Down);
                if let Some(i) = cursor.filter(|_| reselects && vertical) {
                    op(s, |sel, src| sel.select_only(src, i));
                    s.emit(Outcome::Rested);
                }
                self.edge(d, edge_before, s);
            }
            Landing::Nowhere => {}
        }
    }

    fn dispatch_single(
        &mut self,
        command: Command,
        edge_before: Option<(Direction, Option<EntryId>)>,
        s: &mut dyn Surface,
    ) -> Option<bool> {
        match command {
            Command::Extend(target) => {
                self.select(target, edge_before, s);
                Some(true)
            }
            Command::Toggle
            | Command::ToggleAdvance
            | Command::SelectAll
            | Command::Invert
            | Command::Range(_) => Some(false),
            _ => None,
        }
    }

    fn route(
        &mut self,
        command: Command,
        edge_before: Option<(Direction, Option<EntryId>)>,
        s: &mut dyn Surface,
    ) -> bool {
        let single = if s.settings().multiple {
            None
        } else {
            self.dispatch_single(command, edge_before, s)
        };
        single.unwrap_or_else(|| self.dispatch(command, edge_before, s))
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
                self.select(target, edge_before, s);
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
                    select_all(s, len - 1);
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

    fn extend(&mut self, target: Target, s: &mut dyn Surface) -> bool {
        let len = s.source().len();
        if len == 0 {
            return !matches!(target, Target::Step(Direction::Left | Direction::Right));
        }
        let cursor = s.selection().cursor_index(s.source());
        let (origin, alone) = self.extend_origin(target, cursor, s);
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
        let stays = alone || (empty && matches!(target, Target::Step(_)));
        let to = if stays {
            origin
        } else {
            self.land_or(target, origin, s)
        };
        op(s, |sel, src| sel.extend_to(src, to, Extend::Replace));
        s.emit(Outcome::Reveal(to));
        true
    }

    fn extend_origin(
        &self,
        target: Target,
        cursor: Option<usize>,
        s: &dyn Surface,
    ) -> (usize, bool) {
        if let Some(i) = cursor {
            return (i, false);
        }
        if !matches!(target, Target::Step(_))
            && let Landing::At(i) = self.land(target, s)
        {
            return (i, true);
        }
        let last = s.source().len().saturating_sub(1);
        (if forward(target) { 0 } else { last }, false)
    }

    /// Where `target` lands from the cursor, or `origin` where it lands nowhere.
    fn land_or(&self, target: Target, origin: usize, s: &dyn Surface) -> usize {
        match self.land(target, s) {
            Landing::At(i) => i,
            Landing::Edge(_) | Landing::Nowhere => origin,
        }
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
            Target::Step(d) => match cursor {
                Some(i) => self.step(i, d, s),
                None => first_step(d, last, s.settings().left_leaves_without_cursor),
            },
        }
    }

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

fn first_step(d: Direction, last: usize, left_leaves: bool) -> Landing {
    match d {
        Direction::Left if left_leaves => Landing::Edge(d),
        Direction::Down | Direction::Right => Landing::At(0),
        Direction::Up | Direction::Left => Landing::At(last),
    }
}

fn select_all(s: &mut dyn Surface, last: usize) {
    if !s.settings().select_all_to_last {
        if op(s, |sel, src| sel.select_all(src)).cursor {
            s.emit(Outcome::Reveal(0));
        }
        return;
    }
    let anchor = match s.selection().anchor() {
        Some(_) => AnchorRule::Keep,
        None => AnchorRule::Move,
    };
    op(s, |sel, src| sel.select_all(src));
    op(s, |sel, src| sel.move_cursor(src, last, anchor));
}

/// Runs one selection operation on the surface.
fn op(
    s: &mut dyn Surface,
    mut f: impl FnMut(&mut Selection, &dyn EntrySource) -> Changed,
) -> Changed {
    s.apply(&mut f)
}

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
