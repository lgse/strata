//! The palette's modes, as data: Ctrl+K search and Ctrl+Shift+K folder jump are two values
//! of one type, not two components.

use strata_kit::icons::IconName;

/// What a mode's results are.
///
/// Used by: hosts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ResultKind {
    /// Files and folders: each result's own icon, and its thumbnail if it has one.
    #[default]
    Files,
    /// Folders only: a folder icon unless a result names another, and no thumbnails.
    Folders,
}

/// The status an empty query shows in a mode whose empty query lists nothing.
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdleText {
    /// On open, before anything is typed.
    pub first: String,
    /// After a typed query is cleared again.
    pub cleared: String,
}

/// One key hint in the footer: an optional icon and its words, such as "↑↓  navigate".
/// The host writes the keys its key map binds.
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    pub icon: Option<IconName>,
    pub text: String,
}

impl Hint {
    /// A hint of `text` alone.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            icon: None,
            text: text.into(),
        }
    }

    /// A hint of `text` beside `icon`.
    pub fn icon(icon: IconName, text: impl Into<String>) -> Self {
        Self {
            icon: Some(icon),
            text: text.into(),
        }
    }
}

/// A mode: what the palette is called, what its field says, its results' kind, and the words
/// of each state. [`PaletteMode::search`] and [`PaletteMode::jump`] are Strata's two, in its
/// English; a translated host fills in its own strings.
///
/// Used by: hosts.
#[derive(Clone, Debug)]
pub struct PaletteMode {
    pub kind: ResultKind,
    /// The dialog's accessible name.
    pub title: String,
    /// The field's accessible name.
    pub field_label: String,
    /// The field's placeholder (Strata uses the same words as the name).
    pub placeholder: String,
    /// The field's accessible description: the searched locations.
    pub description: Option<String>,
    /// The result list's accessible name.
    pub list_label: String,
    /// `Some` when an empty query lists nothing and shows this instead (search); `None` when
    /// an empty query is a query like any other (jump lists its history).
    pub idle: Option<IdleText>,
    /// While a query has nothing to show yet.
    pub searching: String,
    /// A finished query that found nothing.
    pub no_results: String,
    /// A finished empty query that found nothing (jump: no history yet).
    pub no_results_empty: String,
    /// The spinner's accessible name.
    pub busy: String,
    /// The footer's partial-results notice.
    pub partial: String,
    /// The footer's key hints, in order.
    pub hints: Vec<Hint>,
    /// What assistive technology hears for `n` results (a hidden polite live region).
    pub count: fn(usize) -> String,
}

fn english_count(n: usize) -> String {
    if n == 1 {
        "1 result".to_owned()
    } else {
        format!("{n} results")
    }
}

/// Strata's footer hints, with its default keys.
fn hints() -> Vec<Hint> {
    vec![
        Hint::text("↑↓  navigate"),
        // Strata's corner-down-left beside "open", which is all assistive technology hears.
        Hint::icon(IconName::Enter, "open"),
        Hint::icon(IconName::FolderOpen, "Alt+Enter  open containing folder"),
    ]
}

impl PaletteMode {
    /// Ctrl+K: global file search over `locations`, which the field's description lists
    /// (Strata's `reset_for_search`).
    pub fn search<L: AsRef<str>>(locations: &[L]) -> Self {
        let list = locations
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>()
            .join("\n");
        Self {
            kind: ResultKind::Files,
            title: "Search files and folders".to_owned(),
            field_label: "Search files and folders…".to_owned(),
            placeholder: "Search files and folders…".to_owned(),
            description: Some(format!(
                "Search locations:\n{list}\nRemote shares are not included."
            )),
            list_label: "Search results".to_owned(),
            idle: Some(IdleText {
                first: "Type to search Home and mounted local drives".to_owned(),
                cleared: "Type to search Home and mounted local drives\nFuzzy matching · try a name or path fragment".to_owned(),
            }),
            searching: "Searching…".to_owned(),
            no_results: "No matching files or folders".to_owned(),
            no_results_empty: "No matching files or folders".to_owned(),
            busy: "Indexing files…".to_owned(),
            partial: "Partial results".to_owned(),
            hints: hints(),
            count: english_count,
        }
    }

    /// Ctrl+Shift+K: jump to a folder visited before (Strata's `show_history`).
    pub fn jump() -> Self {
        Self {
            kind: ResultKind::Folders,
            title: "Jump to a folder".to_owned(),
            field_label: "Jump to a folder…".to_owned(),
            placeholder: "Jump to a folder…".to_owned(),
            description: Some("Folders previously visited in Strata".to_owned()),
            list_label: "Visited folders".to_owned(),
            idle: None,
            searching: "Searching…".to_owned(),
            no_results: "No matching folders".to_owned(),
            no_results_empty: "No folder history yet".to_owned(),
            busy: "Searching…".to_owned(),
            partial: "Partial results".to_owned(),
            hints: hints(),
            count: english_count,
        }
    }
}
