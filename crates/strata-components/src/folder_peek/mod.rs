//! Folder peek (C22): resting the pointer on a folder in Icons or List shows its first rows in
//! a hover card beside it.
//!
//! Built on rune-kit's `HoverCard` and Rune's `El::hover_delays`, through C1e's rest hook
//! ([`Collection::rest`]). Off by default: the host turns it on from its preference
//! ([`PeekState::set_enabled`]). It owns no I/O: when a peek opens the host hears which
//! folder ([`FolderPeek::on_change`], [`PeekState::target`]) and lists it, then passes the
//! rows, the status and how many more there are back by accessor ([`PeekContents`]).
//!
//! # Host recipe
//!
//! ```ignore
//! struct Pane { listing: ListSource, coll: CollectionState, peek: PeekState,
//!               peeked: Peeked /* the host's listing of peek.target() */, look: Look }
//!
//! fn peek() -> FolderPeek<Pane> {
//!     FolderPeek::new(
//!         |p: &Pane| &p.peek,
//!         |p: &mut Pane| &mut p.peek,
//!         |p: &Pane| PeekContents::new(&p.peeked.name, &p.peeked.status, &p.peeked.rows).more(p.peeked.more),
//!         |p: &Pane| &p.look,
//!     )
//!     .on_change(|p: &mut Pane, change, tx| p.relist(change, tx)) // start or drop a listing
//!     .on_open(|p: &mut Pane, target, tx| p.open_folder(target.folder, tx))
//!     .bounds(|p: &Pane| Some(p.pane_rect)) // the pane, around C4's sideways scroll
//! }
//!
//! // The view: the folder rows wait on the pointer, the card goes in the root layer.
//! let base = Collection::new(/* … */)
//!     .marks(|p: &Pane, id| { let mut m = RowMarks::default();
//!                             m.peek_anchor = p.peek.is_anchor("files", id); m })
//!     .on_intent(|p: &mut Pane, intent, _| if let Intent::BeginDrag { .. } = intent {
//!         p.peek.close(); });
//! let list = ListTable::build(p, peek().rows(base), &mut layer); // or IconsGrid's base
//! peek().card(p, &mut layer);
//! rune_kit::root(page, layer)
//! ```
//!
//! - Attach it to the List and Icons collections only, whatever the preference: Columns never
//!   peeks. The flag is [`PeekState::set_enabled`]; rows keep waiting while it is off, so
//!   turning it on never peeks the folder under a parked pointer.
//! - Pass the pane's rect in window points as [`FolderPeek::bounds`] when the collection is
//!   wider than what shows of it: C4's List inside its sideways scroll is as wide as its
//!   columns. Without it, a card is placed against the columns' width.
//! - Call [`PeekState::close`] on C1b's `Intent::BeginDrag`, on navigation, on a view switch
//!   (a keyboard peek has no pointer leave to close it), and first in Escape's chain (it
//!   returns whether one was open).
//! - Call `CollectionState::hold_rest` on each peeking collection when any drag that didn't
//!   start in it ends: a sidebar row, a tab, another pane, a P1 or X2 system drag. The folder
//!   a drag is dropped on then never peeks until the pointer arrives on a row again. (A drag of
//!   the collection's own holds by itself.)
//! - Call `CollectionState::hold_rest` on each peeking collection when the preference turns
//!   peeking on or off, so a rest under way then never opens a peek.
//! - Follow C4's sideways scroll with [`FolderPeek::follow_sideways`], through
//!   `ListTable::follow_sideways`: a sideways scroll closes the peek, and its card is never
//!   drawn moved. The table keeps the scroll's one handler and calls every follower from it,
//!   after its own scroll bars, so neither drops the other.
//! - Call [`PeekState::set_covered`] while an overlay covers the collections and back when it
//!   goes: a context menu (C21), a popover (`rune_kit::Popover`), a dialog (C15), the search
//!   palette (C17), anything over the rows with or without a scrim. Rune counts the rows under
//!   an overlay as hovered (Rune draft U-C22-1).
//! - The file chooser never turns peeking on.
//! - Hidden entries, the order and the 64-entry cap are the host's: it passes the rows it
//!   wants shown, in its order. Give a capped count as [`More::AtLeast`].
//! - Thumbnails: [`FolderPeek::thumbnails`] draws C25's handles; the host keeps a viewer of
//!   the peek's rows in its store.
//! - The keyboard peek (Strata's `i` in 10xer mode): [`PeekState::toggle`] at the cursor's
//!   row, with C1e's `Collection::window_rect_of`, and [`FolderPeek::viewport`] of
//!   `CollectionState::viewport`.
//!
//! # Decisions
//!
//! - **The waits are Rune's.** Each folder row waits 500 ms (`hover_delays`) before it reports
//!   the pointer resting on it, and 80 ms after the pointer left; the card reports at once
//!   and 80 ms after. A peek stays open while either its row or its card reports the pointer
//!   on it. The row a peek is open from waits no time, so the pointer coming back from the card
//!   keeps the peek. C22 owns no timer.
//! - **A peek starts only from the pointer arriving on a folder**, as Strata's starts only from
//!   a motion enter (C1e's rest hook): not on a folder a scroll carries under a parked pointer,
//!   one a drag was dropped on, nor one under a parked pointer when peeking turns on; moving
//!   inside the row doesn't count, leaving it (for another row, or out of the collection) for
//!   more than its 80 ms leave delay and coming back does.
//! - **A peek closes when its folder leaves the view** (scrolled out, unmounted, removed),
//!   whether the pointer or a key opened it. Rune's `on_visible_rows` gives no transaction, so
//!   the host's [`FolderPeek::on_change`] hears that close with one whose scrolls and wakes go
//!   nowhere.
//! - **Placement is Strata's rule** ([`place`]), worked out once as the peek opens (as Strata
//!   sets its revealer's margins once), measured against the collection's viewport inside
//!   [`FolderPeek::bounds`]:
//!   8 pt right of the item, else 8 pt left of it, else over the viewport's trailing edge, and
//!   no peek in a viewport narrower than the card. The card is a `HoverCard` anchored
//!   RightStart to a zero-width part C22 pins in the item's row, at the offset that lands the
//!   card where the rule says, so Rune never flips it and the card rides the row as it scrolls.
//! - **Assistive technology** hears a group named "Folder peek" (Strata's name): a heading
//!   named by the folder, a progress bar while it loads, a list of up to 8 items named by the
//!   entries, and the "and N more", empty or error text. Icons, chevrons and the anchor are
//!   hidden. A click on the card opens the folder (Strata's `commit_peek`) through a press
//!   layer hidden from assistive technology, so the card reads as a group, not a button.
//!
//! # Deviations
//!
//! 1. The card keeps 8 pt inside the viewport's trailing edge, and opens left only 8 pt clear
//!    of the window's leading edge (Rune's anchored margin); Strata lets it touch them. In a
//!    viewport 256 to 263 pt wide the card still shows (Strata shows it at 262) and hangs past
//!    it by up to 8 pt: past the trailing edge when the pane starts at the window's edge.
//! 2. The entrance is `HoverCard`'s, a fade and a 0.96 scale from its top-left corner on
//!    either side; Strata slides from the item's side over 150 ms.
//! 3. `HoverCard`'s 4 pt inner padding, raised surface and elevation, where Strata's header is
//!    flush with the border and its shadow is the glow.
//! 4. From an open peek onto another folder, the first closes 80 ms after the pointer left it
//!    and the second opens at 500 ms. Strata keeps the first open until the second opens.
//! 5. Rows under the card start no peek while the pointer is on the card, and a row the card
//!    covers peeks only once the pointer leaves it and comes back. Rows under the host's other
//!    overlays (a menu's scrim, a popover, a dialog) start none while the host says they are
//!    covered ([`PeekState::set_covered`]); without that call, Rune would peek them (Rune
//!    draft U-C22-1).
//! 6. No loading skeleton: a spinner in the header over an empty body. Strata shows its Miller
//!    skeleton after a delay (owner ruling).
//! 7. "and N more" is new (the work package's); Strata shows no such line.
//! 8. A keyboard-opened peek is a polite live region, so it is announced (owner ruling);
//!    Strata's popover is not.
//! 9. A press on a folder cancels its wait (Rune). After a press of the collection's own ends,
//!    and after any other drag ends once the host calls `CollectionState::hold_rest`, no
//!    folder peeks until the pointer arrives on a row again, so a drag never ends in a peek.
//!    Strata's plain click leaves a pending peek running.
//! 10. A quick out and back (under the 80 ms leave delay) is no leave to Rune, so it doesn't
//!     restart a folder's wait; Strata's motion enter would.
//! 11. A sideways scroll of the list closes the peek (owner ruling): the card is hidden in the
//!     frame the scroll moves the folder's row ([`FolderPeek::follow_sideways`]), and the peek closes
//!     on the next. Strata's card stays where it opened, in its overlay.
//!
//! # Rune drafts (not filed)
//!
//! Headless repros at the pinned rev in `strata-spikes/c22-probes` (`src/lib.rs`).
//!
//! - **U-C22-1 (a comment for anomalyco/rune#265): hover reaches elements under an overlay.**
//!   `on_hover_change` and `hover_delays` treat every element whose hit rect holds the pointer
//!   as hovered (`runtime/input/hover.rs`, `within_next` walks every hit), as
//!   `on_pointer_move` does (#265). Repro: rows with `hover_delays(500 ms, 80 ms)`, and a
//!   `HoverCard` anchored over them; rest the pointer on the card: after 0.5 s the row under
//!   it reports `true`, and its `on_pointer_move_local` hears every move. The same with
//!   `rune_kit::scrim` over the whole window (a menu's outside-press catcher, which takes
//!   every press) and with any pressable layer element: the row under it reports `true`
//!   after 0.48 s. Ask: hover and its delays stop at the topmost element hit, as presses do.
//! - **U-C22-2: rune-kit `HoverCard` is fixed where a peek needs choices.** `Placement` has
//!   no LeftStart, LeftEnd or RightEnd, the gap is 4 pt and the width 200 to 320 pt, the
//!   surface takes no hover handler, and the card has no role or name: assistive technology
//!   hears an unnamed group (`kit-root/{key}-card`, role `group`, no label). Ask: those
//!   placements, a gap and width, hover on the surface, and a role and label (Kobalte's
//!   `HoverCard.Content` takes ARIA attributes).
//! - **U-C22-3: anchored overlays are bounded by the window only.** `anchor_position` flips
//!   and clamps against the window (`runtime/anchor.rs`), so a card beside a list next to a
//!   drawer opens over the drawer. Repro: a 400 pt list at x 100 and a 300 pt drawer at x 500
//!   in an 800 pt window; a 256 pt card anchored RightStart to a row lands at x 508, over the
//!   drawer. Ask: a boundary element for flip and clamp, as Floating UI's `boundary`. Related:
//!   #362 (report the side a placement flipped to).
//!
//! At Rune 2a44156, U-C22-1, U-C22-2 and U-C22-3 still stand. Hover input moved to new press
//! and focus types with the same rule (every hit under the pointer), `HoverCard` is unchanged,
//! and anchoring only gained right-to-left mirroring. The peek owns no timer (`hover_delays`),
//! so `Tx::clock` changes nothing here.
//!
//! Work package: C22, `docs/work/phase-1-leaves/C22-folder-peek.md`.
//!
//! [`Collection::rest`]: crate::collection::element::Collection::rest

