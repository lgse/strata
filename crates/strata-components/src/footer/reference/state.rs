//! [`ReferenceState`]: whether the reference is open, its query and category, held by the host
//! and reached by accessor; and [`ReferenceKeys`], the keys it answers.

use std::cell::RefCell;
use std::fmt;
use std::ops::Range;
use std::rc::Rc;

use rune::{Chord, ChordKey, ChordMod, KeyEvent, TextEdit, Tx};

use super::data::ShortcutData;
use super::search::{Index, Lines, fold_query, lines};

/// The shortcut reference's state. It outlives the panel: the query and the category survive
/// closing and reopening, and while it stays open, the window losing focus changes nothing (the
/// scroll position included), as in Strata. Each open shows the results from the top: the panel
/// mounts afresh.
///
/// Used by: hosts, through [`ShortcutReference`](super::ShortcutReference) and the footer's
/// F1 button.
pub struct ReferenceState {
    open: bool,
    /// The search field.
    pub search: TextEdit,
    /// Interior: [`ReferenceState::lines`] forgets a title the data lost while building the view.
    category: RefCell<Option<String>>,
    pub(super) visible: Range<usize>,
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
            visible: 0..0,
            width: None,
            height: None,
            cache: RefCell::default(),
        }
    }

    /// Whether the reference shows.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens it with the query and category it had, its results from the top. Focus goes to
    /// the search field (the panel's first focus stop). A host holding a footer prompt open
    /// while the reference has the keyboard calls C18's `PromptState::hold_open(true)`.
    pub fn open(&mut self) {
        self.open = true;
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
    }

    /// The chosen category's title, or `None` for All. A title the data loses is forgotten
    /// once the panel shows the data without it, so the section coming back later isn't chosen
    /// again behind the user's back.
    pub fn category(&self) -> Option<String> {
        self.category.borrow().clone()
    }

    /// Chooses a category by title, or All.
    pub fn set_category(&mut self, title: Option<String>) {
        *self.category.get_mut() = title;
    }

    /// The chosen category's section in `data`, if it still has it.
    pub(super) fn section(&self, data: &ShortcutData) -> Option<usize> {
        let category = self.category.borrow();
        let title = category.as_deref()?;
        data.sections.iter().position(|s| s.title == title)
    }

    /// The results for the current query and category, `columns` rows to a line, from the
    /// cache while nothing they depend on changed.
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

/// The keys the reference answers. [`ReferenceKeys::default`] is Strata's default map: F1
/// toggles, Ctrl+B, Ctrl+F and Ctrl+L focus the categories, the search and the list;
/// [`ReferenceKeys::tenxer`] adds `~` as a toggle. Which chords toggle where (not `~` from a
/// text field) is the host's key dispatch; the reference matches them inside its panel.
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
    /// Strata's 10xer map: `~` toggles too, typed with Shift (US) or without (AltGr on German
    /// and French layouts).
    pub fn tenxer() -> Self {
        let mut keys = Self::default();
        keys.toggle.push(tilde());
        keys.toggle.push(Chord::new(ChordKey::Char('~')));
        keys
    }

    /// Whether `ev` is a toggle chord, following the layout as shortcuts do (`Chord::matches_tx`):
    /// for the host's key dispatch, and inside the panel outside its search field. The chord
    /// matches exactly, so F1 with Ctrl, Alt, Super or Shift held doesn't toggle.
    pub fn toggles(&self, ev: &KeyEvent, tx: &Tx) -> bool {
        self.toggle.iter().any(|c| c.matches_tx(ev, tx))
    }

    /// Whether `ev` is a toggle chord by the character it typed alone (`Chord::matches`): in
    /// the search field, so a typed `~` closes the reference while a key that reads as `~`
    /// only through the layout fallback (Russian ё) types itself (lgse/strata#1714).
    pub fn toggles_typed(&self, ev: &KeyEvent) -> bool {
        self.toggle.iter().any(|c| c.matches(ev))
    }
}

/// `~`: Shift with the key left of 1 on a US layout.
pub fn tilde() -> Chord {
    Chord::new(ChordKey::Char('`')).with(ChordMod::Shift)
}
