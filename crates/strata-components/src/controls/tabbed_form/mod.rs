//! Tabbed form (C20c): the custom-action editor's notebook. A tablist over pages, each page
//! kept mounted or rebuilt by a flag, with a "has errors" mark per page.
//!
//! Work package: C20c, `docs/work/phase-1-leaves/C20c-tabbed-form-settings-search.md`.
//! Strata: `src/ui/settings/actions/editor.rs` (a `GtkNotebook` with General, Script and
//! Behavior) and `.settings-action-tabs` in `src/style.css`, at upstream `fb7eb0c4e`.
//!
//! # Use
//!
//! ```ignore
//! let form = TabbedForm::new("editor", |s: &App| &s.tabs, |s: &mut App| &mut s.tabs)
//!     .labels(FormLabels { tabs: "Action editor sections".into(), ..FormLabels::default() });
//! let pages = [
//!     FormPage::new("general", "General").errors(s.name_invalid),
//!     FormPage::new("script", "Script"),
//!     FormPage::new("behavior", "Behavior"),
//! ];
//! col()
//!     .child(form.build(s, look, &pages, |s, i| page_body(s, i)))
//!     .child(footer)                 // Cancel and Save
//! ```
//!
//! - **Save with an invalid field:** the host validates, sets [`FormPage::has_errors`], and
//!   calls [`TabbedFormState::show_error`] with the page and the field's key: the page shows
//!   and the keyboard moves to the field (Strata's `invalid_field`). Every edit stays in the
//!   host's state, so nothing is lost and nothing is written.
//! - **Keys:** the host's `on_key` calls [`TabbedForm::window_key`] for Ctrl+Page Up and
//!   Ctrl+Page Down, and its poll calls [`TabbedFormState::tick`] every frame, which carries a
//!   keyboard the switch dropped into the new page.
//! - The page bodies are the host's, padding and scrolling included (Strata pads 22 px and
//!   scrolls each page). With [`Mounting::Kept`] each page keeps its scroll offset.
//!
//! # What assistive technology hears
//!
//! A tablist named by [`FormLabels::tabs`]; each tab by its title, selected when chosen, with
//! its place in the set. A page with errors: its tab is described "Has errors" and reports
//! itself invalid. The chosen panel is a `tabpanel` named by its title (Rune 2a44156 has no
//! labelled-by relation to name it by its tab); hidden panels are out of the tree, and nothing
//! in them takes focus or the pointer.
//!
//! # Deviations
//!
//! - **D2 (new against Strata):** the error mark. Strata's notebook marks no tab; it switches
//!   to the invalid field's tab and shows one error line, as the host still does here.
//! - **D3 (OWNER: Strata's look on own elements):** the plan builds this from rune-kit
//!   `SegmentedControl`, which at Rune 5d421fd is a radiogroup of radios that take a text
//!   label only, so it can neither be a tablist nor carry a mark (Rune draft U-C20c-1). The
//!   tabs are our own elements with tab roles, in Strata's underline look (bullets 43, 44 of
//!   `integration/custom-actions`).
//! - **D5:** with Ctrl+Page Up or Down, a keyboard on the page being hidden is dropped by the
//!   switch and carried into the new page a frame later, from the host's poll: Rune's
//!   `Tx::focused` gives only the focused key, not whether the hidden page holds it, so the
//!   form waits to see whether the switch dropped it.
//! - **D6:** a kept page is hidden by opacity 0, `inert` and `a11y_hidden` in a shared grid
//!   cell, so it still costs layout every frame: Rune has no subtree that stays mounted out of
//!   layout (Rune draft U-C20c-2).
//! - The panels' changes land at once (`instant_changes`): Strata's notebook switches pages
//!   with no transition.
//! - A rebuilt page's body is keyed by a visit count, so each visit mounts afresh (U-C20c-4).
//!
//! # Rune
//!
//! Rune 2a44156: partly. It has the tab, tablist and tabpanel roles, arrow, Home and End
//! movement in a `focus_group` (wrapping, with `focus_follows` choosing), `inert`,
//! `instant_changes` and `Tx::focus`; it lacks a tab-style SegmentedControl and an offstage
//! subtree.
//!
//! # Rune drafts (not filed)
//!
//! - **U-C20c-1, SegmentedControl can't be a tablist.** At 5d421fd it builds a `radiogroup` of
//!   `radio` segments from text labels only. Ask: a tab mode (`tablist`, `tab` with
//!   `selected`, a `tabpanel` it names), and a per-segment trailing slot (a badge or an error
//!   dot) that the segment's accessible description can name. Here: own tab elements.
//! - **U-C20c-2, no offstage subtree.** Nothing keeps a subtree mounted (its scroll offsets,
//!   focus memory, measured rows) while taking it out of layout, hit testing, Tab order and
//!   the tree. Here: opacity 0, `inert` and `a11y_hidden` in a shared grid cell, which still
//!   lays the hidden pages out every frame. Ask: `El::offstage(bool)` (SwiftUI's hidden
//!   `TabView` pages, a web `display: none` that keeps state).
//! - **U-C20c-4, an offset survives an instant removal.** A scroll viewport removed in an
//!   `instant_changes` (or `Tx::instant`) commit, then mounted again at the same path, comes
//!   back at its old offset; removed with the default exit fade, it comes back at 0. Repro: a
//!   row with `instant_changes` holding one of two keyed scroll columns by a flag; wheel the
//!   first down, flip the flag, run 30 frames, flip it back: its content's `scrolled` is the
//!   old offset (without `instant_changes`: 0). Ask: one rule, a removed viewport forgets its
//!   offset (or an explicit keep). Here: a per-visit key.

mod state;
mod view;

#[cfg(test)]
mod tests;

pub use state::{FormPage, Mounting, TabbedFormState};
pub use view::{
    FormLabels, LABEL_REST_OPACITY, LABEL_WEIGHT, MARK, RING_ALPHA, RULE_ALPHA, STRIP_PAD_X,
    TAB_GAP, TAB_HEIGHT, TAB_PAD_X, TabbedForm, UNDERLINE,
};
