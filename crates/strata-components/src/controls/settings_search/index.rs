//! The search index: the settings window's pages and rows, as data.

/// Every page and row a settings search can find. The host builds it (titles translated,
/// keywords in the current language plus English), leaves out rows and pages this
/// installation doesn't have, and bumps [`SearchIndex::revision`] whenever it changes (a
/// language switch, an action renamed, a key rebound). A row on a page it leaves out is never
/// found, nor is a group member on one or of a group on one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchIndex {
    pub revision: u64,
    /// In navigation order. The first is where a cleared search with no results goes back to.
    pub pages: Vec<IndexPage>,
    /// In page order. A tie between two rows goes to the one listed first, so list settings
    /// before group members (Strata lists its settings before the Keyboard rows).
    pub rows: Vec<IndexRow>,
}

/// A page of the settings window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexPage {
    /// Unique among pages. It is searched too, as Strata searches a page's id ("theme").
    pub key: String,
    /// Its shown, translated title, which every row on it can be found by.
    pub title: String,
    /// More words every row on the page can be found by: Strata adds "settings" and its
    /// translation.
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
    /// Its shown, translated title, which counts in full and gets the matched text. A row
    /// without one (a Keyboard row, found by its action's names) is found by its keywords only.
    pub title: Option<String>,
    /// Its English title, which counts in full too. `None` when the title is English.
    pub source_title: Option<String>,
    /// Words that also find it, at half a title word's weight: aliases, the current language's
    /// keywords, an action's names, a shortcut's key labels ("Ctrl+R").
    pub keywords: String,
    /// The key of a row this one belongs to (Strata's Keyboard shortcuts). A query that finds
    /// the group row shows every member, and a member that matches also shows the group row
    /// (so a table switch on it stays up) and counts for the group's page.
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

    /// A member of `group`, found by `names` (an action's title and the names of what runs
    /// it) and its keywords only.
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
