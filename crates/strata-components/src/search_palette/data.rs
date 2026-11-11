//! What the host hands the palette: results, the queries it answers, and its answers.

use std::ops::Range;

use strata_kit::icons::IconName;

use crate::contract::ThumbKey;

/// One result: a file or folder the host found.
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PaletteItem {
    /// The host's id for it (its path). [`super::PaletteIntent::Open`] and
    /// [`super::PaletteIntent::Reveal`] name it, the highlight follows it from one answer to
    /// the next, and its row is built again only when it changes (rows are keyed by place,
    /// [`super::option_key`]).
    pub id: String,
    /// The first line: the name.
    pub label: String,
    /// Byte ranges of `label` the query matched, drawn in the accent.
    pub matched: Vec<Range<usize>>,
    /// The second line: the full path, shortened in the middle when it doesn't fit. It is
    /// also the row's accessible name, as in Strata.
    pub detail: String,
    /// Byte ranges of `detail` the query matched.
    pub detail_matched: Vec<Range<usize>>,
    /// The row's icon, drawn under the thumbnail while one loads.
    pub icon: IconName,
    /// The thumbnail key, for images: drawn from the palette's [`ThumbnailStore`] when the
    /// palette has one ([`super::SearchPalette::thumbnails`]).
    ///
    /// [`ThumbnailStore`]: crate::thumbnails::ThumbnailStore
    pub thumb: Option<ThumbKey>,
}

impl PaletteItem {
    /// A result `id` showing `label` over `detail`, with a file icon.
    pub fn new(id: impl Into<String>, label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            matched: Vec::new(),
            detail: detail.into(),
            detail_matched: Vec::new(),
            icon: IconName::File,
            thumb: None,
        }
    }

    /// The same result with `ranges` of its label matched.
    pub fn matched(mut self, ranges: impl IntoIterator<Item = Range<usize>>) -> Self {
        self.matched = ranges.into_iter().collect();
        self
    }

    /// The same result with `ranges` of its detail matched.
    pub fn detail_matched(mut self, ranges: impl IntoIterator<Item = Range<usize>>) -> Self {
        self.detail_matched = ranges.into_iter().collect();
        self
    }

    /// The same result with `icon`.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = icon;
        self
    }

    /// The same result with thumbnail key `key`.
    pub fn thumb(mut self, key: ThumbKey) -> Self {
        self.thumb = Some(key);
        self
    }
}

/// A search the palette asks for ([`super::PaletteIntent::Query`]): on every open and every
/// edit, an empty query included.
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteQuery {
    /// Answer with this generation; an answer for any other is dropped.
    pub generation: u64,
    /// The field's text when it asked, untrimmed.
    pub text: String,
}

/// An answer to a [`PaletteQuery`]: one batch. A search that streams sends several with the
/// same generation, each holding every result so far, the last one [`Answer::done`]. More
/// answers for the same generation after that (a rescan for outside changes) are taken as
/// updates of the same query.
///
/// The batch shape matches C18's footer-prompt candidates (`generation`, every item so far,
/// `done`).
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    /// The query's generation.
    pub generation: u64,
    /// Every result so far, best first: each batch replaces the last.
    pub items: Vec<PaletteItem>,
    /// The search finished: the spinner goes, and an empty answer reads as no results
    /// instead of "Searching…".
    pub done: bool,
    /// Why the results are partial (the walk stopped early), if they are: the footer shows
    /// "Partial results" with this as its description and tooltip.
    pub partial: Option<String>,
}

impl Answer {
    /// A batch of `items` for `generation`, with more to come.
    pub fn new(generation: u64, items: Vec<PaletteItem>) -> Self {
        Self {
            generation,
            items,
            done: false,
            partial: None,
        }
    }

    /// The same batch, the last for its query.
    pub fn done(mut self) -> Self {
        self.done = true;
        self
    }

    /// The same batch, partial for `causes`.
    pub fn partial(mut self, causes: impl Into<String>) -> Self {
        self.partial = Some(causes.into());
        self
    }
}
