//! IconPicker (C20d): a searchable grid of bundled icons and emoji, inline or in a popover
//! behind a button. It replaces GTK's `EmojiChooser` in Strata's Customize dialog, and can lay
//! out the dialog's 16 bundled icons too.
//!
//! Work package: C20d, `docs/work/phase-4-integration/C20d-icon-picker.md`. Strata:
//! `src/ui/browser/customization/icon_picker.rs` at lgse/strata `upstream/main` (a FlowBox of
//! 16 icon buttons, and GTK's `EmojiChooser` in a popover); GTK 4.18's
//! `gtk/gtkemojichooser.c` for the chooser's behaviour.
//!
//! ```ignore
//! let emoji = Rc::new(PickerSource::emoji(DEFAULT_VERSION));   // once
//! let st = IconPickerState::new(emoji);                         // in the host's state
//!
//! IconPicker::new("emoji", |s: &App| &s.emoji, |s: &mut App| &mut s.emoji, |s: &App| &s.look)
//!     .on_choose(|s, choice, _| s.save_icon(choice))     // + set_current, remember, set_recent
//!     .button(s, &s.look, &s.emoji_label(), Some("Choose any emoji for this item"), &mut layer)
//! // the poll: picker.poll(s, tx)
//! ```
//!
//! **Rune 2a44156: partly.** Covered: virtual lines, 2D keys and the active descendant (C1e over
//! C3's `IconsGeometry`), the grid, row, grid cell and tab roles, `El::modal` and rune-kit's
//! `Popover`, core `text_input`, and colour emoji from the system's font (CBDT and COLRv0, through
//! swash), `Tx::focus`. Missing: emoji whose default presentation is emoji draw as the outline of a
//! symbol font that covers them (anomalyco/rune#102; worked around, below); COLRv1-only fonts
//! (Fedora 44's Noto) draw nothing (anomalyco/rune#103); no way to ask whether a font draws a
//! grapheme (draft U-C20d-1); rune-kit's Popover has no Escape or first focus of its own
//! (U-C20b-1); an assistive client's action on a cell does nothing (U-C1e-8).
//!
//! # What it shows
//!
//! - **A source** ([`PickerSource`]), data the host builds once: categories and their items,
//!   each a [`Choice`] (a K3 icon by its catalog name, or an emoji by its text) with a name and
//!   keywords. [`PickerSource::catalog`] offers a K3 catalog by name (Interface and File types);
//!   [`PickerSource::emoji`] the bundled emoji (`emoji/`: Unicode emoji 18.0 and CLDR
//!   release-49's English names and keywords, 1,923 emoji, 1,898 up to Emoji 15.1, the
//!   default cap the host may raise). Hosts may pass their own, and [`PickerSource::chain`]
//!   joins two.
//! - **The grid**: a cell per item, an icon in the accent or an emoji as text, the host's
//!   current choice ([`IconPickerState::set_current`]) marked as Strata's `.active` buttons (a
//!   fifth of the accent, an accent border) and described "Current". The cursor shows only as
//!   a focus ring in keyboard mode, as GTK's chooser shows focus. Optional tooltips (Strata's
//!   16 have them).
//! - **Tabs** ([`Page`]): "Recently used" first while there are recent choices
//!   ([`IconPickerState::set_recent`], the host's list kept with [`remember`], 21 at most as
//!   GTK's), then each category; none with one category. The popover opens on Recently used
//!   when it has any, else the first category (GTK's).
//! - **Search**: GTK's rule, `matching.rs`: each query word starts a word of the name, or each
//!   starts a keyword; case and Latin accents fold; results in the source's order, across every
//!   category (never the recent tab, so nothing shows twice), the panel named "Search
//!   results", no tab selected. A tab press clears the query. No results: "No results" in the
//!   grid and the status.
//! - **The name line** under the grid: the cursor's glyph and name with the query's matches
//!   lit (W1's `MatchedText`), and the keyword it matched when the name didn't ("grinning face
//!   · happy").
//!
//! # Keys and pointer
//!
//! - **Field**: typing searches; Return chooses the grid's cursor (the first result: GTK's
//!   Enter); Down goes to the grid; Escape closes the popover at once, or
//!   inline clears the text and then goes to [`IconPicker::on_escape`] (Rune's field takes an
//!   Escape its owner leaves); a menu press goes to [`IconPicker::on_menu`] (C21's
//!   `FieldMenu`; [`IconPicker::edited`] after its edits).
//! - **Grid** (C1c's keyboard, a map of its own): arrows move (Down into a short last row
//!   lands on its last cell, Left and Right cross row ends), Home, End and the page keys;
//!   Return or Space chooses. No ranges and no type-ahead.
//! - **Tabs**: one Tab stop; arrows move and choose, wrapping. Ctrl+N and Ctrl+P (GTK's) step
//!   the tabs from anywhere in the picker.
//! - **Pointer**: a click chooses, whatever modifiers are held (C1b in single-click mode with
//!   modifiers dropped: probe P4).
//! - Tab order: field, grid, tabs. In the popover Rune's modal traps Tab and starts in the
//!   field.
//!
//! **Choosing** fires [`IconPicker::on_choose`] once; in a popover the popover then closes and
//! [`IconPicker::on_close`] runs, and focus goes back to the button. A press outside closes it
//! with no choice (reported by [`IconPicker::poll`] a frame later: Rune's modal dismissal has
//! no `Tx`), as Escape does.
//!
//! # What assistive technology hears
//!
//! A grid named by [`PickerWords::grid`] ("Search results" while searching), of rows of grid
//! cells named by their items, the cursor's cell selected and the grid's active descendant, the
//! current choice described "Current". With tabs, a tablist named "Categories" of tabs named by
//! category, the selected one selected, and the grid inside a tab panel named by it. A hidden
//! polite status says how many results a search found, or "No results". The popover is a dialog
//! named by [`PickerWords::popover`]; the button keeps the host's label and description.
//! Glyphs are hidden. Rows read "row N of the item count" (C3's U-C3-4).
//!
//! # Tokens
//!
//! Core Look and Kit tokens only, no slot file (owner ruling): a cell is `icon_size.lg` and
//! `space.md` around it and a point of border (Strata's 38 px buttons), `space.sm` between;
//! emoji at `icon_size.lg + space.xs`; hover `c.hover`, the focus ring `border_focus`, the
//! current choice `text_accent`; radii `radius.md`; the field is C18's frame.
//!
//! # Bullets
//!
//! Scripts are in `tests/scripts/c20d/` on the gallery page (`scenes/c20d.rs`: a C15 dialog
//! standing in for Strata's Customize); in-process tests in `crates/strata-gallery/tests/c20d.rs`;
//! unit tests in `tests.rs` and `emoji/tests.rs`; perf in `tests/perf.rs`.
//!
//! **Perf**: the plan's bar (§6) is `RUNE_COMMIT_BUDGET_MS=8` per UI commit, and over about
//! 2,370 choices no single commit passes it. Key plus frame at p95: a keystroke 1.17 ms, a tab
//! switch 11.32 ms, a page 9.31 ms, the last two mostly drawing new colour glyphs into the
//! atlas (`tests/perf.rs`'s own bars: 4 ms and a 60 Hz frame).
//!
//! | Bullet (`browser/folder-customization`) | Check |
//! | --- | --- |
//! | The ICON grid offers 16 bundled icons: Documents, … Tasks | **Partial**: the 16 as the host's data, by K3 name, inline: script `icon-choices`, a11y golden `c20d-icons`; the dialog is Strata's |
//! | Choose Emoji… opens an emoji picker; the picked emoji becomes the item's icon and the button reads "Emoji" followed by it | scripts `popover-opens`, `enter-chooses`, `space-chooses`, `click-chooses`, `search-enter`; the label is the host's (the page's `emoji_label`); gallery `each_choice_is_heard_once…`. The picker's own behaviour, on this bullet: `keyboard-*`, `search-*`, `no-results`, `inline-escape-*`, `category-*`, `recent-*`, `popover-outside` |
//! | Each choice updates the dialog's 56 px preview and the item's icons immediately; Done only closes the dialog | **Partial**: `on_choose` fires at once (scripts `choose-icon`, `choose-icon-ctrl`, `choose-icon-shift`, `enter-chooses`, `click-chooses`: the page's preview); the preview and Done are Strata's |
//! | With the emoji picker open, Escape closes only the picker; a second Escape closes Customize | scripts `escape-closes-picker`, `escape-twice`, `escape-from-grid` (the stand-in is C15's dialog) |
//! | Icons saved as a bundled icon name or `emoji:` followed by the emoji | **Partial**: the [`Choice`] carries the name or the emoji's text (gallery `each_choice_is_heard_once…`); writing it is Strata's |
//!
//! N/A here (Strata's dialog and storage): Opening Customize, Clear, focus back to the opener,
//! Rendering, Following file operations, the rest of Storage; the colour bullets are C20b's.
//!
//! # Decisions and deviations
//!
//! - **D1 (owner ruling): tabs as pages.** GTK's chooser is one scrolled list of sections with
//!   jump buttons; here a tab shows its category alone, and a search spans every category.
//! - **D2 (owner ruling): no skin tones.** GTK offers each emoji's tones on a right-click or
//!   long press; the bundled set has the base emoji only.
//! - **D3: emoji the font lacks still show.** GTK hides emoji Pango can't draw; Rune can't
//!   tell (U-C20d-1), so the host caps the set by Emoji version instead (15.1 by default).
//! - **D4 (owner ruling): typing in the grid doesn't search.** GTK sends a key typed anywhere
//!   to its search entry. Ruled when Rune couldn't move focus to the field; `Tx::focus` (Rune
//!   22aa8cde) now could.
//! - **D5 (owner ruling): the name line** is new against GTK.
//! - **D6 (owner ruling): Ctrl doesn't keep the popover open** after a choice (GTK's
//!   `should_close`).
//! - **D7: recents are the host's** to store. GTK kept them in its own settings, shared by
//!   every GTK app (`org.gtk.gtk4.Settings.EmojiChooser recently-used-emoji`).
//! - **D8: emoji come from the system's colour font.** None on COLRv1-only systems (#103).
//!   Emoji whose default presentation is emoji draw with U+FE0F after them, so Rune sends them
//!   to the colour font (#102): probe P1 found 23 of the 1,898 (♈ to ♓, ⛎, ⛪, ⛲, ⛺, ⛽, ⚓,
//!   ⛵, ⚡, ⛳, 🆎, 😐) drawn as Noto Sans Symbols outlines here without it, every one in colour
//!   with it. The choice keeps the emoji's own text. A host hits #102 too wherever it draws
//!   an emoji, so Strata must draw a saved emoji (the button's label, the 56 px preview, the
//!   item's icons) through [`drawn_text`], and save and compare the [`Choice`] itself.
//! - **D9 (owner ruling): no emoji in PNG goldens.** The font is the system's (CI has none,
//!   and its art moves between versions), so emoji scenes are review scenes, whose scripts end
//!   on accessibility trees.
//! - **D10: an assistive client's action on a cell does nothing** (U-C1e-8): the keyboard
//!   chooses through the grid's active descendant.
//! - **D11: C1e and C3's geometry, not C3's `IconsGrid`.** Its tiles are file tiles (116 pt at
//!   least, captions, thumbnails, rename, marquee); the picker keeps C1e's virtual lines, C3's
//!   [`crate::icons_grid::IconsGeometry`], C1c's keyboard and C1b's pointer.
//! - **D12: the recent tab is left out of search results**, so no choice shows twice. GTK 4.18
//!   filters its Recent section with the rest, so a recent match shows there and in its group.
//! - The cursor starts on the current choice where the page shows it, else on the first item;
//!   a search's results start on the first.
//! - Each picker shown at once takes its own [`IconPickerState::id_base`], so cell keys differ:
//!   ids are the base plus an item's index, so the ranges `base..base + item count` must not
//!   overlap.
//!
//! # Rune drafts (not filed)
//!
//! - **U-C20d-1: no way to ask whether a grapheme draws.** GTK's chooser hides emoji Pango
//!   can't draw (`emoji_is_supported`, from `pango_layout_get_unknown_glyphs_count`); Rune's
//!   text system has no coverage query, so a picker shows tofu for emoji newer than the font.
//!   Repro: `text("🫩")` (Emoji 16.0) with a font set lacking it (a Noto Color Emoji older
//!   than its Emoji 16.0 release) draws a missing-glyph box, and nothing in `TextSystem` says
//!   so beforehand. Ask:
//!   `TextSystem::draws(&str) -> Option<Presentation>` (colour, outline, missing). Searched
//!   upstream (coverage, missing glyph, tofu, notdef, has_glyph): nothing filed.
//! - **A comment draft for anomalyco/rune#102**: the outline route isn't DejaVu's alone. With
//!   no DejaVu installed (Noto Color Emoji 2.057 CBDT, Noto Sans Symbols and Symbols 2), 23 of
//!   the 1,898 emoji up to 15.1 draw as Noto Sans Symbols outlines (the list in D8); probe:
//!   one runtime, one emoji a frame, colour iff an image atlas page turns dirty. Every one draws
//!   in colour with U+FE0F after it, which a picker can add as it draws.
//!
//! # Probes (C20d, at 5d421fd)
//!
//! - **P1**: colour emoji headless, above (D8).
//! - **P2**: C1e's grid inside a modal Popover: the field takes first focus, Tab and Down reach
//!   the grid, its cursor moves and reveals, Return chooses, the popover closes and focus goes
//!   back to the button.
//! - **P3**: the popover inside C15's dialog: Escape closes only the popover; the next closes
//!   the dialog.
//! - **P4**: C1b in single-click mode chose on a plain click, but a Ctrl or Shift click toggled
//!   or extended a selection of several cells and chose none: the pointer drops modifiers now.

#![warn(missing_docs)]

mod data;
mod emoji;
mod grid;
mod matching;
mod recent;
mod state;
mod tabs;
mod view;
mod words;

#[cfg(test)]
mod tests;

pub use data::{Choice, PickerCategory, PickerItem, PickerSource};
pub use emoji::DEFAULT_VERSION;
pub use grid::drawn_text;
pub use recent::{RECENT_MAX, remember};
pub use state::{IconPickerState, Page};
pub use view::IconPicker;
pub use words::PickerWords;
