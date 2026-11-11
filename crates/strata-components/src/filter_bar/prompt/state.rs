//! [`PromptState`]: the open footer prompt, held by the host and reached by accessor.

use std::ops::Range;

use rune::{Key, KeyEvent, TextEdit};
use strata_kit::icons::IconName;

use super::spec::{Arrows, PromptSpec};
use crate::contract::Point;
use crate::filter_bar::field::FocusWatch;

/// One candidate under a prompt: a folder from the picker or the history.
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// The host's id for it (its path): [`PromptIntent::Submit`] names the chosen one by it,
    /// and a choice the user made survives later batches by it.
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

/// A listing the prompt asked for ([`PromptIntent::Complete`]).
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionQuery {
    /// Answer with this generation; an answer for an older one is dropped.
    pub generation: u64,
    /// The prompt's text when it asked.
    pub text: String,
}

/// An answer to a [`CompletionQuery`]: one batch. A search that streams sends several with
/// the same generation, the last one `done`.
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidates {
    /// The query's generation.
    pub generation: u64,
    /// Every candidate so far, best first (each batch replaces the last).
    pub items: Vec<Candidate>,
    /// The search finished: a submit waiting for it ([`PromptState::wait_until_settled`])
    /// goes now.
    pub done: bool,
}

/// What closed a prompt.
///
/// Used by: hosts.
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

/// What a prompt asks of its host. Finding, filtering, searching, navigating, renaming and
/// validating are the host's.
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PromptIntent<K> {
    /// The user edited the text: not when a prompt opens, closes, or the prompt writes it
    /// (Tab's completion does tell, as Strata's does). `serial` names this text: a check
    /// started for it answers with it.
    Edited {
        /// The prompt.
        kind: K,
        /// Its text now.
        text: String,
        /// The text's serial.
        serial: u64,
    },
    /// List candidates for this text (a prompt with [`Arrows::Candidates`], on opening and
    /// on each edit): answer with [`PromptState::deliver`]. The previous list stays until then.
    Complete {
        /// The prompt.
        kind: K,
        /// The listing asked for.
        query: CompletionQuery,
    },
    /// The listing asked for last is no longer wanted: the prompt closed or was replaced.
    CancelCompletion,
    /// Up (`-1`) or Down (`1`) in a prompt with [`Arrows::Listing`]: move the listing's cursor
    /// while the prompt keeps the keyboard.
    Step {
        /// The prompt.
        kind: K,
        /// The step.
        delta: i32,
    },
    /// Enter (Shift+Enter too). The prompt stays open: the host closes it
    /// ([`PromptState::close`]) once it acts, or keeps it open with a reason
    /// ([`PromptState::refuse`]). An Enter an input method takes never arrives here.
    Submit {
        /// The prompt.
        kind: K,
        /// Its text.
        text: String,
        /// The chosen candidate's id, if candidates show.
        chosen: Option<String>,
        /// Whether the user chose it (Up, Down), rather than the ranking's first.
        stepped: bool,
        /// Whether the last listing asked for has finished (always, for prompts without
        /// candidates). A host that must not act on a partial ranking calls
        /// [`PromptState::wait_until_settled`]; the prompt submits again once it has.
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
    /// A right-click, the Menu key or Shift+F10 on the field: the host's field menu (C21's
    /// `FieldMenu::open`), at `at` in window points.
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

/// The footer prompt: which one is open (`K`, the host's kinds), its text, hint, candidates
/// and the serial that names its text. The host keeps one per window.
///
/// The host calls [`PromptState::open`] from its keys (`/`, `f`, `g Space`, `r`),
/// [`PromptState::tick`] each frame (a field that lost the keyboard closes the prompt), and
/// [`PromptState::close`] once it acted on a [`PromptIntent::Submit`]. Leaving 10xer mode or
/// closing the window closes it too.
#[derive(Debug)]
pub struct PromptState<K> {
    /// The field's text, caret and focus. The prompt binds it; C21's `FieldMenu` edits it.
    pub edit: TextEdit,
    open: Option<Open<K>>,
    serial: u64,
    generation: u64,
    watch: FocusWatch,
    /// The host keeps the prompt open while the field lacks the keyboard (the shortcut
    /// reference has it).
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

    /// Whether the prompt's field has the keyboard: while it does, only its own keys act, and
    /// every other key edits the text.
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

    /// Opens prompt `kind` on `text` with the keyboard in it. Any open prompt closes first
    /// ([`By::Replaced`]), and none of its text carries over. `select` selects a byte range
    /// (`rename ›`'s stem); without it, the caret goes after the text, unselected, so typing
    /// extends a pre-filled query. A prompt with candidates asks for them.
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

    /// Closes the prompt and clears its text (no undo history stays): the host acted on a
    /// submit, or left 10xer mode. The keyboard goes back to the listing (`focus_return`).
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

    /// Shows `hint` beside the field (and as its description), while the text is still the
    /// one `serial` names. False, changing nothing, for an older serial.
    pub fn show_hint(&mut self, serial: u64, hint: Option<String>) -> bool {
        match &mut self.open {
            Some(o) if serial == self.serial => {
                o.hint = hint;
                true
            }
            _ => false,
        }
    }

    /// Keeps the prompt open on a refused text, with `reason` as its hint and the field marked
    /// invalid: a taken name, a name the location can't store. A check that answers after the
    /// text changed, or after the prompt closed, names an older serial and changes nothing
    /// (false): it must create or rename nothing either.
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

    /// An answer to a [`PromptIntent::Complete`]. One for an older generation (every open,
    /// close and edit starts a new one), with no prompt open, or for a prompt that lists no
    /// candidates, is dropped. A choice the user made stays on its candidate while the
    /// candidate is still listed; otherwise the first is chosen. A `done` answer releases a
    /// submit waiting for it.
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

    /// Holds the submit for text `serial` until the listing finishes, then sends it again with
    /// `settled` (Strata's `when_settled`: an early Enter must not act on a partial ranking).
    /// An edit drops it. False when nothing is pending: act now.
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

    /// Keeps the prompt open while its field lacks the keyboard, or lets it close again: the
    /// host holds it while the shortcut reference (F1, C19) has the keyboard, as Strata's
    /// `abandoned` does. Released with the field still unfocused, the next tick closes it.
    pub fn hold_open(&mut self, on: bool) {
        self.held = on;
    }

    /// One frame: a prompt whose field lost the keyboard (a clicked row, another pane) closes
    /// ([`By::Blur`]), so its text never reaches a browsing command, unless the host holds it
    /// open ([`PromptState::hold_open`]). Rune has no focus-change hook (Rune draft U-C6-3),
    /// so this notices a frame later.
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

    /// The user changed the text (Tab's write included): a new serial, the hint and refusal
    /// go, a waiting submit is dropped, the user's choice is forgotten (Strata's
    /// `forget_candidate_step`), and a prompt with candidates asks for a new listing.
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

    /// The text changed outside the field's own editing: C21's `FieldMenu` cut, pasted or
    /// deleted (`FieldMenu::on_change`). The same as typing.
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
            // Shift+Tab moves focus back, as GTK's does.
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

    /// A press on candidate `i`: it is chosen, and submitted at once, as Strata's
    /// `row_activated` opens it.
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
