//! What the strip shows, as plain data the host rebuilds each view, and the pure rules on it:
//! the label's length, the window title, and the order helpers the host's key map calls.

use std::borrow::Cow;
use std::fmt;

use strata_kit::icons::IconName;

/// Characters a tab's label shows before it ends in an ellipsis (Strata's
/// `max_width_chars(22)`). The window title and the accessible name keep the whole name.
pub const LABEL_CHARS: usize = 22;

/// A tab, by the host's id: stable for the window's life, never reused within it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

impl fmt::Display for TabId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// What a tab takes when something is dropped on it: files into its folder, and content
/// (text, images, web addresses) saved there. Trash and Recent take neither; a remote folder
/// takes files only (`browser/tabs`, File drops).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Accepts {
    pub files: bool,
    pub content: bool,
}

impl Accepts {
    /// Files and content.
    pub const ALL: Accepts = Accepts {
        files: true,
        content: true,
    };
    /// Files only.
    pub const FILES: Accepts = Accepts {
        files: true,
        content: false,
    };
    /// Nothing: no drop zone at all, so a drag passes through to the strip.
    pub const NONE: Accepts = Accepts {
        files: false,
        content: false,
    };
}

/// One tab: its id, its name (the folder's, as the host names it: `~`, `/`, Trash, a share),
/// the location's icon, and what it takes from a drop.
///
/// Fields may be added: build one with [`Tab::new`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Tab<'a> {
    pub id: TabId,
    pub name: Cow<'a, str>,
    pub icon: IconName,
    pub accepts: Accepts,
}

impl<'a> Tab<'a> {
    /// A tab that takes files and content.
    pub fn new(id: TabId, name: impl Into<Cow<'a, str>>, icon: IconName) -> Self {
        Tab {
            id,
            name: name.into(),
            icon,
            accepts: Accepts::ALL,
        }
    }

    /// The same tab, taking `accepts` from drops.
    pub fn accepts(mut self, accepts: Accepts) -> Self {
        self.accepts = accepts;
        self
    }
}

/// The window's tabs in strip order, and the active one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabsData<'a> {
    pub tabs: &'a [Tab<'a>],
    pub active: Option<TabId>,
}

impl<'a> TabsData<'a> {
    pub fn new(tabs: &'a [Tab<'a>], active: Option<TabId>) -> Self {
        TabsData { tabs, active }
    }

    /// Where tab `id` is in the strip.
    pub fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// Tab `id`.
    pub fn get(&self, id: TabId) -> Option<&Tab<'a>> {
        self.tabs.iter().find(|t| t.id == id)
    }

    /// The active tab.
    pub fn active_tab(&self) -> Option<&Tab<'a>> {
        self.active.and_then(|id| self.get(id))
    }

    /// The tab `delta` places from the active one, wrapping at either end: Ctrl+Tab (1),
    /// Ctrl+Shift+Tab (-1), Ctrl+Page Down and Up. `None` with no tabs or no active tab.
    pub fn step(&self, delta: isize) -> Option<TabId> {
        let len = self.tabs.len() as isize;
        let at = self.active.and_then(|id| self.index_of(id))? as isize;
        Some(self.tabs[(at + delta).rem_euclid(len) as usize].id)
    }

    /// The `n`th tab (0 first) in strip order: Ctrl+Shift+1 is `nth(0)`, Ctrl+Shift+0
    /// `nth(9)`.
    pub fn nth(&self, n: usize) -> Option<TabId> {
        self.tabs.get(n).map(|t| t.id)
    }

    /// Where tab `id` lands moved `delta` places, stopping at either end instead of wrapping
    /// (Ctrl+Shift+Page Up and Down). `None` when it can't move that way.
    pub fn moved(&self, id: TabId, delta: isize) -> Option<usize> {
        let at = self.index_of(id)?;
        let to = (at as isize + delta).clamp(0, self.tabs.len() as isize - 1) as usize;
        (to != at).then_some(to)
    }

    /// The tab to select once `id` closes: the active one stays, and closing the active tab
    /// selects its right neighbour, or its left one when it was last. `None` when nothing
    /// is left.
    pub fn after_close(&self, id: TabId) -> Option<TabId> {
        if self.active != Some(id) {
            return self.active.filter(|&a| self.get(a).is_some());
        }
        let at = self.index_of(id)?;
        self.tabs
            .get(at + 1)
            .or_else(|| at.checked_sub(1).and_then(|i| self.tabs.get(i)))
            .map(|t| t.id)
    }
}

/// `name` shortened to [`LABEL_CHARS`] characters, the last an ellipsis, when it is longer.
pub fn elide_end(name: &str, chars: usize) -> Cow<'_, str> {
    if chars == 0 || name.chars().count() <= chars {
        return Cow::Borrowed(name);
    }
    let mut out: String = name.chars().take(chars - 1).collect();
    out.push('…');
    Cow::Owned(out)
}

/// How the window title reads: `<name><separator><app>`, or the app alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleFormat {
    pub app: Cow<'static, str>,
    pub separator: Cow<'static, str>,
}

impl Default for TitleFormat {
    /// Strata's: `docs — Strata`, with an em dash.
    fn default() -> Self {
        TitleFormat {
            app: Cow::Borrowed("Strata"),
            separator: Cow::Borrowed(" — "),
        }
    }
}

/// The window title for an active tab named `name`: the whole name, never shortened, then
/// the app's; the app's name alone with no active tab or an empty name (no folder open and
/// none awaited).
pub fn window_title(name: Option<&str>, format: &TitleFormat) -> String {
    match name.filter(|n| !n.is_empty()) {
        Some(name) => format!("{name}{}{}", format.separator, format.app),
        None => format.app.clone().into_owned(),
    }
}
