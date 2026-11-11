//! Settings search (C20c): a search field over an index of the settings window's rows (page,
//! section, row key, title and keywords, all as data) that filters the window in place, as
//! Strata's does, and on Enter jumps to the best row, scrolls it into view, moves the keyboard
//! to it and highlights it briefly.
//!
//! Work package: C20c, `docs/work/phase-1-leaves/C20c-tabbed-form-settings-search.md`.
//! Strata: `src/ui/settings/search.rs` and `search/{matching,filter,targets}.rs` at upstream
//! `fb7eb0c4e`; the matcher is ported rule for rule ([`matching`]'s docs).
//!
//! # Behaviour
//!
//! - **Typing** shows only the rows that match (and the section headings over them), hides the
//!   navigation entries of pages without a match, opens the best-matching page and scrolls it
//!   to its top. A query matches row titles, in English and translated, and keywords (aliases,
//!   the current language's keywords, a page's own words); see [`matching`].
//! - **No match:** the host shows [`SettingsSearch::empty`], "No settings match your search.",
//!   under the page title [`SettingsSearch::results_title`], "Search results". Clearing that query goes back to the first page.
//! - **An empty query** (or one of spaces and punctuation) filters nothing.
//! - **Escape** with text clears the query and every row comes back; the next Escape goes to
//!   the host's [`SettingsSearch::on_escape`], which closes Settings. The clear button ("Clear
//!   search") clears as Escape does.
//! - **Enter** goes to the best row: its page shows (the host picks a table or tab inside it
//!   from [`Navigate::row`]), it scrolls into view, the keyboard moves to its first control
//!   (the first in the tree: `Tx::focus` passes over a radio group's `focus_selected` one) and
//!   it is highlighted at [`FLASH_ALPHA`] of the accent, full for 0.6 s and faded by 1.5 s
//!   (static and removed at 1.5 s with animations off). With no match Enter does nothing.
//! - **Matched text:** [`SettingsSearch::title`] draws the query's matched words in a row's
//!   title in the accent (W1's `MatchedText`). A match on keywords alone marks nothing.
//! - Searching changes no preference: the search holds only its field and its matches.
//!
//! # What assistive technology hears
//!
//! A searchbox named "Search settings". While a query filters, a hidden polite status says how
//! many rows show ("4 settings") or "No settings match your search."; it is empty otherwise.
//! Hidden rows and navigation entries leave the tree. After Enter, focus lands on the row's
//! first control, which says its own name.
//!
//! # Host recipe
//!
//! ```ignore
//! let search = SettingsSearch::new("settings-search", |s: &App| &s.search,
//!         |s: &mut App| &mut s.search, |s: &App| &s.index)
//!     .page_scroll("settings-page")
//!     .on_navigate(|s: &mut App, to: Navigate, _| s.show_page(&to.page, to.row.as_deref()))
//!     .on_menu(|s: &mut App, at, tx| s.field_menu.open(s, Field::Search, at, tx))
//!     .on_escape(|s: &mut App, _| s.close_settings());
//! // Navigation: list a page while search.shows_page(s, key).
//! // The field: search.field(s, look), above the navigation.
//! // The page: search.empty(look) while s.search.no_results(), else for each section,
//! //   its heading while search.shows_section(s, key), and each row through
//! //   search.row(s, look, row_key, content) with its title from search.title(..).
//! //   Rows that follow another row's visibility (Strata's installed-actions list follows
//! //   Custom actions) ask shows_row for that row.
//! // Poll: search.tick(s, now, tx) every frame.
//! ```
//!
//! - The index is the host's: titles translated, keywords in the current language plus
//!   English, rows and pages this installation lacks left out (and left out of the view), and
//!   [`SearchIndex::revision`] bumped on any change. A row the index doesn't list always shows.
//! - The page's scroll view is keyed by [`SettingsSearch::page_scroll`]'s key, and each row is
//!   built through [`SettingsSearch::row`], which keys it for the jump's reveal and focus.
//! - A host window key handler stands down while a C21 `FieldMenu` is open over the field.
//!
//! # Deviations
//!
//! - **D1 (OWNER):** Strata's search filters in place and Enter, arrows and clicks do nothing
//!   special; there is no results list. This keeps the filter and adds Enter's jump, reveal,
//!   focus and highlight. The WP doc asks for a results list as a listbox with click-to-jump:
//!   there is none, so the accessibility goldens show the searchbox, the status and the
//!   filtered navigation instead.
//! - **D4 (OWNER):** the polite count status, as C17's and C19's; Strata announces nothing.
//! - The matched text is new against Strata, which marks nothing.
//! - Strata's icon-only navigation turns the field into a button with a popover; that layout
//!   is the host's (the field fits in a rune-kit `Popover`, and the query lives in the state).
//! - Core `text_input` in C18's frame, not rune-kit's `TextInput` (U-C21-2). Its chords
//!   follow Rune's editor: Ctrl+A selects the query.
//! - Under Rune's reduced motion with [`Look::animations`](strata_kit::look::Look) on, the
//!   highlight's `Sequence` lands its final pose at once, so it doesn't show: set the Look's
//!   animations from the same switch (Rune draft U-C20c-3).
//!
//! # Rune
//!
//! Rune 2a44156: partly. It has the searchbox and status roles, polite live regions,
//! `scroll_into_view` (which lands on a row mounted by the same commit, probe P3), `Sequence`
//! and `Tx::focus` (the jump's, resolved against the page it shows); nothing on listboxes.
//!
//! # Rune drafts (not filed)
//!
//! - **U-C20c-3, a `Sequence` has no reduced pose.** Under reduced motion (or a settle) a
//!   sequence lands its final pose at once, so a highlight that fades out never shows. Ask:
//!   `Sequence::reduced(pose, hold)`, a pose held for a time instead of the final one. Here:
//!   with the Look's animations off the highlight is a static fill the host's tick removes.

#[doc(hidden)]
pub mod fixture;
mod index;
pub mod matching;
mod state;
mod view;

#[cfg(test)]
mod tests;

pub use index::{IndexPage, IndexRow, SearchIndex};
pub use state::{FLASH_FOR, FLASH_HOLD, SettingsSearchState};
pub use view::{FLASH_ALPHA, Navigate, SearchLabels, SettingsSearch};
