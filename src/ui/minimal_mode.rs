// SPDX-License-Identifier: MIT

//! Window-local state for minimal (Yazi-style) browsing mode.
//!
//! Filesystem state stays in `app::Browser`; this tracks only the transient
//! browse / visual / chord / prompt mode of one window. The persisted
//! preference lives in `PreferenceManager::minimal_mode` and is process-wide.

/// Shown next to every user-visible "Minimal mode" label.
pub(crate) const EXPERIMENTAL_NOTE: &str = "(experimental feature, under active development)";

/// Settings, Keybindings, and F1 heading that includes the experimental note.
pub(crate) const LABELED_TITLE: &str =
    "Minimal mode (experimental feature, under active development)";

mod goto_complete;
pub(crate) use goto_complete::{GotoCycle, cycle_goto_path, resolve_goto_input};

/// Visual selection polarity: `v` adds the walked range, `V` subtracts it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MinimalVisual {
    Select,
    Unset,
}

/// Pending multi-key prefix. `g` is places, `c` is copy path/name, `,` is sort.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MinimalChord {
    Go,
    Copy,
    Sort,
}

impl MinimalChord {
    pub(crate) fn mark(self) -> &'static str {
        match self {
            Self::Go => "g-",
            Self::Copy => "c-",
            Self::Sort => ",-",
        }
    }

    pub(crate) fn hint_title(self) -> &'static str {
        match self {
            Self::Go => "Go destinations",
            Self::Copy => "Copy options",
            Self::Sort => "Sort options",
        }
    }

    pub(crate) fn hints(self) -> &'static [(&'static [&'static str], &'static str)] {
        match self {
            Self::Go => &[
                (&["g"], "first item"),
                (&["h"], "Home"),
                (&["d"], "Downloads"),
                (&["c"], "Config"),
                (&["t"], "Trash"),
                (&["n"], "Network"),
                (&["r"], "Recent"),
                (&["k"], "Documents"),
                (&["p"], "Pictures"),
                (&["v"], "Videos"),
                (&["1–9"], "pins"),
                (&["Space"], "path"),
            ],
            Self::Copy => &[(&["c"], "path"), (&["n"], "name")],
            Self::Sort => &[
                (&["a"], "name"),
                (&["m"], "modified"),
                (&["s"], "size"),
                (&["e"], "type"),
            ],
        }
    }

    pub(crate) fn hint_note(self) -> Option<&'static str> {
        match self {
            Self::Sort => Some("shift reverses"),
            _ => None,
        }
    }
}

/// Footer prompt kind. The prompt is a short-lived `gtk::Entry`, not Vim
/// insert; `S` never enters it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MinimalPrompt {
    FindNext,
    FindPrev,
    Filter,
    Search,
    NewEntry,
    Rename,
    Goto,
    HistoryFuzzy,
    HistoryRecent,
}

