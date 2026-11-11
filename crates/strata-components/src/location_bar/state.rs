//! [`LocationBarState`]: the bar's state, held by the host and reached by accessor.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use rune::{TextEdit, Tx};

use super::completion::{Command, Completion, Out};
use super::data::{Candidate, Completions};
use super::intent::{LocationIntent, Via};
use crate::contract::dnd::DropHover;
use crate::scroll::{Bar, ScrollFeed};

/// How long the Copy path button says "Path copied" (Strata's 2 s).
pub const COPIED_FOR: f64 = 2.0;

/// How long a file drag rests on an ancestor crumb before it opens (Strata's 750 ms):
/// [`super::LocationBar::spring_delay`]'s default.
pub const SPRING_DELAY: f64 = 0.75;

/// What a call into the bar changed, and the intents it raised for the host to route as it
/// routes the bar's own. Every host-facing call that can raise one returns this.
#[must_use = "route the intents as the bar's own"]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polled {
    /// The bar's look changed: the host's poll returns true.
    pub changed: bool,
    pub intents: Vec<LocationIntent>,
}

impl Polled {
    fn changed() -> Self {
        Self {
            changed: true,
            intents: Vec::new(),
        }
    }

    fn push(&mut self, intent: LocationIntent) {
        self.changed = true;
        self.intents.push(intent);
    }

    fn merge(&mut self, other: Polled) {
        self.changed |= other.changed;
        self.intents.extend(other.intents);
    }
}

/// A drag resting on an ancestor crumb, waiting to spring it open.
#[derive(Clone, Debug, PartialEq)]
struct Spring {
    index: usize,
    id: String,
    delay: f64,
    /// When it arrived, on `Tx::clock`.
    since: f64,
}

/// The location bar's state: crumbs or the entry, the entry's text, the suggestions, the
/// hierarchy menu, the drag over the crumbs and the copy feedback. The host keeps one per bar
/// (per tab) and hands the bar an accessor to it.
///
/// The host calls, from its poll, [`LocationBarState::tick`] every frame (the bar's timers and
/// a press that took focus away from the entry), and [`LocationBarState::deliver`] with each
/// answer to a [`LocationIntent::Complete`]. It forwards its window's activation with
/// [`LocationBarState::set_window_focused`] (Rune draft U-C6-6).
#[derive(Debug)]
pub struct LocationBarState {
    editing: bool,
    /// The entry's text, caret and focus. The entry binds it; a host reads it, and changes it
    /// only through the methods here.
    pub edit: TextEdit,
    completion: Completion,
    window_focused: bool,
    /// The entry has held focus since the edit began: only then does losing it end the edit.
    seen_focus: bool,
    menu: Option<(f32, f32)>,
    /// The ancestor crumb under the pointer.
    hovered: Option<usize>,
    drop: Option<(usize, DropHover)>,
    spring: Option<Spring>,
    /// When "Path copied" showed, on `Tx::clock`.
    copied: Option<f64>,
    reveal: bool,
    /// Navigations so far: the crumb row's key, so each new trail mounts afresh.
    trail_generation: u64,
    pub(crate) feed: ScrollFeed,
    pub(crate) bar: Bar,
    pub(crate) handler: HandlerSlot,
}

/// The bar's intent handler as the latest view built it. Suggestion rows are memoised, so a
/// row kept since its list came would otherwise call the handler from the view that first
/// built it; they read it from here when pressed instead. Type-erased, as the state isn't
/// generic over the app's.
#[derive(Clone, Default)]
pub(crate) struct HandlerSlot(Rc<RefCell<Option<Rc<dyn Any>>>>);

impl HandlerSlot {
    pub fn set<T: 'static>(&self, handler: T) {
        *self.0.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn get<T: Clone + 'static>(&self) -> Option<T> {
        self.0.borrow().as_ref()?.downcast_ref::<T>().cloned()
    }
}

impl std::fmt::Debug for HandlerSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HandlerSlot")
    }
}

