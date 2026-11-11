//! What the host hands the location bar: the crumbs of the folder it shows, the suggestions it
//! listed, and the labels it reads out. Plain data, rebuilt by the host: no I/O.

use std::borrow::Cow;
use std::ops::Range;

use strata_kit::icons::IconName;

/// The longest crumb label drawn whole, in characters. A longer one is shortened in the middle
/// (Strata's `BREADCRUMB_LABEL_MAX_CHARS`).
pub const CRUMB_CHARS: usize = 32;

/// One folder on the path: an ancestor, or the current folder when it is the last.
///
/// Fields may be added: build one with [`Crumb::new`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Crumb {
    /// The host's identity for the folder (a URI, say). Intents carry it.
    pub id: String,
    /// What the crumb shows: `~`, `/`, `Documents`. Drawn shortened in the middle past
    /// [`CRUMB_CHARS`]; assistive technology hears it whole.
    pub label: String,
    /// The folder's full path as the host displays it: the crumb's accessible description.
    pub description: String,
    /// The folder's icon in the hierarchy menu.
    pub icon: IconName,
    /// An icon drawn before the label (Strata's lock on an administrator root).
    pub badge: Option<IconName>,
    /// No separator follows the crumb: the `/` root, which reads as the separator itself.
    pub joins_next: bool,
}

impl Crumb {
    /// The folder `id`, shown as `label`, described by its label, with a folder icon.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            id: id.into(),
            description: label.clone(),
            label,
            icon: IconName::Folder,
            badge: None,
            joins_next: false,
        }
    }

    /// The same crumb, described by `path`.
    pub fn description(mut self, path: impl Into<String>) -> Self {
        self.description = path.into();
        self
    }

    /// The same crumb, with `icon` in the hierarchy menu.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = icon;
        self
    }

    /// The same crumb, with `badge` before its label.
    pub fn badge(mut self, badge: Option<IconName>) -> Self {
        self.badge = badge;
        self
    }

    /// The same crumb, followed by no separator or by one.
    pub fn joins_next(mut self, joins: bool) -> Self {
        self.joins_next = joins;
        self
    }
}

/// The folder the bar names: its crumbs, root first and the current folder last, and the text
/// the entry opens with (the folder's path as the host displays it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trail {
    pub crumbs: Vec<Crumb>,
    pub text: String,
}

impl Trail {
    pub fn new(crumbs: Vec<Crumb>, text: impl Into<String>) -> Self {
        Self {
            crumbs,
            text: text.into(),
        }
    }

    /// The current folder's crumb, if any.
    pub fn current(&self) -> Option<&Crumb> {
        self.crumbs.last()
    }
}

/// One suggestion, as the host listed it.
///
/// Fields may be added: build one with [`Candidate::new`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Candidate {
    /// What the row shows: `Documents/`.
    pub label: String,
    /// The byte ranges of `label` the typed text matched, drawn in the accent
    /// ([`crate::contract::MatchedText`]).
    pub matched: Vec<Range<usize>>,
    /// The entry's text once the suggestion is taken: `~/Documents/`.
    pub replacement: String,
    /// Its parent folder, as the host displays it (`~`): drawn after the label, and the row's
    /// accessible description.
    pub hint: String,
    /// Its icon.
    pub icon: IconName,
}

impl Candidate {
    /// A folder suggestion shown as `label`, completing to `replacement`.
    pub fn new(label: impl Into<String>, replacement: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            matched: Vec::new(),
            replacement: replacement.into(),
            hint: String::new(),
            icon: IconName::Folder,
        }
    }

    /// The same suggestion, with `ranges` of its label matched.
    pub fn matched(mut self, ranges: impl IntoIterator<Item = Range<usize>>) -> Self {
        self.matched = ranges.into_iter().collect();
        self
    }

    /// The same suggestion, with its parent folder `hint`.
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = hint.into();
        self
    }

    /// The same suggestion, with `icon`.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = icon;
        self
    }
}

/// What the bar asks suggestions for: the entry's text, and the generation its answer must
/// carry back. Each text change starts a new generation; an answer for any other is stale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionQuery {
    pub generation: u64,
    pub text: String,
}

/// The host's answer to a [`CompletionQuery`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completions {
    pub generation: u64,
    pub candidates: Vec<Candidate>,
}

impl Completions {
    pub fn new(generation: u64, candidates: Vec<Candidate>) -> Self {
        Self {
            generation,
            candidates,
        }
    }
}

/// What assistive technology hears and the buttons' tooltips say. The host follows its key map
/// (the shortcut in each name) and its language. The defaults are Strata's English names with
/// its default keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocationLabels {
    /// The bar: a navigation landmark.
    pub bar: String,
    /// The crumbs' list.
    pub crumbs: String,
    /// The entry and the combobox around it.
    pub entry: String,
    /// What the empty entry shows.
    pub placeholder: String,
    /// The entry's buttons.
    pub navigate: String,
    pub cancel: String,
    /// The current crumb's copy button, at rest and for two seconds after a copy.
    pub copy: String,
    pub copied: String,
    /// The suggestion list.
    pub suggestions: String,
}

impl Default for LocationLabels {
    fn default() -> Self {
        Self {
            bar: "Location".into(),
            crumbs: "Path".into(),
            entry: "Location (Ctrl+L)".into(),
            placeholder: "Enter a path or URI…".into(),
            navigate: "Navigate (Enter)".into(),
            cancel: "Cancel (Escape)".into(),
            copy: "Copy path".into(),
            copied: "Path copied".into(),
            suggestions: "Suggestions".into(),
        }
    }
}

/// `label`, shortened in the middle to `max` characters (counting the ellipsis) when longer:
/// the head keeps one character fewer than the tail when they can't split evenly.
pub fn elide_middle(label: &str, max: usize) -> Cow<'_, str> {
    let count = label.chars().count();
    if count <= max || max < 3 {
        return Cow::Borrowed(label);
    }
    let keep = max - 1;
    let head = keep / 2;
    let tail = keep - head;
    let start: String = label.chars().take(head).collect();
    let end: String = label.chars().skip(count - tail).collect();
    Cow::Owned(format!("{start}…{end}"))
}
