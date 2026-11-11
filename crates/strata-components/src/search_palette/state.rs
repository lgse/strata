//! [`PaletteState`]: the open palette, held by the host and reached by accessor.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rune::{TextEdit, Tx};

use super::data::{Answer, PaletteItem};
use super::intent::{By, PaletteIntent, Via};
use super::mode::PaletteMode;
use super::model::{Follow, Model, Phase};
use crate::contract::Point;
use crate::scroll::{Bar, ScrollFeed};

/// What a call into the palette changed, and the intents it raised for the host to route as
/// it routes the palette's own.
#[must_use = "route the intents as the palette's own"]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polled {
    /// The palette's look changed: the host's poll returns true.
    pub changed: bool,
    pub intents: Vec<PaletteIntent>,
}

impl Polled {
    fn changed() -> Self {
        Self {
            changed: true,
            intents: Vec::new(),
        }
    }

    fn push(&mut self, intent: PaletteIntent) {
        self.changed = true;
        self.intents.push(intent);
    }
}

/// The palette's intent handler as the latest view built it. Rows are memoised, so a row
/// kept since an earlier answer would otherwise call the handler of the view that built it.
/// Type-erased, as the state isn't generic over the app's.
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

/// The palette's state: the mode it is open in, the field's text, the results and the
/// highlight. The host keeps one per window and hands the palette an accessor to it.
///
/// The host calls [`PaletteState::tick`] from its poll every frame, and
/// [`PaletteState::deliver`] with each [`Answer`] to a [`PaletteIntent::Query`].
#[derive(Debug, Default)]
pub struct PaletteState {
    mode: Option<PaletteMode>,
    /// The field's text, caret and focus. The field binds it; a host reads it, and changes it
    /// only through the methods here.
    pub edit: TextEdit,
    model: Model,
    pub(crate) bar: Bar,
    /// The list's scroll position as it last reported it.
    pub(crate) feed: ScrollFeed,
    /// The offset at the last tick, and whether the user is scrolling the list: it moved
    /// between the last two ticks, and not because the palette asked (a fling or a wheel's
    /// motion in flight), which a batch's reveal leaves alone (Strata's
    /// `interaction_revision`).
    settled: Cell<f32>,
    moving: Cell<bool>,
    /// The palette asked for a scroll (an arrow's or an answer's reveal) and the list hasn't
    /// come to rest since: its motion is the palette's, not the user's (review round 3, B2).
    own_scroll: Cell<bool>,
    /// The palette's key as the latest view built it, for the scroll requests an answer makes.
    pub(crate) key: RefCell<String>,
    /// Where focus goes when a result opens, as the latest view built it
    /// ([`super::SearchPalette::return_focus_on_open`]).
    pub(crate) return_to: RefCell<Option<String>>,
    pub(crate) handler: HandlerSlot,
}

