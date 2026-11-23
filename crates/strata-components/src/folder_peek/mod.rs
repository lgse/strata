// SPDX-License-Identifier: MIT

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
    /// It couldn't be read: the message, under "Unable to read this directory".
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
    /// Folder `title`'s `entries` at `status`, the more line counting the entries past the rows shown.
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

/// The words the card shows and assistive technology reads.
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

/// The folder peek's builder: `FolderPeek::rows` on the List or Icons collection.
pub struct FolderPeek<S> {
    sh: Shared<S>,
}

impl<S: 'static> FolderPeek<S> {
    /// A peek over the state `get` and `get_mut` return, showing what `contents` returns, drawn with the look `look` returns.
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

    /// Draws each row's thumbnail from store over its icon, when it has a key and thumbnails are on.
    pub fn thumbnails(mut self, store: impl Fn(&S) -> &ThumbnailStore + 'static) -> Self {
        self.sh.thumbs = Some(Rc::new(store));
        self
    }

    /// The viewport a peek of `s` measures, inside `FolderPeek::bounds`.
    pub fn viewport(&self, s: &S, viewport: Rect) -> Rect {
        match self.sh.bounds.as_ref().and_then(|f| f(s)) {
            Some(bounds) => attach::clip(viewport, bounds),
            None => viewport,
        }
    }

    /// A follower of the sideways scroll around the collection, for `ListTable::follow_sideways`.
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

    /// The part of the window the collection shows itself in, in window points, when that is less than its own viewport.
    pub fn bounds(mut self, f: impl Fn(&S) -> Option<Rect> + 'static) -> Self {
        self.sh.bounds = Some(Rc::new(f));
        self
    }

    /// Hears a peek open or close from the pointer, and close as its folder leaves the view.
    pub fn on_change(mut self, f: impl Fn(&mut S, Change, &mut Tx) + 'static) -> Self {
        self.sh.on_change = Some(Rc::new(f));
        self
    }

    /// Hears a click on the card: open its folder.
    pub fn on_open(mut self, f: impl Fn(&mut S, PeekTarget, &mut Tx) + 'static) -> Self {
        self.sh.on_open = Some(Rc::new(f));
        self
    }

    /// `base` with its folder rows waiting on the pointer, and the anchor in the row a peek is open from.
    pub fn rows(&self, base: Collection<S>) -> Collection<S> {
        attach::rows(&Rc::new(self.sh.clone()), base)
    }

    /// Pushes the open peek's card into `layer`, the root layer (`rune_kit::root`), after the collection.
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