impl Default for LocationBarState {
    fn default() -> Self {
        Self {
            editing: false,
            edit: TextEdit::new(""),
            completion: Completion::default(),
            window_focused: true,
            seen_focus: false,
            menu: None,
            hovered: None,
            drop: None,
            spring: None,
            copied: None,
            reveal: true,
            trail_generation: 0,
            feed: ScrollFeed::new(),
            bar: Bar::new(),
            handler: HandlerSlot::default(),
        }
    }
}

impl LocationBarState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the entry shows instead of the crumbs.
    pub fn editing(&self) -> bool {
        self.editing
    }

    /// Whether the entry has the keyboard: the host's key dispatch leaves the view's shortcuts
    /// alone then (Ctrl+K, Ctrl+1–3), as Strata's does.
    pub fn entry_focused(&self) -> bool {
        self.editing && self.edit.focused
    }

    /// The suggestions the list holds, shown or not.
    pub fn suggestions(&self) -> &[Candidate] {
        self.completion.shown()
    }

    pub(crate) fn suggestions_generation(&self) -> u64 {
        self.completion.shown_generation()
    }

    /// Whether the suggestion list shows.
    pub fn suggestions_open(&self) -> bool {
        self.editing && self.window_focused && self.completion.is_open()
    }

    /// The highlighted suggestion.
    pub fn highlighted(&self) -> Option<usize> {
        self.completion.highlighted()
    }

    /// Highlights suggestion `i` (a scene, or a host restoring a tab).
    pub fn set_highlighted(&mut self, i: Option<usize>) {
        self.completion.set_highlighted(i);
    }

    /// Whether the Copy path button says "Path copied".
    pub fn copied(&self) -> bool {
        self.copied.is_some()
    }

    /// Where the hierarchy menu is open, in window points.
    pub fn menu(&self) -> Option<(f32, f32)> {
        self.menu
    }

    /// Opens the hierarchy menu at window point (`x`, `y`): its top-left corner.
    pub fn open_menu(&mut self, x: f32, y: f32) {
        self.menu = Some((x, y));
    }

    pub fn close_menu(&mut self) {
        self.menu = None;
    }

    /// The ancestor crumb a drag is over, by its index, and how.
    pub fn drop_hover(&self) -> Option<(usize, DropHover)> {
        self.drop
    }

    /// Sets the drag's hover over crumb `index` (a scene, or a binder of the host's own).
    pub fn set_drop_hover(&mut self, index: usize, hover: DropHover) {
        self.drop = match hover {
            DropHover::Left if self.drop.is_some_and(|(i, _)| i != index) => self.drop,
            DropHover::Left => None,
            _ => Some((index, hover)),
        };
    }

    /// Opens the entry on `text` (the folder the bar names), all of it selected, with the
    /// keyboard: Ctrl+L, or a press on the crumbs' empty space. Asks for suggestions.
    pub fn begin_edit(&mut self, text: &str) -> Polled {
        let mut out = Polled::changed();
        self.menu = None;
        if self.editing {
            // Ctrl+L again keeps what was typed, selects it, and lists it afresh, as Strata's
            // `begin_location_edit` does.
            self.edit.set_selection(0, self.edit.text.len());
            self.edit.break_history_group();
            self.edit.focused = true;
            out.merge(self.edited());
            return out;
        }
        if self.completion.clear() {
            out.push(LocationIntent::CancelCompletion);
        }
        self.editing = true;
        self.seen_focus = false;
        self.edit = TextEdit::new(text);
        self.edit.set_selection(0, text.len());
        self.edit.focused = true;
        out.merge(self.edited());
        out
    }

    /// Back to the crumbs without a [`LocationIntent::Cancelled`]: the host's own call, after a
    /// location that can't be opened, say, or when it hides the bar.
    pub fn end_edit(&mut self) -> Polled {
        let mut out = Polled::default();
        if self.editing {
            out.changed = true;
            if self.close() {
                out.push(LocationIntent::CancelCompletion);
            }
        }
        out
    }

    /// Keeps the entry open on `text` with the keyboard, the caret at its end: a submitted
    /// location that mounts first, which Strata shows without its password.
    pub fn resume_edit(&mut self, text: &str) {
        self.editing = true;
        self.seen_focus = false;
        self.edit = TextEdit::new(text);
        self.edit.set_caret(text.len());
        self.edit.focused = true;
    }

    /// The host opened another folder. An entry the user is typing in stays open with its text
    /// (a new tab's first load); otherwise the crumbs show, scrolled to the current one.
    pub fn navigated(&mut self) -> Polled {
        self.reveal = true;
        self.trail_generation = self.trail_generation.wrapping_add(1);
        self.spring = None;
        self.drop = None;
        self.hovered = None;
        self.menu = None;
        if self.entry_focused() {
            return Polled::changed();
        }
        let mut out = self.end_edit();
        out.changed = true;
        out
    }

    /// An answer to a [`LocationIntent::Complete`]. A stale one (an older generation, text
    /// that changed since, the crumbs showing, or the window unfocused) is dropped.
    pub fn deliver(&mut self, answer: Completions) -> Polled {
        if !self.editing || !self.window_focused {
            return Polled::default();
        }
        let text = self.edit.text.clone();
        match self
            .completion
            .arrive(answer.generation, answer.candidates, &text)
        {
            Some(out) => {
                let mut polled = Polled::changed();
                polled.merge(self.apply(out));
                polled
            }
            None => Polled::default(),
        }
    }

    /// The window was activated or deactivated (the root's `on_window_focus`). Losing it closes
    /// the suggestions and drops a listing in flight; the entry and its text stay. Regaining it
    /// shows nothing until the text changes again.
    pub fn set_window_focused(&mut self, focused: bool) -> Polled {
        if self.window_focused == focused {
            return Polled::default();
        }
        self.window_focused = focused;
        let mut out = Polled::changed();
        if !focused && self.completion.dismiss() {
            out.push(LocationIntent::CancelCompletion);
        }
        out
    }

    /// Closes the suggestions and drops a listing in flight: the host hid the entry (a tab
    /// switch) without ending the edit.
    pub fn dismiss_completion(&mut self) -> Polled {
        let mut out = Polled::changed();
        if self.completion.dismiss() {
            out.push(LocationIntent::CancelCompletion);
        }
        out
    }

    /// One frame at `now` seconds on the window's clock (the poll's, as `Tx::clock`): ends an edit
    /// whose entry lost the keyboard to a press elsewhere (U-C6-3), springs a crumb a drag rested
    /// on, and ends the copy feedback.
    pub fn tick(&mut self, now: f64, tx: &mut Tx) -> Polled {
        let mut out = Polled::default();
        if self.editing {
            if self.edit.focused {
                self.seen_focus = true;
            } else if self.seen_focus {
                // A press outside the entry took the keyboard: cancel, and let the press do
                // what it does.
                out.changed = true;
                if self.close() {
                    out.push(LocationIntent::CancelCompletion);
                }
                out.push(LocationIntent::Cancelled);
            }
        }
        if let Some(spring) = self.spring.as_mut() {
            let left = spring.delay - (now - spring.since);
            if left <= 1e-6 {
                let id = spring.id.clone();
                self.spring = None;
                out.push(LocationIntent::Open {
                    id,
                    via: Via::Spring,
                });
            } else {
                tx.wake_at(spring.since + spring.delay);
            }
        }
        match self.copied {
            None => {}
            Some(t) if now - t >= COPIED_FOR - 1e-6 => {
                self.copied = None;
                out.changed = true;
            }
            Some(t) => tx.wake_at(t + COPIED_FOR),
        }
        out
    }

    // The bar's own handlers.

    /// The user changed the entry's text.
    pub(crate) fn edited(&mut self) -> Polled {
        let mut out = Polled::changed();
        if self.window_focused {
            let query = self.completion.request(&self.edit.text.clone());
            out.push(LocationIntent::Complete(query));
        } else if self.completion.clear() {
            out.push(LocationIntent::CancelCompletion);
        }
        out
    }

    /// A completion key. The second value is whether the key goes on.
    pub(crate) fn command(&mut self, command: Command) -> (Polled, bool) {
        let text = self.edit.text.clone();
        let (out, proceeds) = self.completion.press(command, true, &text);
        (self.apply(out), proceeds)
    }

    /// Escape, Cancel, or the pointer elsewhere: back to the crumbs, telling the host.
    pub(crate) fn cancel(&mut self) -> Polled {
        let mut out = self.end_edit();
        out.push(LocationIntent::Cancelled);
        out
    }

    /// Enter or Navigate with `text`: back to the crumbs, then the host opens it.
    pub(crate) fn submit(&mut self, text: String) -> Polled {
        let mut out = self.end_edit();
        out.push(LocationIntent::Submit { text });
        out
    }

    /// A suggestion chosen with the pointer.
    /// Taken at once, even while keys wait for a listing (Strata's `row_activated`): the keys and
    /// the listing are dropped.
    pub(crate) fn choose(&mut self, i: usize) -> Polled {
        match self.completion.choose(i) {
            Some(out) => self.apply(out),
            None => Polled::default(),
        }
    }

    /// Shows "Path copied" for [`COPIED_FOR`] from `now` (`Tx::clock`): the Copy path button
    /// does this itself; a scene, or a host restoring a tab, may too. The host's next tick asks
    /// for the wake that ends it.
    pub fn show_copied(&mut self, now: f64) {
        self.copied = Some(now);
    }

    /// A drag the crumb `index` (`id`) accepts arrived or left at `now` (`Tx::clock`). `delay`
    /// is the spring delay.
    pub(crate) fn drag_over(
        &mut self,
        index: usize,
        id: &str,
        hover: DropHover,
        delay: Option<f64>,
        now: f64,
    ) {
        self.set_drop_hover(index, hover);
        match (hover, delay) {
            // A binder may report the same hover again (X2's on each motion): the spring keeps
            // its start, as Strata's `schedule_spring_load_navigation` does.
            (DropHover::Accepting, Some(_))
                if self
                    .spring
                    .as_ref()
                    .is_some_and(|s| s.index == index && s.id == id) => {}
            (DropHover::Accepting, Some(delay)) => {
                self.spring = Some(Spring {
                    index,
                    id: id.to_owned(),
                    delay,
                    since: now,
                });
            }
            _ if self.spring.as_ref().is_some_and(|s| s.index == index) => self.spring = None,
            _ => {}
        }
    }

    /// The drag dropped or was refused: nothing springs, nothing is hovered.
    pub(crate) fn drag_ended(&mut self) {
        self.spring = None;
        self.drop = None;
    }

    pub(crate) fn hovered_crumb(&self) -> Option<usize> {
        self.hovered
    }

    pub(crate) fn hover_crumb(&mut self, index: usize, over: bool) {
        if over {
            self.hovered = Some(index);
        } else if self.hovered == Some(index) {
            self.hovered = None;
        }
    }

    pub(crate) fn trail_generation(&self) -> u64 {
        self.trail_generation
    }

    /// Whether the crumbs should scroll to their end the next time they lay out, and forgets it.
    pub(crate) fn take_reveal(&mut self) -> bool {
        std::mem::take(&mut self.reveal)
    }

    fn apply(&mut self, out: Out) -> Polled {
        let mut polled = Polled::default();
        if let Some(text) = out.text {
            self.edit.replace_text(text.clone());
            self.edit.set_caret(text.len());
            polled.changed = true;
        }
        if out.cancelled {
            polled.push(LocationIntent::CancelCompletion);
        }
        for q in out.queries {
            polled.push(LocationIntent::Complete(q));
        }
        if let Some(text) = out.submit {
            // The listing was already dropped above, if any was in flight.
            self.close();
            polled.push(LocationIntent::Submit { text });
        }
        polled.changed |= self.completion.is_pending() || self.completion.is_open();
        polled
    }

    /// Back to the crumbs. True when a listing in flight was dropped.
    fn close(&mut self) -> bool {
        self.editing = false;
        self.seen_focus = false;
        self.edit.focused = false;
        self.edit.cancel_composition();
        self.reveal = true;
        self.menu = None;
        self.completion.clear()
    }
}