#![warn(missing_docs)]

mod attach;
mod card;
mod place;
mod state;

#[cfg(test)]
mod tests;

use std::rc::Rc;
use std::time::Duration;

use rune::{El, Tx};
use strata_kit::look::Look;

pub use place::{GAP, Placement, Side, WIDTH, place};
pub use state::{Change, Origin, PeekState, PeekTarget};

use crate::collection::element::Collection;
use crate::contract::{EntrySource, Rect};
use crate::thumbnails::ThumbnailStore;

/// How the peek times and fills: Strata's `PeekBehavior`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PeekBehavior {
    /// How long the pointer rests on a folder before its peek opens.
    pub open: Duration,
    /// How long after the pointer left both the folder and the card the peek closes.
    pub close: Duration,
    /// The most rows the card lists.
    pub rows: usize,
}

impl PeekBehavior {
    /// Strata's: 500 ms, 80 ms and 8 rows.
    pub const STRATA: Self = Self {
        open: Duration::from_millis(500),
        close: Duration::from_millis(80),
        rows: 8,
    };
}

impl Default for PeekBehavior {
    fn default() -> Self {
        Self::STRATA
    }
}

/// Where the peeked folder's listing stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PeekStatus {
    /// Still listing: the header shows a spinner, and rows show as they arrive.
    #[default]
    Loading,
    /// Listed: the rows, or "This directory is empty" with none.
    Ready,
    /// It couldn't be read: the message, under "Unable to read this directory". The card
    /// offers no Open as Administrator.
    Error(String),
}

