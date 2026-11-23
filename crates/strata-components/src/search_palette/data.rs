// SPDX-License-Identifier: MIT

//! What the host hands the palette: results, the queries it answers, and its answers.

use std::ops::Range;

use strata_kit::icons::IconName;

use crate::contract::ThumbKey;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PaletteItem {
    pub id: String,
    /// The first line: the name.
    pub label: String,
    /// Byte ranges of `label` the query matched, drawn in the accent.
    pub matched: Vec<Range<usize>>,
    pub detail: String,
    /// Byte ranges of `detail` the query matched.
    pub detail_matched: Vec<Range<usize>>,
    /// The row's icon, drawn under the thumbnail while one loads.
    pub icon: IconName,
    pub icon_key: Option<String>,
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
            icon_key: None,
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

    /// The same result with catalog icon key `key`.
    pub fn icon_key(mut self, key: impl Into<String>) -> Self {
        self.icon_key = Some(key.into());
        self
    }

    /// The same result with thumbnail key `key`.
    pub fn thumb(mut self, key: ThumbKey) -> Self {
        self.thumb = Some(key);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteQuery {
    /// Answer with this generation; an answer for any other is dropped.
    pub generation: u64,
    /// The field's text when it asked, untrimmed.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    /// The query's generation.
    pub generation: u64,
    /// Every result so far, best first: each batch replaces the last.
    pub items: Vec<PaletteItem>,
    pub done: bool,
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
