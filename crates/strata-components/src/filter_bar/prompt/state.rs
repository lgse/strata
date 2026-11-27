// SPDX-License-Identifier: MIT

//! [`PromptState`]: the open footer prompt, held by the host and reached by accessor.

use std::ops::Range;

use rune::{Key, KeyEvent, TextEdit};
use strata_kit::icons::IconName;

use super::spec::{Arrows, PromptSpec};
use crate::contract::Point;
use crate::filter_bar::field::FocusWatch;

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub id: String,
    /// What the row shows: the folder's name.
    pub label: String,
    /// Byte ranges of `label` the query matched, drawn in the accent.
    pub matched: Vec<Range<usize>>,
    /// The parent folder, dim at the row's end: `~/dev`.
    pub hint: String,
    /// What Tab writes into the prompt for it: `./…/`, `~/…/` or an absolute path.
    pub insert: String,
    /// The row's icon.
    pub icon: IconName,
}

impl Candidate {
    /// A folder candidate whose Tab text is `insert`.
    pub fn folder(
        id: impl Into<String>,
        label: impl Into<String>,
        hint: impl Into<String>,
        insert: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            matched: Vec::new(),
            hint: hint.into(),
            insert: insert.into(),
            icon: IconName::Folder,
        }
    }

    /// The same candidate with `ranges` of its label matched.
    pub fn matched(mut self, ranges: impl IntoIterator<Item = Range<usize>>) -> Self {
        self.matched = ranges.into_iter().collect();
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionQuery {
    /// Answer with this generation; an answer for an older one is dropped.
    pub generation: u64,
    /// The prompt's text when it asked.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candidates {
    /// The query's generation.
    pub generation: u64,
    /// Every candidate so far, best first (each batch replaces the last).
    pub items: Vec<Candidate>,
    pub done: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum By {
    /// Escape (Shift+Escape too).
    Escape,
    /// The field lost the keyboard: a clicked row, another pane, the sidebar.
    Blur,
    /// Another prompt opened over it.
    Replaced,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PromptIntent<K> {
    Edited {
        /// The prompt.
        kind: K,
        /// Its text now.
        text: String,
        /// The text's serial.
        serial: u64,
    },
    Complete {
        /// The prompt.
        kind: K,
        /// The listing asked for.
        query: CompletionQuery,
    },
    /// The listing asked for last is no longer wanted: the prompt closed or was replaced.
    CancelCompletion,
    Step {
        /// The prompt.
        kind: K,
        /// The step.
        delta: i32,
    },
    Submit {
        /// The prompt.
        kind: K,
        /// Its text.
        text: String,
        /// The chosen candidate's id, if candidates show.
        chosen: Option<String>,
        /// Whether the user chose it (Up, Down), rather than the ranking's first.
        stepped: bool,
        settled: bool,
        /// The text's serial, for [`PromptState::refuse`] and [`PromptState::show_hint`].
        serial: u64,
    },
    /// The prompt closed itself; its text, when the host needs it (`s` keeps non-empty hits).
    Dismissed {
        /// The prompt.
        kind: K,
        /// Its text when it closed.
        text: String,
        /// What closed it.
        by: By,
    },
    FieldMenu {
        /// Where the menu opens.
        at: Point,
    },
}

/// What a call into the prompt changed, and its intents for the host.
#[must_use = "route the intents as the prompt's own"]
#[derive(Clone, Debug, PartialEq)]
pub struct Polled<K> {
    /// The prompt's look changed: the host's poll returns true.
    pub changed: bool,
    /// The intents, in order.
    pub intents: Vec<PromptIntent<K>>,
}

impl<K> Default for Polled<K> {
    fn default() -> Self {
        Self {
            changed: false,
            intents: Vec::new(),
        }
    }
}

impl<K> Polled<K> {
    fn changed() -> Self {
        Self {
            changed: true,
            intents: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, intent: PromptIntent<K>) {
        self.changed = true;
        self.intents.push(intent);
    }

    fn merge(&mut self, other: Polled<K>) {
        self.changed |= other.changed;
        self.intents.extend(other.intents);
    }
}

#[derive(Clone, Debug)]
struct Open<K> {
    kind: K,
    spec: PromptSpec,
    hint: Option<String>,
    invalid: bool,
    items: Vec<Candidate>,
    chosen: usize,
    /// The user chose with Up or Down: the choice survives later batches.
    stepped: bool,
    /// The last listing asked for hasn't finished.
    pending: bool,
    /// A submit waits for it.
    deferred: bool,
}

#[derive(Debug)]
pub struct PromptState<K> {
    pub edit: TextEdit,
    open: Option<Open<K>>,
    serial: u64,
    generation: u64,
    watch: FocusWatch,
    held: bool,
}

impl<K> Default for PromptState<K> {
    fn default() -> Self {
        Self {
            edit: TextEdit::new(""),
            open: None,
            serial: 0,
            generation: 0,
            watch: FocusWatch::default(),
            held: false,
        }
    }
}

impl<K: Copy + PartialEq + std::fmt::Debug + 'static> PromptState<K> {
    /// No prompt open.
    pub fn new() -> Self {
        Self::default()
    }

    /// The open prompt's kind.
    pub fn kind(&self) -> Option<K> {
        self.open.as_ref().map(|o| o.kind)
    }

    /// Whether a prompt is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn focused(&self) -> bool {
        self.is_open() && self.edit.focused
    }

    /// The open prompt's spec.
    pub fn spec(&self) -> Option<&PromptSpec> {
        self.open.as_ref().map(|o| &o.spec)
    }

    /// The text.
    pub fn text(&self) -> &str {
        &self.edit.text
    }

    /// The serial naming the text now: it changes on every open, edit and close.
    pub fn serial(&self) -> u64 {
        self.serial
    }

    /// The host's hint (a reason, "Searching…", "No matching folders").
    pub fn hint(&self) -> Option<&str> {
        self.open.as_ref()?.hint.as_deref()
    }

    /// Whether the host refused the text ([`PromptState::refuse`]).
    pub fn invalid(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.invalid)
    }

    /// The candidates showing.
    pub fn candidates(&self) -> &[Candidate] {
        self.open.as_ref().map_or(&[], |o| &o.items)
    }

    /// The chosen candidate's index.
    pub fn chosen_index(&self) -> Option<usize> {
        let o = self.open.as_ref()?;
        (o.chosen < o.items.len()).then_some(o.chosen)
    }

    /// The chosen candidate.
    pub fn chosen(&self) -> Option<&Candidate> {
        let o = self.open.as_ref()?;
        o.items.get(o.chosen)
    }

    /// Whether the user chose the candidate (Up, Down), rather than the ranking.
    pub fn stepped(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.stepped)
    }

    /// Whether a listing asked for hasn't finished.
    pub fn pending(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.pending)
    }

    pub fn open(
        &mut self,
        kind: K,
        spec: PromptSpec,
        text: &str,
        select: Option<Range<usize>>,
    ) -> Polled<K> {
        let mut out = Polled::changed();
        if let Some(old) = self.open.take() {
            if old.pending {
                out.push(PromptIntent::CancelCompletion);
            }
            out.push(PromptIntent::Dismissed {
                kind: old.kind,
                text: std::mem::take(&mut self.edit.text),
                by: By::Replaced,
            });
        }
        let candidates = spec.arrows == Arrows::Candidates;
        self.open = Some(Open {
            kind,
            spec,
            hint: None,
            invalid: false,
            items: Vec::new(),
            chosen: 0,
            stepped: false,
            pending: false,
            deferred: false,
        });
        self.serial += 1;
        // A listing asked for by an earlier prompt never lands in this one.
        self.generation += 1;
        self.edit = TextEdit::new(text);
        match select {
            Some(r) if r.start <= r.end && r.end <= text.len() => {
                self.edit.set_selection(r.start, r.end)
            }
            _ => self.edit.set_caret(text.len()),
        }
        self.edit.focused = true;
        self.watch.reset();
        if candidates {
            out.merge(self.request());
        }
        out
    }

    pub fn close(&mut self) -> Polled<K> {
        let Some(old) = self.open.take() else {
            return Polled::default();
        };
        let mut out = Polled::changed();
        if old.pending {
            out.push(PromptIntent::CancelCompletion);
        }
        self.serial += 1;
        self.generation += 1;
        self.edit = TextEdit::new("");
        self.watch.reset();
        // A hold is for this prompt alone (its field menu): the next one closes on a blur.
        self.held = false;
        out
    }

    pub fn show_hint(&mut self, serial: u64, hint: Option<String>) -> bool {
        match &mut self.open {
            Some(o) if serial == self.serial => {
                o.hint = hint;
                true
            }
            _ => false,
        }
    }

    pub fn refuse(&mut self, serial: u64, reason: impl Into<String>) -> bool {
        match &mut self.open {
            Some(o) if serial == self.serial => {
                o.hint = Some(reason.into());
                o.invalid = true;
                true
            }
            _ => false,
        }
    }

    pub fn deliver(&mut self, answer: Candidates) -> Polled<K> {
        let generation = self.generation;
        let serial = self.serial;
        let text = self.edit.text.clone();
        let Some(o) = self.open.as_mut() else {
            return Polled::default();
        };
        if answer.generation != generation || o.spec.arrows != Arrows::Candidates {
            return Polled::default();
        }
        let kept = o
            .stepped
            .then(|| o.items.get(o.chosen).map(|c| c.id.clone()))
            .flatten()
            .and_then(|id| answer.items.iter().position(|c| c.id == id));
        o.stepped = kept.is_some();
        o.chosen = kept.unwrap_or(0);
        o.items = answer.items;
        let mut out = Polled::changed();
        if answer.done {
            o.pending = false;
            if std::mem::take(&mut o.deferred) {
                out.push(PromptIntent::Submit {
                    kind: o.kind,
                    text,
                    chosen: o.items.get(o.chosen).map(|c| c.id.clone()),
                    stepped: o.stepped,
                    settled: true,
                    serial,
                });
            }
        }
        out
    }

    pub fn wait_until_settled(&mut self, serial: u64) -> bool {
        match &mut self.open {
            Some(o) if serial == self.serial && o.pending => {
                o.deferred = true;
                true
            }
            _ => false,
        }
    }

    /// Whether the host holds the prompt open ([`PromptState::hold_open`]).
    pub fn is_held(&self) -> bool {
        self.held
    }

    pub fn hold_open(&mut self, on: bool) {
        self.held = on;
    }

    pub fn tick(&mut self) -> Polled<K> {
        if self.open.is_none() || self.held || !self.watch.lost(self.edit.focused) {
            return Polled::default();
        }
        self.dismiss(By::Blur)
    }

    // The prompt's own handlers.

    fn dismiss(&mut self, by: By) -> Polled<K> {
        let kind = self.kind();
        let text = self.edit.text.clone();
        let mut out = self.close();
        if let Some(kind) = kind {
            out.push(PromptIntent::Dismissed { kind, text, by });
        }
        out
    }

    /// Asks for a new listing of the text.
    fn request(&mut self) -> Polled<K> {
        let mut out = Polled::default();
        let Some(o) = self.open.as_mut() else {
            return out;
        };
        self.generation += 1;
        o.pending = true;
        out.push(PromptIntent::Complete {
            kind: o.kind,
            query: CompletionQuery {
                generation: self.generation,
                text: self.edit.text.clone(),
            },
        });
        out
    }

    pub(crate) fn edited(&mut self) -> Polled<K> {
        let serial = self.serial + 1;
        let Some(o) = self.open.as_mut() else {
            return Polled::default();
        };
        self.serial = serial;
        o.hint = None;
        o.invalid = false;
        o.deferred = false;
        o.stepped = false;
        let (kind, candidates) = (o.kind, o.spec.arrows == Arrows::Candidates);
        let mut out = Polled::changed();
        out.push(PromptIntent::Edited {
            kind,
            text: self.edit.text.clone(),
            serial,
        });
        if candidates {
            out.merge(self.request());
        }
        out
    }

    pub fn text_edited(&mut self) -> Polled<K> {
        self.edited()
    }

    /// A key in the field, before the editor sees it. `Some` takes it.
    pub(crate) fn key(&mut self, ev: &KeyEvent, repeat: bool) -> Option<Polled<K>> {
        let o = self.open.as_ref()?;
        // Rune gives Enter and Escape to a composition first; the rest stay the editor's.
        if self.edit.is_composing() {
            return None;
        }
        let m = ev.mods;
        let bare = !(m.ctrl || m.alt || m.cmd || m.logo);
        match &ev.key {
            Key::Escape if bare => Some(if repeat {
                Polled::default()
            } else {
                self.dismiss(By::Escape)
            }),
            Key::Enter if bare => {
                if repeat {
                    return Some(Polled::default());
                }
                let mut out = Polled::default();
                out.push(PromptIntent::Submit {
                    kind: o.kind,
                    text: self.edit.text.clone(),
                    chosen: o.items.get(o.chosen).map(|c| c.id.clone()),
                    stepped: o.stepped,
                    settled: !o.pending,
                    serial: self.serial,
                });
                Some(out)
            }
            Key::Up | Key::Down if bare => {
                let delta = if ev.key == Key::Up { -1 } else { 1 };
                Some(self.step(delta))
            }
            Key::Tab
                if bare
                    && !m.shift
                    && o.spec.legend.as_ref().is_some_and(|l| l.complete.is_some()) =>
            {
                let Some(insert) = o.items.get(o.chosen).map(|c| c.insert.clone()) else {
                    return Some(Polled::default());
                };
                self.edit.replace_text(insert.clone());
                self.edit.set_caret(insert.len());
                Some(self.edited())
            }
            // No undo history: the text can't come back once changed (a typed credential).
            Key::Command('z' | 'y') if !o.spec.undo => Some(Polled::default()),
            _ => None,
        }
    }

    pub(crate) fn press_candidate(&mut self, i: usize) -> Polled<K> {
        let serial = self.serial;
        let text = self.edit.text.clone();
        let Some(o) = self.open.as_mut() else {
            return Polled::default();
        };
        let Some(c) = o.items.get(i) else {
            return Polled::default();
        };
        let id = c.id.clone();
        o.chosen = i;
        o.stepped = true;
        let mut out = Polled::default();
        out.push(PromptIntent::Submit {
            kind: o.kind,
            text,
            chosen: Some(id),
            stepped: true,
            settled: true,
            serial,
        });
        out
    }

    fn step(&mut self, delta: i32) -> Polled<K> {
        let Some(o) = self.open.as_mut() else {
            return Polled::default();
        };
        match o.spec.arrows {
            Arrows::Listing => {
                let mut out = Polled::default();
                out.push(PromptIntent::Step {
                    kind: o.kind,
                    delta,
                });
                out
            }
            Arrows::Candidates if !o.items.is_empty() => {
                let n = o.items.len() as i64;
                o.chosen = (o.chosen as i64 + i64::from(delta)).rem_euclid(n) as usize;
                o.stepped = true;
                Polled::changed()
            }
            Arrows::Candidates | Arrows::Swallow => Polled::default(),
        }
    }
}