/// How many entries past the shown rows the folder holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum More {
    /// Exactly this many.
    Exactly(usize),
    /// At least this many: the host's listing stopped at its cap.
    AtLeast(usize),
}

/// What the card shows, from the host.
///
/// Fields may be added: build one with [`PeekContents::new`].
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct PeekContents<'a> {
    /// The peeked folder's name, which heads the card.
    pub title: &'a str,
    /// The listing's status.
    pub status: &'a PeekStatus,
    /// The rows, in the host's order: the card shows the first [`PeekBehavior::rows`].
    pub entries: &'a dyn EntrySource,
    /// How many more there are; `None` counts the entries past the rows shown.
    pub more: Option<More>,
}

impl<'a> PeekContents<'a> {
    /// Folder `title`'s `entries` at `status`, the more line counting the entries past the rows
    /// shown.
    pub fn new(title: &'a str, status: &'a PeekStatus, entries: &'a dyn EntrySource) -> Self {
        Self {
            title,
            status,
            entries,
            more: None,
        }
    }

    /// The more line's count, from the host.
    pub fn more(mut self, more: Option<More>) -> Self {
        self.more = more;
        self
    }
}

type MoreFn = Rc<dyn Fn(More) -> String>;

/// The words the card shows and assistive technology reads. Strata passes translated text;
/// the defaults are English (Strata's).
///
/// Used by: hosts.
#[derive(Clone)]
#[non_exhaustive]
pub struct PeekLabels {
    /// The card's name.
    pub peek: String,
    /// The header's spinner.
    pub loading: String,
    /// An empty folder.
    pub empty: String,
    /// Above the error's message.
    pub unreadable: String,
    /// The more line: "and 12 more", "and 56 or more".
    pub more: MoreFn,
}

