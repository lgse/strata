// SPDX-License-Identifier: MIT

use std::borrow::Cow;
use std::fmt;

use rune::Color;
use strata_kit::icons::IconName;

pub const LABEL_CHARS: usize = 22;

/// A tab, by the host's id: stable for the window's life, never reused within it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

impl fmt::Display for TabId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

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

/// A tab's icon: a semantic one, or a folder's custom icon or emoji badge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabIcon {
    /// A semantic icon from [`IconName`].
    Name(IconName),
    Catalog(String),
    /// An emoji sequence drawn inside a filled folder.
    Emoji(String),
}

impl From<IconName> for TabIcon {
    fn from(icon: IconName) -> Self {
        Self::Name(icon)
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Tab<'a> {
    pub id: TabId,
    pub name: Cow<'a, str>,
    pub icon: TabIcon,
    /// The folder's saved colour (folder customization), as the user chose it.
    pub tint: Option<Color>,
    pub accepts: Accepts,
}

impl<'a> Tab<'a> {
    /// A tab that takes files and content.
    pub fn new(id: TabId, name: impl Into<Cow<'a, str>>, icon: impl Into<TabIcon>) -> Self {
        Tab {
            id,
            name: name.into(),
            icon: icon.into(),
            tint: None,
            accepts: Accepts::ALL,
        }
    }

    /// The same tab, with `icon` (a semantic icon, a custom catalog icon, or an emoji badge).
    pub fn icon(mut self, icon: impl Into<TabIcon>) -> Self {
        self.icon = icon.into();
        self
    }

    /// The same tab, tinted with `tint` (the folder's saved colour).
    pub fn tint(mut self, tint: Option<Color>) -> Self {
        self.tint = tint;
        self
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

    pub fn step(&self, delta: isize) -> Option<TabId> {
        let len = self.tabs.len() as isize;
        let at = self.active.and_then(|id| self.index_of(id))? as isize;
        Some(self.tabs[(at + delta).rem_euclid(len) as usize].id)
    }

    pub fn nth(&self, n: usize) -> Option<TabId> {
        self.tabs.get(n).map(|t| t.id)
    }

    pub fn moved(&self, id: TabId, delta: isize) -> Option<usize> {
        let at = self.index_of(id)?;
        let to = (at as isize + delta).clamp(0, self.tabs.len() as isize - 1) as usize;
        (to != at).then_some(to)
    }

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

pub fn window_title(name: Option<&str>, format: &TitleFormat) -> String {
    match name.filter(|n| !n.is_empty()) {
        Some(name) => format!("{name}{}{}", format.separator, format.app),
        None => format.app.clone().into_owned(),
    }
}
