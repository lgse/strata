//! The shortcut reference (F1): a [`DialogShell`] panel listing the
//! active key map, with a search and category filters. See the [footer module](super) docs.

mod data;
mod keymap;
mod keys;
mod search;
mod state;
mod view;

use std::fmt;
use std::rc::Rc;

pub use data::{Context, Keys, Section, Shortcut, ShortcutData};
pub use keymap::{chord_label, keymap_rows};
pub use state::{ReferenceKeys, ReferenceState, tilde};

use rune::{Chord, El};
use strata_kit::look::Look;

use crate::dialog::DialogShell;

/// The panel's width when the window has room, in points: Strata's panel is the window less
/// 80 px, up to its results' 1200 plus the 225 px sidebar (`src/ui/shortcut_footer/reference.rs`).
pub const PANEL_WIDTH: f32 = 1440.0;

/// Below this panel width the reference lays out compact: the header stacks, the categories
/// wrap as pills without counts, and the results take one column. Strata switches at a 1480 px
/// window, whose panel is about 1400 wide; a Rune view knows its panel's width, not the window's.
pub const COMPACT_BELOW: f32 = 1400.0;

/// The categories' column in the wide layout (`.shortcut-reference-sidebar`'s `min-width`).
pub const SIDEBAR_WIDTH: f32 = 225.0;

/// The results' height when the window has room (Strata's `height_request(440)`, grown to
/// its 620 cap less the header and note); a short window shrinks it.
pub const RESULTS_HEIGHT: f32 = 560.0;

/// The panel's words. [`ReferenceLabels::default`] is Strata's English.
#[derive(Clone)]
pub struct ReferenceLabels {
    /// The panel's title and name: "Keyboard shortcuts".
    pub title: String,
    /// The search field's name and placeholder: "Search actions or keys…".
    pub search: String,
    /// The category list's name: "Categories".
    pub categories: String,
    /// The first category: "All".
    pub all: String,
    /// The results' name: "Shortcuts".
    pub results: String,
    /// Shown, and announced, when nothing matches: "No matching shortcuts".
    pub no_match: String,
    /// A number of rows, for each category's description and the announced result count:
    /// "12 shortcuts".
    pub count: Rc<dyn Fn(usize) -> String>,
    /// The note at the bottom, hint by hint; it wraps only between hints, after a "·".
    pub note: Vec<String>,
}

impl fmt::Debug for ReferenceLabels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReferenceLabels")
            .field("title", &self.title)
            .field("note", &self.note)
            .finish_non_exhaustive()
    }
}

impl Default for ReferenceLabels {
    fn default() -> Self {
        Self {
            title: "Keyboard shortcuts".to_owned(),
            search: "Search actions or keys\u{2026}".to_owned(),
            categories: "Categories".to_owned(),
            all: "All".to_owned(),
            results: "Shortcuts".to_owned(),
            no_match: "No matching shortcuts".to_owned(),
            count: Rc::new(|n| {
                if n == 1 {
                    "1 shortcut".to_owned()
                } else {
                    format!("{n} shortcuts")
                }
            }),
            note: note(&ReferenceKeys::default(), chord_label),
        }
    }
}