impl Default for PeekLabels {
    fn default() -> Self {
        Self {
            peek: "Folder peek".into(),
            loading: "Loading".into(),
            empty: "This directory is empty".into(),
            unreadable: "Unable to read this directory".into(),
            more: Rc::new(|more| match more {
                More::Exactly(n) => format!("and {n} more"),
                More::AtLeast(n) => format!("and {n} or more"),
            }),
        }
    }
}

type GetFn<S> = Rc<dyn Fn(&S) -> &PeekState>;
type GetMutFn<S> = Rc<dyn Fn(&mut S) -> &mut PeekState>;
type ContentsFn<S> = Rc<dyn for<'a> Fn(&'a S) -> PeekContents<'a>>;
type LookFn<S> = Rc<dyn Fn(&S) -> &Look>;
type ChangeFn<S> = Rc<dyn Fn(&mut S, Change, &mut Tx)>;
type OpenFn<S> = Rc<dyn Fn(&mut S, PeekTarget, &mut Tx)>;
type ThumbsFn<S> = Rc<dyn Fn(&S) -> &ThumbnailStore>;
type BoundsFn<S> = Rc<dyn Fn(&S) -> Option<Rect>>;

/// What the peek was built with, shared by its handlers.
pub(crate) struct Shared<S> {
    get: GetFn<S>,
    get_mut: GetMutFn<S>,
    contents: ContentsFn<S>,
    look: LookFn<S>,
    labels: PeekLabels,
    behavior: PeekBehavior,
    thumbs: Option<ThumbsFn<S>>,
    bounds: Option<BoundsFn<S>>,
    on_change: Option<ChangeFn<S>>,
    on_open: Option<OpenFn<S>>,
}