impl MinimalPrompt {
    pub(crate) fn prefix(self) -> &'static str {
        match self {
            Self::FindNext => "/",
            Self::FindPrev => "?",
            Self::Filter => "filter",
            Self::Search => "search",
            Self::NewEntry => "new",
            Self::Rename => "rename",
            Self::Goto => "go ›",
            Self::HistoryFuzzy => "jump",
            Self::HistoryRecent => "recent",
        }
    }

    pub(crate) fn placeholder(self) -> &'static str {
        match self {
            Self::FindNext | Self::FindPrev => "type to jump, Enter keeps the cursor",
            Self::Filter => "filter current listing",
            Self::Search => "recursive name search in this folder",
            Self::NewEntry => "new file (append / for a folder)",
            Self::Rename => "rename focused item",
            Self::Goto => "go to path or URI",
            Self::HistoryFuzzy => "fuzzy history (empty is frecency)",
            Self::HistoryRecent => "recent folders by last visit",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MinimalMode {
    Browse,
    Visual(MinimalVisual),
    Chord(MinimalChord),
    Prompt(MinimalPrompt),
}

/// Transient per-window mode. Leaving visual mode keeps the filled selection;
/// leaving the mode entirely (preference off) also keeps it until Escape.
pub(crate) struct MinimalState {
    mode: MinimalMode,
    chord_from: MinimalMode,
    last_find: Option<(String, i32)>,
    /// True after Space, Ctrl+A, Ctrl+R, or leaving visual. Browse motions
    /// then move the cursor without rewriting the fill.
    explicit_fill: bool,
    /// Invalidates a pending post-filter fill restore when bumped.
    fill_restore: u64,
    /// Last `f` query kept while other prompts (especially `s`) reuse the pane
    /// filter entry. `None` means no committed filter to restore.
    applied_filter: Option<String>,
    /// Keyboard cursor into recursive `s` hits, independent of the fill.
    search_cursor: Option<u32>,
    /// Visual-mode anchor into recursive `s` hits.
    search_anchor: Option<u32>,
    /// Tab-complete cycle for the `g` Space path prompt.
    goto_cycle: Option<GotoCycle>,
    /// Right/`l` moved keys into the open preview. Folder motion then scrolls
    /// that pane; Left/`h` returns to the listing. Independent of GTK text
    /// focus so a source view cannot swallow `j`/`k`.
    preview_owns_keys: bool,
}

impl MinimalState {
    pub(crate) fn new() -> Self {
        Self {
            mode: MinimalMode::Browse,
            chord_from: MinimalMode::Browse,
            last_find: None,
            explicit_fill: false,
            fill_restore: 0,
            applied_filter: None,
            search_cursor: None,
            search_anchor: None,
            goto_cycle: None,
            preview_owns_keys: false,
        }
    }

    pub(crate) fn preview_owns_keys(&self) -> bool {
        self.preview_owns_keys
    }

    pub(crate) fn set_preview_owns_keys(&mut self, owned: bool) {
        self.preview_owns_keys = owned;
    }

    pub(crate) fn take_goto_cycle(&mut self) -> Option<GotoCycle> {
        self.goto_cycle.take()
    }

    pub(crate) fn set_goto_cycle(&mut self, cycle: Option<GotoCycle>) {
        self.goto_cycle = cycle;
    }

    pub(crate) fn search_cursor(&self) -> Option<u32> {
        self.search_cursor
    }

    pub(crate) fn set_search_cursor(&mut self, index: Option<u32>) {
        self.search_cursor = index;
    }

    pub(crate) fn search_anchor(&self) -> Option<u32> {
        self.search_anchor
    }

    pub(crate) fn set_search_anchor(&mut self, index: Option<u32>) {
        self.search_anchor = index;
    }

    pub(crate) fn clear_search_nav(&mut self) {
        self.search_cursor = None;
        self.search_anchor = None;
    }

    pub(crate) fn applied_filter(&self) -> Option<String> {
        self.applied_filter.clone()
    }

    pub(crate) fn remember_applied_filter(&mut self, query: String) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            self.applied_filter = None;
        } else {
            self.applied_filter = Some(trimmed.to_owned());
        }
    }

    pub(crate) fn clear_applied_filter(&mut self) {
        self.applied_filter = None;
    }

    pub(crate) fn explicit_fill(&self) -> bool {
        self.explicit_fill
    }

    pub(crate) fn set_explicit_fill(&mut self, explicit: bool) {
        self.explicit_fill = explicit;
    }

    pub(crate) fn start_fill_restore(&mut self) -> u64 {
        self.fill_restore = self.fill_restore.saturating_add(1);
        self.fill_restore
    }

    pub(crate) fn cancel_fill_restore(&mut self) {
        self.fill_restore = self.fill_restore.saturating_add(1);
    }

    pub(crate) fn fill_restore_is(&self, generation: u64) -> bool {
        self.fill_restore == generation
    }

    pub(crate) fn visual(&self) -> Option<MinimalVisual> {
        match self.mode {
            MinimalMode::Visual(kind) => Some(kind),
            _ => None,
        }
    }

    pub(crate) fn chord(&self) -> Option<MinimalChord> {
        match self.mode {
            MinimalMode::Chord(kind) => Some(kind),
            _ => None,
        }
    }

    pub(crate) fn prompt(&self) -> Option<MinimalPrompt> {
        match self.mode {
            MinimalMode::Prompt(kind) => Some(kind),
            _ => None,
        }
    }

    pub(crate) fn last_find(&self) -> Option<(String, i32)> {
        self.last_find.clone()
    }

    pub(crate) fn record_find(&mut self, query: String, direction: i32) {
        if !query.is_empty() {
            self.last_find = Some((query, direction));
        }
    }

    /// The visual mode a pending chord started from, if any.
    pub(crate) fn chord_from_visual(&self) -> Option<MinimalVisual> {
        match self.chord_from {
            MinimalMode::Visual(kind) => Some(kind),
            _ => None,
        }
    }

    pub(crate) fn enter_visual(&mut self, kind: MinimalVisual) {
        self.mode = MinimalMode::Visual(kind);
    }

    /// Leaves visual mode keeping the filled selection. No-op outside visual.
    pub(crate) fn leave_visual(&mut self) {
        if matches!(self.mode, MinimalMode::Visual(_)) {
            self.mode = MinimalMode::Browse;
            self.explicit_fill = true;
        }
    }

    pub(crate) fn enter_chord(&mut self, kind: MinimalChord) {
        self.chord_from = match self.mode {
            MinimalMode::Chord(_) => self.chord_from,
            // A prompt never survives under a chord; starting one cancels it.
            MinimalMode::Prompt(_) => MinimalMode::Browse,
            other => other,
        };
        self.mode = MinimalMode::Chord(kind);
    }

    /// Cancels a pending chord, returning to the mode it started from.
    pub(crate) fn cancel_chord(&mut self) {
        if matches!(self.mode, MinimalMode::Chord(_)) {
            self.mode = self.chord_from;
        }
    }

    /// Finishes a chord into an explicit mode (usually Browse).
    pub(crate) fn finish_chord(&mut self, visual: Option<MinimalVisual>) {
        self.mode = match visual {
            Some(kind) => MinimalMode::Visual(kind),
            None => MinimalMode::Browse,
        };
    }

    pub(crate) fn enter_prompt(&mut self, kind: MinimalPrompt) {
        self.goto_cycle = None;
        self.mode = MinimalMode::Prompt(kind);
    }

    pub(crate) fn leave_prompt(&mut self) {
        if matches!(self.mode, MinimalMode::Prompt(_)) {
            self.goto_cycle = None;
            self.mode = MinimalMode::Browse;
        }
    }

    /// Drops all transient state for teardown when the preference turns off,
    /// keeping the filled selection and the last `/` query.
    pub(crate) fn reset(&mut self) {
        self.mode = MinimalMode::Browse;
        self.chord_from = MinimalMode::Browse;
        self.applied_filter = None;
        self.goto_cycle = None;
        self.preview_owns_keys = false;
        self.clear_search_nav();
        self.cancel_fill_restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_cancel_returns_to_its_origin_keeping_the_fill() {
        let mut state = MinimalState::new();
        assert_eq!(state.visual(), None);
        assert_eq!(state.chord(), None);
        state.enter_visual(MinimalVisual::Select);
        state.enter_chord(MinimalChord::Go);
        assert_eq!(state.chord(), Some(MinimalChord::Go));
        assert_eq!(state.chord_from_visual(), Some(MinimalVisual::Select));
        state.cancel_chord();
        assert_eq!(state.visual(), Some(MinimalVisual::Select));
        state.enter_chord(MinimalChord::Copy);
        state.finish_chord(None);
        assert_eq!(state.chord(), None);
        assert_eq!(state.visual(), None);
        state.enter_visual(MinimalVisual::Unset);
        state.set_preview_owns_keys(true);
        state.reset();
        assert_eq!(state.chord(), None);
        assert_eq!(state.visual(), None);
        assert_eq!(state.applied_filter(), None);
        assert!(!state.preview_owns_keys());
    }

    #[test]
    fn prompt_tracks_kind_and_last_find() {
        let mut state = MinimalState::new();
        assert_eq!(state.prompt(), None);
        state.enter_prompt(MinimalPrompt::FindNext);
        assert_eq!(state.prompt(), Some(MinimalPrompt::FindNext));
        state.record_find("read".to_owned(), 1);
        state.leave_prompt();
        assert_eq!(state.prompt(), None);
        assert_eq!(state.last_find(), Some(("read".to_owned(), 1)));
        state.reset();
        assert_eq!(state.prompt(), None);
        // Reset keeps the repeat query across teardown.
        assert_eq!(state.last_find(), Some(("read".to_owned(), 1)));
    }
}
