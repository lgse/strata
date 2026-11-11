//! SearchPalette (C17): one modal palette for Ctrl+K search and the Ctrl+Shift+K folder jump
//! (`browser/search`, `browser/navigation/folder-jump`), its modes as data.
//!
//! Work package: C17, `docs/work/phase-2-composites/C17-search-palette.md`. Behaviour follows
//! the feature map, against lgse/strata `upstream/main` (`src/ui/search.rs`,
//! `src/ui/search/panel.rs`, `src/ui/search/results.rs`, `src/ui/window/composition/search.rs`
//! and `.search-*` in `src/style.css`).
//!
//! **Rune 2a44156: partly.** C15's modal shell (`El::modal`, `backdrop_blur`, presence
//! motion), core `text_input` (`on_input_key`, `on_secondary_press`, `active_descendant`),
//! `virtual_rows` with `on_visible_rows` (and the set size and positions Rune gives virtual
//! rows), `Tx::scroll_to_y` and `Tx::scroll_into_view_in`, `live_region`, `truncate_middle`
//! on rich text, `Tx::wake_after` and `rune::Latest` cover it. Missing: the combobox role on an
//! input (U-C6-1), virtual rows that show content added above at offset 0 (U-C17-3) and
//! `reveal_selected` on rows built by index (U-C17-4), both worked around with scroll requests,
//! and input-method events in scripts (U-C6-7). `Tx::focus` and `El::modal_tx` (22aa8cde)
//! cover the focus an open hands over and the dismissal's intent.
//!
//! # Hosting
//!
//! ```ignore
//! // The view: the palette goes onto the overlay layer `rune_kit::root` mounts.
//! SearchPalette::new("palette", |s: &App| &s.palette, |s: &mut App| &mut s.palette)
//!     .new_tab_keys(keymap.chords("open_in_new_tab"))         // default Ctrl+Enter
//!     .reveal_keys(keymap.chords("open_containing_folder"))   // default Alt+Enter
//!     .thumbnails(|s: &App| &s.thumbs, Step::for_tile(look, PALETTE_THUMB, scale))
//!     .on_visible(|s: &mut App, rows| s.sync_palette_thumbs(rows))   // a C25 viewer
//!     .return_focus_on_open("files")                          // the revealed item's view
//!     .reduced_motion(cx.is_reduced_motion())
//!     .on_intent(|s: &mut App, intent, tx| s.palette_intent(intent, tx))
//!     .build(&look, s, &mut layer);
//! // Ctrl+K:        route(s.palette.toggle(PaletteMode::search(&roots)))
//! //                (no roots: s.palette.unavailable("No local search locations available."))
//! // Ctrl+Shift+K:  route(s.palette.toggle(PaletteMode::jump()))
//! // PaletteIntent::Query(q): search; each batch -> s.palette.deliver(Answer::new(q.generation,
//! //                every result so far), tx), the last `.done()`, `.partial(causes)` if it
//! //                stopped early. The `Tx` scrolls the list in the commit that draws the batch.
//! //                One-shot answers can come through `rune::Latest`; a walk streams over a channel.
//! // Open { id, new_tab, .. } / Reveal { id }: navigate, preview, reveal (Strata's policy).
//! // ResultMenu { id, at }: C21's menu with "Open containing folder" -> s.palette.reveal(&id, tx).
//! // FieldMenu { at }: C21's field menu.
//! // Each poll: s.palette.tick().
//! ```
//!
//! The palette owns no I/O and no ranking: the host searches, caps (Strata's 100) and orders
//! the results, and gives each its matched byte ranges, icon and thumbnail key.
//!
//! # Behaviour
//!
//! - **Modes as data.** [`PaletteMode`] holds the title, the field's name, placeholder and
//!   description, the result kind, every state's words and the footer's hints;
//!   [`PaletteMode::search`] and [`PaletteMode::jump`] are Strata's two. One component, not two.
//! - **Queries and answers.** Every open and every edit asks for a [`PaletteQuery`] with a new
//!   generation; an [`Answer`] for any other generation is dropped, so a superseded query's
//!   results never show. Each batch holds every result so far and replaces the last. The shown
//!   results stay until the current query's first answer lands (Strata); an empty search query
//!   clears them at once and shows the idle words.
//! - **The highlight.** A changed (trimmed) query highlights its first result. A same-query
//!   batch keeps the highlight on its result by id, else at its place, clamped (Strata's
//!   `restore_selection`).
//! - **Scrolling as answers land** ([`PaletteState::deliver`], Strata's `restore_scroll` with its
//!   `navigation_started`):
//!   - A new query's answer scrolls the list to the top, where its highlighted first result is.
//!   - A batch keeps the list's pixel offset, so better results it puts above the rows in view
//!     show, at offset 0 too. Rows are keyed by place ([`option_key`]): Rune's virtual rows
//!     keep the row whose key was first in view where it is (U-C17-3), and with keys by place
//!     that row's place doesn't move, so the offset stays without any scroll request. Nothing
//!     the palette asks for can then undo a wheel or stop a fling that lands in the same frame.
//!   - After the user moved the highlight in this query, any batch that changes the results
//!     reveals the highlighted row (Strata's `scroll_row_into_view`), only as far as it takes:
//!     a row still in view moves nothing, so rows added above stay visible with it. A list the
//!     user is still scrolling is left alone, as Strata's `interaction_revision` leaves it.
//!     *Still scrolling* means exactly: the offset the list last reported differs from the one
//!     at the previous [`PaletteState::tick`], and the palette hasn't asked for a scroll (an
//!     arrow's reveal or a batch's reveal) since the list was last at rest
//!     (two ticks with the same offset). So a fling or a wheel still landing counts; the
//!     palette's own scroll, gliding over frames or not, never does (review round 3, B2). A
//!     wheel that starts while the palette's own glide is still running counts as the
//!     palette's until the list comes to rest.
//!   - An arrow reveals its row from the key handler, only as far as it takes, and wins over a
//!     batch in its frame.
//!   - The palette's scrolls are Rune's reveals (`Tx::scroll_into_view_in`), which a wheel or a
//!     fling wins over. One case still loses: after the user moved the highlight, a batch
//!     handed over in the very frame a wheel arrives, before Rune has applied the wheel,
//!     reveals the highlighted row over it (Strata does too).
//! - **States** ([`Phase`]): idle, loading ("Searching…"), results, no results (the empty-query
//!   words for the jump's history), error ([`PaletteState::fail`]) and unavailable
//!   ([`PaletteState::unavailable`]: the field takes neither text nor focus, the panel keeps the
//!   keyboard, Escape still closes). The spinner shows while the current query isn't done;
//!   "Partial results" while the shown answer says why it is partial, its causes as its
//!   description and tooltip.
//! - **Keys, in the field.** Up and Down move the highlight and stop at the ends (Strata), the
//!   caret staying put; Enter and Shift+Enter open it; the new-tab chords (Shift held too) open
//!   it in a new tab; the reveal chords (Super held too) open its folder; Escape closes. Home,
//!   End and the paging keys are the editor's. Rune gives Return and Escape to an input-method
//!   composition first. Ctrl+K and Ctrl+Shift+K pass through to the host's key map (probe P1).
//! - **The pointer.** A click opens a result, a middle click opens it in a new tab, a
//!   right-click highlights it and asks for the host's menu. Rows keep the field's keyboard and
//!   tint on hover; the pointer never moves the highlight. A press acts only when it is released
//!   on the result it was pressed on, as GTK's ListBox activates only when press and release
//!   land on the same row: a press held while a batch moves another result into its row opens
//!   nothing and asks for no menu (review round 3, B1). Each row (keyed by place) holds an inner
//!   box keyed by its result that takes the presses; Rune resolves a release by the pressed
//!   element's path and drops it when that path is gone, which covers the right button too
//!   (Rune has no press-down hook for it at the pin). The inner box is `role("none")`, so
//!   assistive technology hears the option alone, whose own click and menu actions act on the
//!   result it shows.
//! - **Closing and focus.** C15's shell: Escape, a click outside and the host's toggle close
//!   it, and Rune's modal gives focus back to the opener. Opening a result or its folder
//!   closes it at once and focuses [`SearchPalette::return_focus_on_open`] with `Tx::focus`, so
//!   focus goes to the revealed item's view instead: focus the closing change puts outside a
//!   modal stays there (`runtime/input/modal_focus.rs`).
//!
//! # What assistive technology hears
//!
//! - The dialog, named by the mode's title ("Search files and folders", "Jump to a folder").
//! - A `combobox` group around the field (Rune makes every input a text box, U-C6-1), expanded
//!   while results show; the field named by the mode, with its placeholder, its description
//!   (the searched locations) and the highlighted result as its active descendant.
//! - The spinner: a progress indicator named "Indexing files…".
//! - The results: a listbox ("Search results", "Visited folders") of options named by full
//!   path (Strata's row name), the highlighted one selected; Rune gives virtual rows their set
//!   size and positions (probe P3).
//! - The status (`{key}-status`): a polite live region with the state's words, or, while results
//!   show, a hidden one with their count (probe P5), set at the query's first results and again
//!   when it is done, not on every batch (owner ruling).
//! - "Partial results": a note with its causes as its description. The hints are labels; their
//!   icons are hidden.
//!
//! # Tokens
//!
//! Kit and strata-kit only: `look.row.palette` and `density::PALETTE_*` for the rows, Kit
//! colours (`icon_accent`, `text_base`, `text_weak`, `text_muted`, `text_accent`, `hover`,
//! `border_base`, `border_weak`, `border_focus`, `surface_weak`, rune-kit's `TableStyle`
//! selected fill), the Kit's `ui`, `ui_compact` (at `ui_medium`'s weight) and `small` type, and
//! C24's app scroll bar. The panel's sizes are the constants below, from Strata's CSS; a
//! `palette` slot in strata-kit is a follow-up (owner ruling).
//!
//! # Coverage
//!
//! Scripts are `tests/scripts/c17/<name>.json`; "test" is in `crates/strata-gallery/tests/c17.rs`;
//! "unit" is in `tests.rs`; "golden" is a scene's a11y or PNG golden. The gallery page stands in
//! for Strata's window, index, history and result menu.
//!
//! `browser/search`:
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | Ctrl+K or the header search button opens the search dialog…; Ctrl+K again closes it | scripts `ctrl-k-toggles`, `search-button-opens` (the page binds Ctrl+K; the blur is C15's) |
//! | The search field's accessible description lists the searched locations… | script `search-after-jump`; golden `c17-idle` (a11y: the description); which locations is Strata's |
//! | Opened with Ctrl+K, the search field's accessible name and placeholder… | scripts `search-button-opens`, `search-after-jump`; golden `c17-idle` |
//! | On open, the empty query shows "Type to search…"; clearing a typed query adds… | scripts `search-button-opens`, `cleared-status`; unit `an_empty_search_query_is_idle_with_the_first_words_then_the_cleared_ones` |
//! | The status text sits centered… | golden `c17-idle`, `c17-no-results` (PNG) |
//! | Each result shows its name, its full path, and a thumbnail for images | **partial**: goldens `c17-results`, `c17-partial`; test `image_results_get_their_thumbnails_in_a_resting_window` (the gallery's fake images; rendering is Strata's) |
//! | A spinner shows in the search bar while indexing runs | scripts `spinner`, `spinner-ends`; test `streamed_batches_land_in_a_resting_window` |
//! | The footer shows key hints for navigate, open, and Alt+Enter "open containing folder"… | golden `c17-idle` and the rest (the hints are the mode's data, so the host's key map) |
//! | When indexing stops early, the footer shows "Partial results"; its description… | script `partial-results`; golden `c17-partial` (a11y: the description) |
//! | Hovering Partial results shows a tooltip…; complete results clear the tooltip | test `the_partial_notice_shows_its_causes_on_hover_until_results_are_complete` |
//! | While indexing adds results, the highlighted result stays selected by path and unchanged rows are not rebuilt | script `highlight-by-path`; unit `a_same_query_batch_keeps_the_highlight_on_its_result`, `rows_build_again_only_when_their_result_or_highlight_changes` |
//! | (the same bullet) the list's scroll as batches land | tests `a_batch_above_the_first_row_shows_at_the_top_until_the_user_moves`, `the_streamed_best_result_shows_in_every_frame`, `a_moved_highlight_stays_in_view_when_a_batch_pushes_it_down` (every frame of a resting window); review round 2: `a_batch_in_a_wheel_s_frame_keeps_the_wheel_s_scroll`, `an_arrow_in_a_batch_s_frame_reveals_its_row`, `a_new_query_s_answers_in_one_poll_scroll_to_the_top`, `a_batch_mid_fling_leaves_the_fling_alone`, `a_batch_keeps_a_wheeled_offset_until_the_user_moves`, `after_moving_any_changed_batch_reveals_the_highlighted_row`, `after_moving_rows_added_above_show_while_the_highlight_stays_in_view`, `after_moving_a_batch_during_a_wheel_keeps_the_wheel_s_scroll`; review round 3: `a_batch_right_after_an_arrow_scroll_keeps_the_highlight_in_view`, `back_to_back_batches_keep_a_moved_highlight_in_view`, `an_arrow_scrolls_only_as_far_as_it_takes`; unit `answers_scroll_as_strata_restores_them` |
//! | Up and Down move the highlight while the caret stays in the query field… | scripts `arrows`, `arrows-second`; tests `up_and_down_leave_the_caret_where_it_was`, `arrows_keep_the_highlighted_row_in_view`, `arrows_while_composing_leave_the_highlight`, `a_resting_pointer_never_moves_the_highlight` |
//! | When the first result is preselected, one Down highlights the second result | script `arrows-second`; unit `a_new_query_highlights_its_first_result_and_arrows_stop_at_the_ends`; test `a_new_query_s_highlighted_first_result_shows_after_arrowing_down` (a new query scrolls to the top: coordinator ruling) |
//! | Enter or a single click on a result opens it and closes search | scripts `enter-opens`, `click-opens`; tests `a_press_held_while_a_batch_moves_its_row_opens_nothing` (left and middle), `a_click_between_batches_still_acts_on_its_row` |
//! | With an input method composing…, Enter that the input method passes back… opens the highlighted result | test `an_enter_after_the_composition_commits_opens_the_highlighted_result` (scripts have no input-method events: U-C6-7) |
//! | Alt+Enter, or right-click → Open containing folder, closes search and opens the result's parent… | scripts `reveal-alt-enter`, `reveal-menu` (the menu is a stand-in for C21's; the parent and selection are Strata's); test `a_right_press_held_while_a_batch_moves_its_row_asks_for_no_menu` |
//! | Alt+Enter with Hyper or Meta held also opens the containing folder | script `reveal-alt-super-enter` (Rune reports those as Super) |
//! | Escape, or a click outside the dialog panel, closes search | scripts `escape-closes`, `click-outside-closes`; test `shift_escape_closes_as_escape_does` (Strata's `ESCAPE_ANY`) |
//! | With a pinyin input method composing…, Escape cancels only the composition… | test `escape_while_composing_keeps_the_palette_open` |
//! | Closing search with Escape, Ctrl+K, or a click outside the panel returns focus to its opener… | scripts `escape-closes`, `click-outside-closes`, `ctrl-k-toggles` |
//! | Opening a result, or Open containing folder, hands focus to the revealed item… | scripts `open-hands-focus`, `reveal-hands-focus`; test `opening_a_result_hands_focus_to_the_view_in_a_resting_window` (the host names the view; the item inside it is Strata's) |
//! | Ctrl+Shift+K opens this dialog as a "Jump to a folder…" palette; Ctrl+K still opens global file search | scripts `jump-opens`, `search-after-jump` |
//! | Pressing Ctrl+Shift+K while the search palette is open closes it | script `jump-toggle`; unit `the_toggle_closes_whatever_mode_is_open` |
//! | Opened with Ctrl+Shift+K, the field's accessible name and placeholder… | script `jump-opens`; golden `c17-jump` |
//! | Arrow keys choose a row, and Enter opens the chosen folder | script `jump-enter-opens` |
//! | Before any folder is recorded, the palette shows "No folder history yet" | script `jump-history-empty`; golden `c17-jump-empty`; test `the_jump_lists_its_history_in_the_frame_it_opens` (the page answers the jump at once, as Strata does, so no "Searching…" frame) |
//! | A query that matches no visited folder shows "No matching folders" | script `jump-no-match` |
//! | Closing the palette with Escape returns focus to the control that opened it… | script `jump-escape` |
//! | Every search covers the global roots…; roots recomputed each open; a drive mounted inside Home appears once | **Strata** (the roots; the palette asks again on every open) |
//! | Matching and results (19: path matches, ranking tiers, typos, NFC, dotfiles, skipped trees, ignore files, the 100 cap, tie order, appended order, dropped hits, walks stopped by file operations) | **Strata** (the search; the palette shows what it is given, in order) |
//! | Search limits (4), Outside changes (10) | **Strata**; a rescan's answer for the same query is a same-query update here |
//! | Opening a folder result navigates…; a file result reveals it and previews…; Open search results directly…; for a folder result, Open containing folder selects it… | **Strata** (`Open` and `Reveal` name the result) |
//! | Only local folders that finish loading are recorded…; A folder that fails to load is recorded after a successful retry | **Strata** (the history) |
//!
//! `browser/navigation/folder-jump`: "Typing narrows the list…" is script `jump-narrows`
//! (**partial**: the page's matcher); the other five (frequency and recency order, fzf terms,
//! ranking, the 100 cap) are **Strata**'s history search.
//!
//! `integration/10xer-mode/places`: **not applicable** (owner ruling): Strata's 10xer folder
//! picker is the footer prompts (`go ›`, `move to ›`, …), which C18 builds.
//!
//! No bullet names these, so they are tests and goldens: a superseded query's answers never
//! show (test `a_superseded_query_s_answers_never_show`, every frame of a resting window; unit
//! `an_answer_for_a_superseded_query_is_dropped_and_the_old_results_stay_until_the_new_land`),
//! each state reads its own words (test `each_state_reads_its_own_words`; goldens `c17-loading`,
//! `c17-error`), the new-tab chords and middle click (test
//! `ctrl_enter_and_a_middle_click_open_in_a_new_tab`), a held Enter opens nothing, even one
//! held through loading (test `a_held_enter_opens_nothing_even_through_loading`), and the
//! count's announcements (unit `the_count_is_announced_at_the_first_results_and_when_done`;
//! test `a_new_query_announces_no_count_until_its_first_answer`: nothing is announced for the
//! old list's size).
//!
//! Perf (`tests/perf.rs`, ignored, perf profile): at 1,000 and 10,000 results an arrow step, a
//! delivery, a streamed batch and a keystroke stay within their bars (4, 8, 8 and 8 ms at p95).
//!
//! # Deliberate changes
//!
//! - **Options are keyed by place** ([`option_key`]), for the scroll policy above (U-C17-3).
//!   What assistive technology hears changes with it, against Strata, which has no active
//!   descendant (its rows take focus) and a widget per result:
//!   - An option's identity follows its place, not its result: when a batch reorders results,
//!     the option at a place keeps its id and changes its name and position, rather than an
//!     option moving.
//!   - The active descendant follows the highlighted result's place: when a batch puts results
//!     above the same highlighted result, the active descendant moves (say from `option-0` to
//!     `option-2`), and a screen reader may announce the same result again.
//!   - When the highlighted result is dropped, the highlight stays at its place (clamped), so
//!     the active descendant stays put and only that option's name changes.
//!   - Names (full paths), the selected state and positions stay right in every case.
//! - **Rune's core `text_input`, frameless**, not rune-kit's `TextInput`: Strata's field has no
//!   box, and rune-kit's inner field takes no secondary press (U-C21-2), so C21's field menu
//!   couldn't attach. C18 made the same choice; one shared field is a follow-up.
//! - **A new-tab key and a middle click** (owner ruling): Strata's palette has none. The host's
//!   `open_in_new_tab` chords (Ctrl+Enter, Shift ignored) and a middle click send
//!   `Open { new_tab: true }`.
//! - **Matched characters on both lines**, from W1's `MatchedText` (the WP asks for it); Strata
//!   highlights none. Coloured, not bold.
//! - **A count live region** (owner ruling), hidden as Strata shows no count.
//! - **The dialogs are named** "Search files and folders" and "Jump to a folder" (owner ruling);
//!   Strata's panel is unnamed.
//! - **Kit tokens for the look**: the Kit's hover and selected row fills and its focus border
//!   for the highlight's outline, where Strata uses accent alphas; the name at the Kit's medium
//!   weight, not 650.
//! - **The unavailable field is read-only**, not disabled: at 5d421fd `El::disabled` neither
//!   stopped a `text_input` taking focus nor editing (probe P4; later Rune changes `disabled`
//!   to opt out of focus, not re-probed here). With no edit handler it takes neither, and reads
//!   read-only.
//! - **Panel sizes are module constants** citing Strata's CSS (owner ruling).
//!
//! # What stays in Strata
//!
//! The search (roots, exclusions, limits, walking, matching, ranking, the cap, rescans), the
//! history and its search, opening, previewing and revealing results, the result and field
//! menus (C21), thumbnails' rendering (C25's host), the key map (Ctrl+K, Ctrl+Shift+K and the
//! chords the palette is given), the header button's active look, and the words in other
//! languages.
//!
//! # Rune gaps
//!
//! Drafts, not filed (the owner files). Probes are in `strata-spikes/c17-probes`.
//!
//! - U-C17-1 is unused: the hidden live region works (probe P5).
//! - Reused: U-C6-1 (the combobox group), U-C21-2 (rune-kit inputs take no secondary press),
//!   U-C6-7 (no input-method events in scripts:
//!   in-process tests), U-C25-3 (scripts can't see a drawn image), #203 (a `virtual_rows` count
//!   change re-reads every key: a 10,000-result batch is the slowest figure, about 4 ms).
//! - **U-C17-3 (draft, not filed): `virtual_rows` keeps its first visible row at offset 0.**
//!   `El::scroll_anchor`'s docs (`el/scroll.rs:126-133`) say anchoring holds "only while the
//!   viewport is … scrolled off its top … at the very top new content above shows", and that
//!   `virtual_rows` lists anchor "this way"; but a `virtual_rows` list at offset 0 that gets rows
//!   inserted above its first row keeps that row at the top, scrolling the new ones out above
//!   the viewport. A search list streaming better results first hides them though the user
//!   never scrolled; and a list that wants to keep its pixel offset as rows move (Strata's)
//!   can't say so. Repro, headless (strata-spikes/c17-probes/probe_reveal.rs): a 200 pt
//!   viewport over `virtual_rows` of 30 rows `b00..b29`, 20 pt each, at offset 0; one commit
//!   later the same rows with `a0`, `a1` in front: `a0` is drawn at y = −40, above the
//!   viewport, and `b00` stays at y = 0. Ask: skip
//!   the anchor at offset 0, as the docs say, or an opt-out. Searched upstream ("scroll_anchor",
//!   "virtual_rows anchor", "overflow-anchor", "first visible row"): none. Here: rows are
//!   keyed by place, so the anchored key keeps its place and the offset stays. (Round 1 asked
//!   for an instant scroll to the old offset in the batch's commit instead; that request beat
//!   a wheel or fling in the same frame, review round 2's B1.)
//! - **U-C17-4 (draft, not filed): `reveal_selected` does nothing for rows built by index.** At
//!   the pin, `runtime/commit/measure.rs:531` checks `focus.reveal_selected` only in the branch
//!   for rows handed over as children (`virtualize`), not for `virtual_rows`' index-built rows,
//!   so a changed `focus_selected` row never scrolls into view. Repro, headless (same file): a
//!   200 pt viewport over 100 rows of 20 pt with `.reveal_selected(true)`, row `i` built
//!   `.focus_selected(i == s.selected)`; set `selected = 60`: with the rows as `virtualize`
//!   children, row 60 is revealed at y = 180; built by index with `virtual_rows`, it is never
//!   mounted and the viewport stays at 0. Ask: the same reveal for index-built rows (their keys are known). Searched upstream
//!   ("reveal_selected", "virtual_rows reveal"): none. Here: an arrow reveals its row with
//!   `Tx::scroll_into_view_in`, and `deliver` scrolls for answers.
//!
//! # Follow-ups
//!
//! - A `palette` slot in strata-kit for the panel's sizes (owner ruling).
//! - One field shared with C18's filter bar and footer prompts, once both have merged.
//! - One candidates model shared with C18's `go ›` prompt (the same answer shape).

mod data;
mod field;
mod intent;
mod mode;
mod model;
mod state;
mod view;

#[cfg(test)]
mod tests;

pub use data::{Answer, PaletteItem, PaletteQuery};
pub use intent::{By, PaletteIntent, Via};
pub use mode::{Hint, IdleText, PaletteMode, ResultKind};
pub use model::Phase;
pub use state::{PaletteState, Polled};
pub use view::SearchPalette;

/// The panel's width when the window has room (`.search-dialog`'s `min-width`, Strata's
/// `src/style.css`).
pub const PANEL_WIDTH: f32 = 760.0;

/// The search bar's height (`.search-bar`'s `min-height`).
pub const BAR_HEIGHT: f32 = 58.0;

/// The space between the bar's icon, field and spinner (`search_bar`'s box spacing,
/// `src/ui/search/panel.rs`).
pub const BAR_GAP: f32 = 10.0;

/// The left and right padding of the bar, the rows and the footer (`.search-bar`,
/// `.search-result > box` and `.search-footer`: 18 px).
pub const PAD_X: f32 = 18.0;

/// The results area's height (`.search-results-scroll` and the stack's 360 px).
pub const RESULTS_HEIGHT: f32 = 360.0;

/// The footer's height (`.search-footer`'s `min-height`).
pub const FOOTER_HEIGHT: f32 = 34.0;

/// The space between footer hints (`search_footer`'s box spacing).
pub const FOOTER_GAP: f32 = 18.0;

/// The space between a hint's icon and its words (`icon_hint`'s box spacing).
pub const HINT_GAP: f32 = 5.0;

/// A hint's icon (`icon_hint`'s 13 px icon).
pub const HINT_ICON: f32 = 13.0;

/// The space between a row's thumbnail and its lines (`result_row`'s box spacing).
pub const ROW_GAP: f32 = 12.0;

/// The status's padding (`.search-status`: 28 px above and below, 24 px beside).
pub const STATUS_PAD_Y: f32 = 28.0;
pub const STATUS_PAD_X: f32 = 24.0;

/// The field's key, under the palette keyed `key`.
pub fn field_key(key: &str) -> String {
    format!("{key}-field")
}

/// The result list's key (role `listbox`), under the palette keyed `key`.
pub fn results_key(key: &str) -> String {
    format!("{key}-results")
}

/// The key of the row at `index` in the result list, under the palette keyed `key`: also the
/// field's active descendant while that row is highlighted.
///
/// Rows are keyed by place, not by result: Rune's virtual rows keep the row whose key was
/// first in view where it is when a commit adds or removes rows (U-C17-3), so keys by place
/// keep the list's pixel offset as a batch reorders its results, as Strata's list does, with no
/// scroll request of the palette's to fight a wheel or a fling.
pub fn option_key(key: &str, index: usize) -> String {
    format!("{key}-option-{index}")
}
