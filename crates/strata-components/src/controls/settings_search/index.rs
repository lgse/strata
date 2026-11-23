// SPDX-License-Identifier: MIT

//! The search index: the settings window's pages and rows, as data.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchIndex {
    pub revision: u64,
    /// In navigation order. The first is where a cleared search with no results goes back to.
    pub pages: Vec<IndexPage>,
    pub rows: Vec<IndexRow>,
}

/// A page of the settings window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexPage {
    /// Unique among pages. It is searched too, as Strata searches a page's id ("theme").
    pub key: String,
    /// Its shown, translated title, which every row on it can be found by.
    pub title: String,
    pub keywords: String,
}

impl IndexPage {
    pub fn new(key: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            keywords: String::new(),
        }
    }

    pub fn keywords(mut self, words: impl Into<String>) -> Self {
        self.keywords = words.into();
        self
    }
}

/// A row a search can find: a setting, or a member of a group such as a Keyboard shortcut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexRow {
    /// Unique among rows; keys the row's element (see [`super::SettingsSearch::row_key`]).
    pub key: String,
    /// The key of the page it is on.
    pub page: String,
    /// The section heading it sits under, shown while any row under it is.
    pub section: Option<String>,
    pub title: Option<String>,
    /// Its English title, which counts in full too. `None` when the title is English.
    pub source_title: Option<String>,
    pub keywords: String,
    pub group: Option<String>,
}

impl IndexRow {
    /// A setting titled `title`.
    pub fn new(key: impl Into<String>, page: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            page: page.into(),
            section: None,
            title: Some(title.into()),
            source_title: None,
            keywords: String::new(),
            group: None,
        }
    }

    pub fn member(
        key: impl Into<String>,
        page: impl Into<String>,
        group: impl Into<String>,
        names: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            page: page.into(),
            section: None,
            title: None,
            source_title: None,
            keywords: names.into(),
            group: Some(group.into()),
        }
    }

    pub fn section(mut self, key: impl Into<String>) -> Self {
        self.section = Some(key.into());
        self
    }

    pub fn source_title(mut self, title: impl Into<String>) -> Self {
        self.source_title = Some(title.into());
        self
    }

    /// Add words that find the row (space-separated, any case or punctuation).
    pub fn keywords(mut self, words: impl AsRef<str>) -> Self {
        if !self.keywords.is_empty() {
            self.keywords.push(' ');
        }
        self.keywords.push_str(words.as_ref());
        self
    }
}