/// Strata's note for `keys` in English, each key written by `label`: "Ctrl+B categories",
/// "Ctrl+F search", "Ctrl+L list", "j / k / ↑ / ↓ move", "Tab cycle", "Escape close". A
/// rebound key shows its new chord; an unbound one leaves its hint out (Strata has no keyless
/// form yet, lgse/strata#1885).
pub fn note(keys: &ReferenceKeys, label: impl Fn(&Chord) -> String) -> Vec<String> {
    let first =
        |chords: &[Chord], word: &str| chords.first().map(|c| format!("{} {word}", label(c)));
    [
        first(&keys.categories, "categories"),
        first(&keys.search, "search"),
        first(&keys.list, "list"),
        Some("j / k / \u{2191} / \u{2193} move".to_owned()),
        Some("Tab cycle".to_owned()),
        Some("Escape close".to_owned()),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The shortcut reference over the [`ReferenceState`] `get` and `get_mut` reach, listing
/// `data`'s rows:
///
/// ```ignore
/// ShortcutReference::new("reference", |s: &App| &s.reference, |s: &mut App| &mut s.reference,
///         |s: &App| &s.shortcuts)
///     .keys(if s.tenxer { ReferenceKeys::tenxer() } else { ReferenceKeys::default() })
///     .reduced_motion(cx.is_reduced_motion())
///     .build(s, &s.look, &mut layer);
/// ```
///
/// Keys under `key`: `<key>-dialog` (the panel), [`ShortcutReference::search_key`],
/// [`ShortcutReference::category_key`], [`ShortcutReference::results_key`],
/// [`ShortcutReference::row_key`], `<key>-status` (the announced count) and `<key>-note`.
///
/// Used by: hosts (Strata's browser windows and the portal chooser).
pub struct ShortcutReference<S> {
    key: String,
    get: fn(&S) -> &ReferenceState,
    get_mut: fn(&mut S) -> &mut ReferenceState,
    data: fn(&S) -> &ShortcutData,
    labels: ReferenceLabels,
    keys: ReferenceKeys,
    compact_below: f32,
    reduced_motion: bool,
}

impl<S: 'static> ShortcutReference<S> {
    pub fn new(
        key: impl Into<String>,
        get: fn(&S) -> &ReferenceState,
        get_mut: fn(&mut S) -> &mut ReferenceState,
        data: fn(&S) -> &ShortcutData,
    ) -> Self {
        Self {
            key: key.into(),
            get,
            get_mut,
            data,
            labels: ReferenceLabels::default(),
            keys: ReferenceKeys::default(),
            compact_below: COMPACT_BELOW,
            reduced_motion: false,
        }
    }

    /// The panel's words.
    pub fn labels(mut self, labels: ReferenceLabels) -> Self {
        self.labels = labels;
        self
    }

    /// The keys the panel answers.
    pub fn keys(mut self, keys: ReferenceKeys) -> Self {
        self.keys = keys;
        self
    }

    /// The panel width below which it lays out compact ([`COMPACT_BELOW`]).
    pub fn compact_below(mut self, width: f32) -> Self {
        self.compact_below = width;
        self
    }

    /// Shows and goes at once while true: `cx.is_reduced_motion()` (C15's rule).
    pub fn reduced_motion(mut self, on: bool) -> Self {
        self.reduced_motion = on;
        self
    }

    /// The search field's key.
    pub fn search_key(&self) -> String {
        format!("{}-search", self.key)
    }

    /// Category `i`'s key: 0 is All, then each section in order.
    pub fn category_key(&self, i: usize) -> String {
        format!("{}-category-{i}", self.key)
    }

    /// The results' key: the focusable, scrolling table.
    pub fn results_key(&self) -> String {
        format!("{}-results", self.key)
    }

    /// The key of section `section`'s row `row`.
    pub fn row_key(&self, section: usize, row: usize) -> String {
        format!("{}-row-{section}-{row}", self.key)
    }

    /// Pushes the panel onto `layer` (the overlays `rune_kit::root` mounts) while open.
    pub fn build(&self, s: &S, look: &Look, layer: &mut Vec<El>) {
        let state = (self.get)(s);
        if !state.is_open() {
            return;
        }
        let get_mut = self.get_mut;
        let shell = DialogShell::new(
            self.key.clone(),
            self.labels.title.clone(),
            move |s: &mut S| get_mut(s).close(),
        )
        .open(true)
        .width(PANEL_WIDTH)
        .reduced_motion(self.reduced_motion);
        view::panel(self, s, look)
            .into_iter()
            .fold(shell, DialogShell::child)
            .build(look, layer);
    }
}

#[cfg(test)]
mod tests;
