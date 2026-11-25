// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use rune::{Chord, ChordKey, ChordMod, KeyEvent, ScrollPos, TextEdit, Tx};

use super::data::ShortcutData;
use super::search::{Index, Lines, fold_query, lines};

#[derive(Clone, Copy, Debug, Default)]
struct ScrollTrack {
    offset_y: f32,
    max_y: Option<f32>,
    viewport_h: f32,
}

pub struct ReferenceState {
    open: bool,
    /// The search field.
    pub search: TextEdit,
    /// Interior: [`ReferenceState::lines`] forgets a title the data lost while building the view.
    category: RefCell<Option<String>>,
    /// Tracked results viewport scroll position, kept current by `El::on_scroll_offset` and
    /// stepped eagerly on scroll keys so held repeats accumulate before the next frame draws.
    scroll: Rc<Cell<ScrollTrack>>,
    pub(super) width: Option<f32>,
    /// The panel's height as last laid out: the compact category pills take at most a quarter.
    pub(super) height: Option<f32>,
    cache: RefCell<Cache>,
}

#[derive(Default)]
struct Cache {
    index: Rc<Index>,
    lines: Option<(LinesKey, Rc<Lines>)>,
}

#[derive(Clone, PartialEq, Eq)]
struct LinesKey {
    revision: u64,
    query: String,
    category: Option<usize>,
    columns: usize,
}

impl Default for ReferenceState {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ReferenceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReferenceState")
            .field("open", &self.open)
            .field("query", &self.search.text)
            .field("category", &self.category.borrow())
            .finish_non_exhaustive()
    }
}

impl ReferenceState {
    /// Closed, with no query, on All.
    pub fn new() -> Self {
        Self {
            open: false,
            search: TextEdit::default(),
            category: RefCell::new(None),
            scroll: Rc::new(Cell::new(ScrollTrack::default())),
            width: None,
            height: None,
            cache: RefCell::default(),
        }
    }

    /// Whether the reference shows.
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.reset_scroll();
        let end = self.search.text.len();
        self.search.anchor = 0;
        self.search.focus = end;
    }

    /// Closes it. Rune's modal gives the keyboard back to what had it before it opened.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Opens it when closed, closes it when open: F1, the footer's F1 button.
    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open();
        }
    }

    /// The query.
    pub fn query(&self) -> &str {
        &self.search.text
    }

    /// Replaces the query, the caret at its end.
    pub fn set_query(&mut self, query: &str) {
        self.search = TextEdit::new(query);
        let end = self.search.text.len();
        self.search.anchor = end;
        self.search.focus = end;
        self.reset_scroll();
    }

    pub fn category(&self) -> Option<String> {
        self.category.borrow().clone()
    }

    /// Chooses a category by title, or All.
    pub fn set_category(&mut self, title: Option<String>) {
        *self.category.get_mut() = title;
        self.reset_scroll();
    }

    /// The results viewport's current vertical scroll offset in points.
    pub fn scroll_offset(&self) -> f32 {
        self.scroll.get().offset_y
    }

    pub(super) fn scroll_watcher(&self) -> impl Fn(ScrollPos) + 'static {
        let scroll = self.scroll.clone();
        move |pos: ScrollPos| {
            scroll.set(ScrollTrack {
                offset_y: pos.offset[1],
                max_y: Some(pos.max[1]),
                viewport_h: pos.viewport[1],
            });
        }
    }

    pub(super) fn step_scroll(&self, delta: f32) -> f32 {
        let mut track = self.scroll.get();
        let next = match track.max_y {
            Some(max) => (track.offset_y + delta).clamp(0.0, max.max(0.0)),
            None => (track.offset_y + delta).max(0.0),
        };
        track.offset_y = next;
        self.scroll.set(track);
        next
    }

    pub(super) fn step_scroll_page(&self, pages: i32) {
        let mut track = self.scroll.get();
        if track.viewport_h <= 0.0 {
            return;
        }
        // Matches Rune's `Tx::scroll_page` step (90% of the viewport height).
        let delta = pages as f32 * track.viewport_h * 0.9;
        track.offset_y = match track.max_y {
            Some(max) => (track.offset_y + delta).clamp(0.0, max.max(0.0)),
            None => (track.offset_y + delta).max(0.0),
        };
        self.scroll.set(track);
    }

    pub(super) fn scroll_to_edge(&self, bottom: bool) {
        let mut track = self.scroll.get();
        track.offset_y = if bottom {
            track.max_y.unwrap_or(track.offset_y)
        } else {
            0.0
        };
        self.scroll.set(track);
    }

    pub(super) fn reset_scroll(&self) {
        self.scroll.set(ScrollTrack::default());
    }

    /// The chosen category's section in `data`, if it still has it.
    pub(super) fn section(&self, data: &ShortcutData) -> Option<usize> {
        let category = self.category.borrow();
        let title = category.as_deref()?;
        data.sections.iter().position(|s| s.title == title)
    }

    pub(super) fn lines(&self, data: &ShortcutData, columns: usize) -> Rc<Lines> {
        if self.section(data).is_none() {
            // Shown as All now: forget the title.
            self.category.borrow_mut().take();
        }
        let key = LinesKey {
            revision: data.revision,
            query: fold_query(&self.search.text),
            category: self.section(data),
            columns,
        };
        let mut cache = self.cache.borrow_mut();
        if let Some((k, l)) = &cache.lines
            && *k == key
        {
            return l.clone();
        }
        if cache.index.revision != Some(data.revision) {
            cache.index = Rc::new(Index::build(data));
        }
        let lines = Rc::new(lines(
            &data.sections,
            &cache.index,
            &key.query,
            key.category,
            columns,
        ));
        cache.lines = Some((key, lines.clone()));
        lines
    }

    /// The folded index of `data`'s rows (built by [`ReferenceState::lines`]).
    pub(super) fn index(&self) -> Rc<Index> {
        self.cache.borrow().index.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceKeys {
    pub toggle: Vec<Chord>,
    pub categories: Vec<Chord>,
    pub search: Vec<Chord>,
    pub list: Vec<Chord>,
}

impl Default for ReferenceKeys {
    fn default() -> Self {
        let ctrl = |c| vec![Chord::new(ChordKey::Char(c)).with(ChordMod::Ctrl)];
        Self {
            toggle: vec![Chord::new(ChordKey::Function(1))],
            categories: ctrl('b'),
            search: ctrl('f'),
            list: ctrl('l'),
        }
    }
}

impl ReferenceKeys {
    pub fn tenxer() -> Self {
        let mut keys = Self::default();
        keys.toggle.push(tilde());
        keys.toggle.push(Chord::new(ChordKey::Char('~')));
        keys
    }

    pub fn toggles(&self, ev: &KeyEvent, tx: &Tx) -> bool {
        self.toggle.iter().any(|c| c.matches_tx(ev, tx))
    }

    pub fn toggles_typed(&self, ev: &KeyEvent) -> bool {
        self.toggle.iter().any(|c| c.matches(ev))
    }
}

/// `~`: Shift with the key left of 1 on a US layout.
pub fn tilde() -> Chord {
    Chord::new(ChordKey::Char('`')).with(ChordMod::Shift)
}