impl<S> Clone for Shared<S> {
    fn clone(&self) -> Self {
        Self {
            get: self.get.clone(),
            get_mut: self.get_mut.clone(),
            contents: self.contents.clone(),
            look: self.look.clone(),
            labels: self.labels.clone(),
            behavior: self.behavior,
            thumbs: self.thumbs.clone(),
            bounds: self.bounds.clone(),
            on_change: self.on_change.clone(),
            on_open: self.on_open.clone(),
        }
    }
}

impl<S: 'static> Shared<S> {
    /// Tells the host what a transition did; one that changed nothing the view reads commits
    /// nothing.
    fn changed(&self, s: &mut S, change: Change, tx: &mut Tx) {
        match change {
            Change::None => tx.live_only(),
            _ => {
                if let Some(f) = &self.on_change {
                    f(s, change, tx);
                }
            }
        }
    }
}

/// The folder peek's builder: [`FolderPeek::rows`] on the List or Icons collection, and
/// [`FolderPeek::card`] into the root layer. See the [module docs](self) for the recipe.
///
/// Used by: hosts.
pub struct FolderPeek<S> {
    sh: Shared<S>,
}

impl<S: 'static> FolderPeek<S> {
    /// A peek over the state `get` and `get_mut` return, showing what `contents` returns, drawn
    /// with the look `look` returns.
    pub fn new(
        get: impl Fn(&S) -> &PeekState + 'static,
        get_mut: impl Fn(&mut S) -> &mut PeekState + 'static,
        contents: impl for<'a> Fn(&'a S) -> PeekContents<'a> + 'static,
        look: impl Fn(&S) -> &Look + 'static,
    ) -> Self {
        Self {
            sh: Shared {
                get: Rc::new(get),
                get_mut: Rc::new(get_mut),
                contents: Rc::new(contents),
                look: Rc::new(look),
                labels: PeekLabels::default(),
                behavior: PeekBehavior::STRATA,
                thumbs: None,
                bounds: None,
                on_change: None,
                on_open: None,
            },
        }
    }

    /// The words: English by default.
    pub fn labels(mut self, labels: PeekLabels) -> Self {
        self.sh.labels = labels;
        self
    }

    /// The delays and the row count: [`PeekBehavior::STRATA`] by default.
    pub fn behavior(mut self, behavior: PeekBehavior) -> Self {
        self.sh.behavior = behavior;
        self
    }

    /// Draws each row's thumbnail from C25's store over its icon, when it has a key and
    /// thumbnails are on. The host keeps a viewer of the peek's rows in the store.
    pub fn thumbnails(mut self, store: impl Fn(&S) -> &ThumbnailStore + 'static) -> Self {
        self.sh.thumbs = Some(Rc::new(store));
        self
    }

    /// The viewport a peek of `s` measures, inside [`FolderPeek::bounds`]: pass it to
    /// [`PeekState::open`] and [`PeekState::toggle`] with the collection's own viewport
    /// (`CollectionState::viewport`).
    pub fn viewport(&self, s: &S, viewport: Rect) -> Rect {
        match self.sh.bounds.as_ref().and_then(|f| f(s)) {
            Some(bounds) => attach::clip(viewport, bounds),
            None => viewport,
        }
    }

    /// A follower of the sideways scroll around the collection, for C4's
    /// `ListTable::follow_sideways`: from the frame a sideways scroll moves the peeked folder,
    /// its card is hidden, and the peek closes on the next frame (owner ruling; Strata's card
    /// stays where it opened, in its overlay). Without it a sideways scroll still closes the
    /// peek, a frame late, with the card drawn moved in that frame.
    ///
    /// ```ignore
    /// ListTable::new(/* … */).follow_sideways(|p: &Pane| peek().follow_sideways(p))
    /// ```
    pub fn follow_sideways(&self, s: &S) -> impl Fn(rune::ScrollPos) + use<S> {
        let st = (self.sh.get)(s);
        let lives = st.lives.clone();
        let at = st.sideways_at();
        move |p: rune::ScrollPos| {
            let x = p.offset[0];
            lives.sideways.set(x);
            if at.is_some_and(|at| (x - at).abs() > 0.5) {
                lives.shown.set(0.0);
            }
        }
    }

    /// The part of the window the collection shows itself in, in window points, when that is
    /// less than its own viewport: the pane around C4's sideways scroll, whose collection is as
    /// wide as its columns. Placement and the narrow-viewport rule measure the viewport inside
    /// it. `None` (or none set) measures the collection's viewport.
    pub fn bounds(mut self, f: impl Fn(&S) -> Option<Rect> + 'static) -> Self {
        self.sh.bounds = Some(Rc::new(f));
        self
    }

    /// Hears a peek open or close from the pointer, and close as its folder leaves the view:
    /// the host starts or drops its listing of [`PeekState::target`]. A close from the view
    /// comes with a transaction whose scrolls and wakes go nowhere (Rune's `on_visible_rows`
    /// gives none). A host's own
    /// calls ([`PeekState::open`], [`PeekState::toggle`], [`PeekState::close`],
    /// [`PeekState::set_enabled`]) return their [`Change`] instead.
    pub fn on_change(mut self, f: impl Fn(&mut S, Change, &mut Tx) + 'static) -> Self {
        self.sh.on_change = Some(Rc::new(f));
        self
    }

    /// Hears a click on the card: open its folder (Strata's `commit_peek`). The peek closes
    /// first.
    pub fn on_open(mut self, f: impl Fn(&mut S, PeekTarget, &mut Tx) + 'static) -> Self {
        self.sh.on_open = Some(Rc::new(f));
        self
    }

    /// `base` with its folder rows waiting on the pointer, and the anchor in the row a peek is
    /// open from: for the List (C4's base) and Icons (C3's base) collections.
    pub fn rows(&self, base: Collection<S>) -> Collection<S> {
        attach::rows(&Rc::new(self.sh.clone()), base)
    }

    /// Pushes the open peek's card into `layer`, the root layer (`rune_kit::root`), after
    /// the collection.
    pub fn card(&self, s: &S, layer: &mut Vec<El>) {
        if let Some(card) = card::card(&Rc::new(self.sh.clone()), s) {
            layer.push(card);
        }
    }
}

/// The key of the zero-width part the card anchors to, in collection `scope`'s peeked row.
pub fn anchor_key(scope: &str) -> String {
    format!("{scope}-peek-trigger")
}

/// The key of collection `scope`'s card (`HoverCard`'s, `{key}-card`).
pub fn card_key(scope: &str) -> String {
    format!("{scope}-peek-card")
}