impl PaletteState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the palette is open.
    pub fn is_open(&self) -> bool {
        self.mode.is_some()
    }

    /// The mode it is open in.
    pub fn mode(&self) -> Option<&PaletteMode> {
        self.mode.as_ref()
    }

    /// The current query's generation.
    pub fn generation(&self) -> u64 {
        self.model.generation()
    }

    /// What the body shows, or `None` while closed.
    pub fn phase(&self) -> Option<Phase> {
        Some(self.model.phase(self.mode.as_ref()?))
    }

    /// The results shown.
    pub fn items(&self) -> &[PaletteItem] {
        self.model.shown()
    }

    /// The highlighted result.
    pub fn highlighted(&self) -> Option<usize> {
        self.model.highlighted()
    }

    /// The highlighted result's item.
    pub fn highlighted_item(&self) -> Option<&PaletteItem> {
        self.highlighted().and_then(|i| self.items().get(i))
    }

    /// Highlights result `i` (a scene, or a host restoring a palette).
    pub fn set_highlighted(&mut self, i: Option<usize>) {
        self.model.set_highlighted(i);
    }

    /// Whether the current query still runs: the spinner shows.
    pub fn busy(&self) -> bool {
        self.is_open() && self.model.busy()
    }

    /// Why the shown results are partial, if they are.
    pub fn partial(&self) -> Option<&str> {
        self.model.partial()
    }

    /// The result count assistive technology was last told.
    pub fn announced(&self) -> Option<usize> {
        self.model.announced()
    }

    /// Opens in `mode` with an empty query, and asks for it. On an open palette it switches
    /// mode and starts over.
    pub fn open(&mut self, mode: PaletteMode) -> Polled {
        self.model.reset();
        self.edit = TextEdit::new("");
        let query = self.model.request("", &mode);
        self.mode = Some(mode);
        let mut out = Polled::changed();
        out.push(PaletteIntent::Query(query));
        out
    }

    /// Ctrl+K and Ctrl+Shift+K: closes an open palette, whatever its mode (Strata), else opens
    /// it in `mode`.
    pub fn toggle(&mut self, mode: PaletteMode) -> Polled {
        if self.is_open() {
            self.close_by(By::Toggle)
        } else {
            self.open(mode)
        }
    }

    /// Closes it ([`By::Host`]).
    pub fn close(&mut self) -> Polled {
        self.close_by(By::Host)
    }

    fn close_by(&mut self, by: By) -> Polled {
        if self.mode.take().is_none() {
            return Polled::default();
        }
        self.model.reset();
        self.edit = TextEdit::new("");
        let mut out = Polled::changed();
        out.push(PaletteIntent::Closed { by });
        out
    }

    /// Sets the query to `text`, the caret at its end, and asks for it: a host opening on a
    /// query, or a scene.
    pub fn set_query(&mut self, text: &str) -> Polled {
        if !self.is_open() {
            return Polled::default();
        }
        self.edit.replace_text(text);
        self.edit.set_caret(text.len());
        self.edited()
    }

    /// An answer to a [`PaletteIntent::Query`]. One for another generation, after the palette
    /// closed, or while nothing can be searched, is dropped.
    ///
    /// `tx` scrolls the list in the commit that draws the answer, as Strata's `restore_scroll`
    /// does: a new query goes back to the top; until the user moves the highlight, the list
    /// keeps its offset, so better results a batch puts above the first row show; after,
    /// the highlighted row stays in view. Call it from the host's poll (or any handler).
    pub fn deliver(&mut self, answer: Answer, tx: &mut Tx) -> Polled {
        let Some(mode) = &self.mode else {
            return Polled::default();
        };
        let Some(follow) = self.model.arrive(answer, mode) else {
            return Polled::default();
        };
        let key = self.key.borrow();
        let results = super::results_key(&key);
        // Reveals only: "just far enough", and a wheel or a fling wins over them (Rune), so a
        // batch never undoes the user's scroll. Rows are keyed by place, so keeping the offset
        // takes no request at all.
        // A batch's reveal is the palette's own scroll: the next ticks don't take it for the
        // user scrolling (review round 3, B2). A new query's top needs no mark: after it, only an
        // arrow (which marks its own) makes the next batch reveal.
        let own = match follow {
            Follow::Reveal(_) => !self.moving.get(),
            Follow::Top | Follow::Stay | Follow::Keep => false,
        };
        if own && !key.is_empty() {
            self.own_scroll.set(true);
        }
        match follow {
            // Not built yet: nothing to scroll.
            _ if key.is_empty() => {}
            Follow::Stay | Follow::Keep => {}
            Follow::Top => tx.scroll_into_view_in(results, super::option_key(&key, 0)),
            // A list the user is scrolling (a fling, a wheel's motion) keeps moving.
            Follow::Reveal(_) if self.moving.get() => {}
            Follow::Reveal(i) => tx.scroll_into_view_in(results, super::option_key(&key, i)),
        }
        Polled::changed()
    }

    /// The palette asked for a scroll of its own (an arrow's reveal): the next ticks don't take
    /// the list's motion for the user scrolling until it comes to rest.
    pub(crate) fn mark_own_scroll(&self) {
        self.own_scroll.set(true);
    }

    /// The query `generation` failed: the shown results go and the body says `message`, until
    /// the next edit or a later answer for the same query. False for another generation, a
    /// closed palette or one that can't search.
    pub fn fail(&mut self, generation: u64, message: impl Into<String>) -> bool {
        self.is_open() && self.model.fail(generation, message.into())
    }

    /// Nothing can be searched (Strata: no local search locations): the body says `message`
    /// and the field takes no text until the palette closes.
    pub fn unavailable(&mut self, message: impl Into<String>) -> bool {
        if !self.is_open() {
            return false;
        }
        self.model.unavailable(message.into());
        true
    }

    /// Opens the result `id`'s containing folder: the host's result menu.
    pub fn reveal(&mut self, id: &str, tx: &mut Tx) -> Polled {
        if !self.is_open() || !self.items().iter().any(|it| it.id == id) {
            return Polled::default();
        }
        self.finish(PaletteIntent::Reveal { id: id.to_owned() }, By::Reveal, tx)
    }

    /// One frame: tells the palette's own scrolls from the user's.
    pub fn tick(&mut self) {
        let offset = self.feed.get().offset[1];
        self.moving.set(offset != self.settled.replace(offset));
        if !self.moving.get() {
            // At rest: whatever the palette asked for has landed.
            self.own_scroll.set(false);
        } else if self.own_scroll.get() {
            // The palette's own scroll, still landing: not the user's.
            self.moving.set(false);
        }
    }

    // The palette's own handlers.

    /// The field's text changed.
    pub(crate) fn edited(&mut self) -> Polled {
        let Some(mode) = &self.mode else {
            return Polled::default();
        };
        let text = self.edit.text.clone();
        let query = self.model.request(&text, mode);
        let mut out = Polled::changed();
        out.push(PaletteIntent::Query(query));
        out
    }

    pub(crate) fn escape(&mut self) -> Polled {
        self.close_by(By::Escape)
    }

    /// Rune's modal dismissed the panel (a click outside).
    pub(crate) fn dismissed(&mut self) -> Polled {
        self.close_by(By::Dismiss)
    }

    pub(crate) fn step(&mut self, delta: i32) -> bool {
        self.is_open() && self.model.step(delta)
    }

    /// Opens the highlighted result.
    pub(crate) fn open_highlighted(&mut self, new_tab: bool, tx: &mut Tx) -> Polled {
        match self.highlighted_item().map(|it| it.id.clone()) {
            Some(id) => self.choose(&id, new_tab, Via::Key, tx),
            None => Polled::default(),
        }
    }

    /// Opens the highlighted result's folder.
    pub(crate) fn reveal_highlighted(&mut self, tx: &mut Tx) -> Polled {
        match self.highlighted_item().map(|it| it.id.clone()) {
            Some(id) => self.reveal(&id, tx),
            None => Polled::default(),
        }
    }

    /// Opens result `id`.
    pub(crate) fn choose(&mut self, id: &str, new_tab: bool, via: Via, tx: &mut Tx) -> Polled {
        if !self.is_open() || !self.items().iter().any(|it| it.id == id) {
            return Polled::default();
        }
        let intent = PaletteIntent::Open {
            id: id.to_owned(),
            new_tab,
            via,
        };
        self.finish(intent, By::Open, tx)
    }

    /// Closes after a result opened, focusing the host's
    /// [`super::SearchPalette::return_focus_on_open`]: focus the closing change puts outside the
    /// modal stays there, so the modal hands nothing back to its opener.
    fn finish(&mut self, intent: PaletteIntent, by: By, tx: &mut Tx) -> Polled {
        let mut out = Polled::changed();
        out.push(intent);
        out.intents.extend(self.close_by(by).intents);
        if let Some(to) = self.return_to.borrow().clone() {
            tx.focus(to, tx.focus_visible() || tx.by_keys());
        }
        out
    }

    /// A right-click on result `id`.
    pub(crate) fn menu(&mut self, id: &str, at: Point) -> Polled {
        if !self.is_open() || self.model.highlight_id(id).is_none() {
            return Polled::default();
        }
        let mut out = Polled::changed();
        out.push(PaletteIntent::ResultMenu {
            id: id.to_owned(),
            at,
        });
        out
    }
}
