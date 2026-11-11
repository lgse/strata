//! FilterBar and FooterPrompt (C18): the Ctrl+F filter and its Enter target
//! (`browser/search/filter`, `.../filter/enter-target`), and the 10xer footer prompts, chord
//! panel and flash (`integration/10xer-mode`).
//!
//! Work package: C18, `docs/work/phase-2-composites/C18-filter-bar-footer-prompt.md`.
//! Behaviour follows lgse/strata `upstream/main`: `src/ui/enter_target.rs`,
//! `src/ui/browser_modes/filter.rs`, `src/ui/shortcut_footer/` (`prompt.rs`, `candidates.rs`,
//! `chord_panel.rs`, `indicators.rs`), `src/ui/tenxer_mode.rs` and
//! `src/ui/window/keyboard/prompts.rs`.
//!
//! **Rune 2a44156: partly.** Core `text_input`, `TextEdit`, `popover::anchored` and
//! `raised`, `El::focus_return`, `Tx::focus`, `preserve_focus_on_press`, live regions,
//! `Tx::send_key` and `Tx::wake_after` cover the field, the bar, the panels and the timers.
//! Missing: a focus-change hook (#106, U-C6-3), a field menu for rune-kit's `TextInput`
//! (U-C21-2) and a `Keybind` that keeps case (U-C18-1).
//!
//! # Parts
//!
//! - [`filter`]: [`FilterState`](filter::FilterState) (held by the host, by accessor) and
//!   [`FilterBar`](filter::FilterBar), the funnel and the field. The host filters the source; the
//!   bar reports the query and picks what Enter opens.
//! - [`prompt`]: [`PromptState`](prompt::PromptState) and
//!   [`FooterPrompt`](prompt::FooterPrompt), with a [`PromptSpec`](prompt::PromptSpec) preset for
//!   each of Strata's twelve prompts. Candidates arrive as data, by generation.
//! - [`chord`]: [`ChordState`](chord::ChordState), its key map as data
//!   ([`ChordMap`](chord::ChordMap)), the footer pill and the panel of next keys.
//! - [`Flash`]: the footer's 2 s message ("Unknown chord", "No matches for “x”").
//! - [`keycap`]: the keycap chip the panel and the legend draw.
//!
//! None of them does I/O. Matching, searching, the folder picker, the name checks, opening and
//! the key map are Strata's.
//!
//! # The filter
//!
//! ```ignore
//! let bar = FilterBar::new("pane-filter", |s: &App| &s.filter, |s: &mut App| &mut s.filter)
//!     .labels(FilterLabels { field: "Filter list (Ctrl+F)".into(), placeholder: "Filter 12 items…".into() })
//!     .results(|s: &App| Results { source: &s.shown, selection: &s.coll.selection })
//!     .previews(|s: &App| s.a_selected_result_is_a_file())
//!     .focus_return("pane")              // the results' focus stop
//!     .on_intent(|s: &mut App, intent, tx| s.filtered(intent, tx));
//! // The view: the bar right before the results' focus stop.
//! // The results: Collection::marks sets `enter_target` where s.filter.outlined(..) == Some(id);
//! //   Collection::on_selection calls s.filter.pick(); Collection::on_press, results_scrolled().
//! // Ctrl+F or `/`: s.filter.open(). A printable key in the listing: s.filter.open_with(c, position).
//! // FilterIntent::Query: filter, then s.filter.shown(&text) (true: clear the results' selection),
//! //   then s.filter.take_reveal(..) into CollectionState::reveal.
//! // FilterIntent::ShowListing { restore }: the whole listing, Selection::restore, then sync.
//! // Each poll: s.filter.tick(). Escape from a result: s.filter.close(). Up from the first: focus_field().
//! // The results dropped (a scope or limit changed): s.filter.restart().
//! ```
//!
//! - **The Enter target** (Strata's `Tracker`): until the user picks a result after the query
//!   last changed, Enter opens the top result. After a pick, while something is selected, it
//!   follows Strata per mode (owner's ruling): search results that replace the listing open
//!   their cursor, or the top result with none (`inline_search.rs`, `current_position`), where
//!   Down lands; a listing narrowed in place opens its first selected result in display order
//!   (`filter_activation_position`). Deselecting everything gives the top result back. A query
//!   edit (one typed back
//!   included) drops the pick; with search results that replace the listing it drops their
//!   selection too ([`filter::FilterState::shown`] returns true), while a listing narrowed in
//!   place ([`filter::Shows::InPlace`]: Trash, Recent, a network share) keeps its selection. A
//!   same-query update keeps the pick. In 10xer mode nothing is outlined and an edit keeps the
//!   selection; Enter follows the per-mode rules from the start.
//! - **The outline** is C1e's `RowMarks::enter_target` (a 1 pt `text_accent` outline), on the top
//!   result while Enter opens it and the field has the keyboard.
//! - **Keeping the top result in view**: after each set of results, while the field has the
//!   keyboard and the user hasn't scrolled them since the query changed.
//! - **Keys in the field.** Enter opens the target (a held Enter once). Down lands on the top
//!   result while Enter opens it, else on the results' cursor (Strata's search results,
//!   `inline_search.rs`; [`filter::FilterState::landing`]), as
//!   [`filter::FilterIntent::FocusResults`], picks it, then moves the keyboard to the results:
//!   to the bar's `focus_return` element with `Tx::focus`, else by a Tab sent from the field
//!   (probe P1).
//!   Space previews when something is selected and the host's `previews` accepts it; with only
//!   the outline, or with Shift, it types. Escape closes the bar. A right-click, the Menu key or
//!   Shift+F10 is [`filter::FilterIntent::FieldMenu`], never the folder menu. Rune hands Enter
//!   and Escape to an input-method composition first.
//! - **Escape and the selection.** When a query starts over results that replace the listing,
//!   the bar saves the listing's selection (C1a's `Position`); closing the bar or clearing the
//!   query gives it back in [`filter::FilterIntent::ShowListing`]. In place, the listing's own
//!   selection stays. Escape removes the bar with the keyboard in it; it goes to the host's focus
//!   stop ([`filter::FilterBar::focus_return`]).
//! - **Restart**: after the host dropped the results (a scope or a search limit changed),
//!   [`filter::FilterState::restart`] makes the next shown query new, so the pick goes even for
//!   the same text (Strata's `EnterTarget::restart`).
//! - **Focus.** The field keeps the keyboard through listing changes. `focus_field` gives it
//!   back (probe P2: setting `TextEdit::focused` moves Rune's focus on the next frame).
//!
//! # The prompts
//!
//! ```ignore
//! let prompt = FooterPrompt::new("prompt", |s: &App| &s.prompt, |s: &mut App| &mut s.prompt)
//!     .focus_return("pane")
//!     .on_intent(|s: &mut App, intent, tx| s.prompted(intent, tx));
//! // `r`: s.prompt.open(Kind::Rename, PromptSpec::rename(), name, Some(stem));
//! // PromptIntent::Submit { serial, .. }: act, then s.prompt.close(); or s.prompt.refuse(serial, reason).
//! // PromptIntent::Complete { query, .. }: list, then s.prompt.deliver(Candidates { generation, items, done }).
//! // Each poll: s.prompt.tick().
//! ```
//!
//! | Prompt | Label | Up, Down | Tab | Candidates |
//! | --- | --- | --- | --- | --- |
//! | `/`, `?` | `/`, `?` (keycaps) | the listing's cursor | | |
//! | `f`, `s`, `create ›` | `filter:`, `search:`, `create ›` | the listing's cursor | | |
//! | `go ›` (no undo) | `go ›` | choose, wrapping | writes the chosen folder | the folder picker's |
//! | `move to ›`, `copy to ›`, `extract to ›` | as named | choose, wrapping | writes the chosen folder | the folder picker's |
//! | `jump ›`, `recent ›` | as named | choose, wrapping | | history, with "2 of 5" |
//! | `rename ›` | `rename ›` | nothing | | |
//!
//! - Opening one closes any other ([`prompt::By::Replaced`]) and carries none of its text; a
//!   pre-filled text keeps the caret after it, unselected, and `rename ›` selects the range the
//!   host names (the stem: C5's `initial_selection`).
//! - Enter (Shift+Enter too) submits and leaves the prompt open: the host closes it once it
//!   acted, or keeps it open with a reason ([`prompt::PromptState::refuse`]: the field is
//!   invalid and the reason is its hint and description). Every edit takes a new serial, so a
//!   check answering for older text (or after the prompt closed) changes nothing.
//! - Escape closes it ([`prompt::PromptIntent::Dismissed`], with the text: `s` keeps non-empty
//!   hits). The field losing the keyboard closes it too, a frame later (a clicked row, another
//!   pane), unless the host holds it open ([`prompt::PromptState::hold_open`]: while the
//!   shortcut reference has the keyboard, as Strata's `abandoned` does). Removing the bar gives
//!   the keyboard back to the host's focus stop (probe P3).
//! - Candidates: every open, close and edit starts a new generation; the previous list stays
//!   until the new one lands, and a stale answer, or one for a prompt that lists none, is
//!   dropped. A choice the user made survives later batches while its candidate is listed, until
//!   the text changes (Tab's write included: Strata's `forget_candidate_step`); otherwise the
//!   first follows the ranking. Shift+Tab completes nothing and moves focus back, as GTK's. An early Enter can wait
//!   for the listing to finish ([`prompt::PromptState::wait_until_settled`], Strata's
//!   `when_settled`). The list hangs above the field, 280 to 560 pt wide, with the keys' legend
//!   under it; the pointer tints a row and a press submits it.
//! - No-undo prompts swallow Ctrl+Z and Ctrl+Y, and closing clears the text, so a typed
//!   credential can't come back.
//!
//! # The chord panel and the flash
//!
//! - The host arms a chord on its prefix key ([`chord::ChordState::arm`]) and gives it every
//!   next key first ([`chord::ChordState::key`], from its pane's key handler, ahead of the
//!   collection). A modifier alone passes; Escape cancels quietly; a key the map names
//!   completes; any other cancels as [`chord::ChordOutcome::Unknown`] ("Unknown chord").
//!   Letters match with their case (`g` and `G` differ), following the layout (Rune's
//!   `Chord::matches_tx`).
//! - The pill ([`chord::pill`]) and the panel ([`chord::panel`]) show in the same frame as the
//!   prefix key, and nothing times a chord out: an armed chord asks for no wake. The panel sits
//!   above the pill, its right edge with the pill's, down two columns; it takes neither focus
//!   nor the keyboard. The host clears the chord when a prompt opens, Settings opens, the mode
//!   is left or the window closes.
//! - [`Flash::show`] asks for a wake on its `Tx`; the next [`Flash::tick`] stamps the time and
//!   asks for a wake when the 2 s are up, so the flash goes in a window at rest. A new flash
//!   starts its 2 s again.
//!
//! # What assistive technology hears
//!
//! - The filter field: a text input named after the pane's toggle ("Filter list (Ctrl+F)"), with
//!   the placeholder. The funnel is hidden. The outline is visual only, as in Strata.
//! - The results' status ("No matching files", [`filter::status`]): a polite live region.
//! - A prompt's field: named by its spec ("Rename the focused item"), its hint (or refusal) as
//!   its description, invalid while refused. The visible label is hidden: the name says it in
//!   full. A prompt with candidates is a combobox group, expanded while they show, with the
//!   chosen option as the field's active descendant; the list is a listbox whose description is
//!   the legend ("↑↓ Choose, Tab Complete, Enter Open, Esc Cancel").
//! - The chord pill: a polite status ("g-"). The panel: a list named by the map ("Keys after
//!   g"), each line "g: First item", with its place in the set. The flash: a polite status.
//!
//! # Coverage
//!
//! Scripts are `tests/scripts/c18/<name>.json`; "test" is in `crates/strata-gallery/tests/c18.rs`;
//! "unit" is in `tests/`; "golden" is a scene's a11y or PNG golden. The gallery page stands in
//! for Strata's key map and matching.
//!
//! `browser/search/filter`:
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | Ctrl+F, or the pane's filter button, reveals the filter field… | script `ctrl-f-opens` (the page binds both) |
//! | With Type to search on, typing a printable character… opens the filter with that character | scripts `type-to-search`, `type-replaces-query` (from a result, the key replaces the query, as Strata's `show_filter_with_query` does); test `a_burst_of_type_to_search_keys_all_reach_the_query` (which keys, and the preference, are Strata's) |
//! | With Type to search on, `/`… opens an empty filter | script `slash-opens-empty` |
//! | Escape in the filter field, or clearing the query, restores the normal listing… | scripts `escape-restores`, `clearing-restores` (the listing's cursor row has the keyboard after); unit `closing_or_clearing_gives_back_the_listing_and_its_selection` (hidden files are Strata's) |
//! | With a pinyin input method composing…, Escape cancels only the composition | test `escape_while_composing_keeps_the_filter_open` |
//! | Escape on a focused result closes the filter… with the directory's own selection | script `escape-on-result` |
//! | Escape on a focused row also closes a filter that narrows rows in place | script `escape-in-place` |
//! | Press Ctrl+F while a large… folder is loading… | **partial**: script `listing-change-keeps-field` (a listing change stands in for a load ending) |
//! | Down from the field focuses a result…; Up from the first result returns to the field | scripts `down-focuses-results`, `down-lands-on-cursor`, `down-and-up`; unit `down_lands_on_the_cursor_after_a_pick_and_on_the_top_result_before` |
//! | In the filter field or its results, Space toggles quick preview…; Shift+Space types a space | scripts `space-previews`, `shift-space-types` (in the field; in the results it is Strata's key map); test `down_and_space_while_composing_stay_with_the_composition` |
//! | When another program deletes the listing's cursor entry…, a focused filter field or result keeps focus | test `a_deleted_cursor_entry_leaves_the_field_and_the_prompt_focused` (the field) |
//! | In Columns…, Ctrl+F returns focus to that column's open filter field with its query | **partial**: `FilterState::open` on an open bar refocuses it (unit `tick_reports_the_field_gaining_and_losing_the_keyboard`); the column is C2's |
//! | In Icons and List, with no results, the status sits centered… | golden `c18-filter-empty` ([`filter::status`]) |
//! | Matching (13), Scope (10 more), Live updates (most), view switching, Ctrl+1–3, results selection, rename, delete and drag | **Strata** and C1–C5: the host filters, opens and selects; [`filter::FilterState::field_focused`] tells its key dispatch |
//!
//! `browser/search/filter/enter-target`:
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | With the field focused and a query with results, the top result has a 1 px accent outline | script `outline-follows-query`; golden `c18-filter-target` |
//! | In search results the outline comes with no selection; the in-place filter keeps the listing's selection | goldens `c18-filter-target`, `c18-filter-in-place`; unit `a_query_edit_drops_the_pick_and_a_same_query_update_keeps_it` |
//! | Moving focus from the field to a result or elsewhere removes the outline | scripts `outline-leaves-with-focus` (a click), `tab-out-removes-outline` (no pick); unit `enter_opens_the_top_result_outlined_while_the_field_has_the_keyboard` |
//! | In Icons and List, Enter in the field with no pick opens the outlined top result | script `enter-opens-top`; test `a_held_enter_opens_once`; unit, as above |
//! | A pick, such as Ctrl+click…, removes the outline; Enter then opens the pick | scripts `pick-then-enter`, `enter-opens-cursor` (search results: the cursor), `enter-in-place-opens-first-selected` (in place: the first selected); units `after_a_pick_enter_opens_the_pick`, `after_a_pick_enter_in_search_results_opens_the_cursor`, `after_a_pick_enter_in_place_opens_the_first_selected_result` |
//! | A pick is an arrow key, Down from the field, a click, a Ctrl+click, focus moving into the results… | scripts `pick-then-enter`, `down-and-up` (the host calls `pick` from C1e's `on_selection`) |
//! | Deselecting every result after a pick returns the outline and Enter to the top result | script `deselect-returns-top`; unit `deselecting_every_result_after_a_pick_returns_the_top_result` |
//! | In search results, editing the query text drops the pick and the results' selection | script `edit-drops-pick`; unit `a_query_edit_drops_the_pick_and_a_same_query_update_keeps_it` |
//! | Editing the query and typing it back… still counts as a query edit | script `edit-typed-back` (both keys within the page's 40 ms debounce, Strata's `FILTER_DEBOUNCE_DELAY`: one query shows); units, as above and `a_restart_makes_the_same_query_new` |
//! | In Trash, Recent, or a network share…, a query edit keeps the selection but drops the pick | script `in-place-edit-keeps-selection`; unit, as above |
//! | A same-query update… keeps the pick, the selection, and its preview | script `same-query-keeps-pick`; unit, as above |
//! | While the field has focus, the list keeps the top result scrolled into view as results arrive | test `the_top_result_follows_the_results_until_they_are_scrolled`; unit `the_top_result_follows_until_the_results_scroll_and_again_after_an_edit` |
//! | A button press, wheel scroll, touch, or drag hover on the results stops that scrolling… | **partial**: the same test calls `results_scrolled` as a host's wheel hook would: a host can't hear a wheel over a C1e collection (Rune's `on_wheel` stops at the nearest scroll view; `Collection::on_scroll` reads no state): a C1e seam follow-up. A press comes through `Collection::on_press` |
//! | With only the outline showing, Space in the field types a space and opens no preview | script `space-types-with-outline` |
//! | With 10xer mode on, no result is outlined and a query edit keeps the selected result | units `tenxer_mode_outlines_nothing_and_keeps_the_selection`, `tenxer_search_results_open_the_cursor_or_the_top_result` |
//! | In the file chooser, Enter in the filter field chooses the top result… | **Strata**: the same [`filter::FilterIntent::Open`] |
//!
//! `integration/10xer-mode`:
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | Arming a chord… shows its pill… with a panel above it | script `chord-panel-shows`; test `an_armed_chord_stays_armed_at_rest`; goldens `c18-chord-go`, `c18-chord-sort` (the 10X pill is C19's) |
//! | The panel lists each valid second key as a keycap beside its action, and closes when the chord completes or Esc cancels it | scripts `chord-completes`, `chord-escape`; unit `the_next_key_completes_cancels_or_passes` |
//! | The panel never takes focus, so the next key still completes the chord | script `chord-panel-shows` |
//! | The `g` panel lists…; The `c` panel lists…; The `,` panel lists… | **partial**: the lists are Strata's data; goldens `c18-chord-go` and script `chord-sort` show the gallery's copy, review scene `c18-chord-copy` the `c` panel |
//! | An Enter that an input method consumes while composing never submits a footer prompt… | test `an_enter_the_input_method_takes_never_submits_a_prompt` |
//! | A `create ›` or `rename ›` name check still running when the prompt is dismissed or edited… | unit `a_check_answer_after_an_edit_or_a_dismissal_is_dropped` |
//! | Dialogs, the reference, text fields, footer prompts, armed chords… take Esc before any listing step | **partial**: scripts `find-empty-and-escape`, `chord-escape` (the prompt and the chord take it); the ladder is Strata's |
//! | Leaving the mode closes footer prompts, armed chords… | script `leaving-closes` (the host calls `close` and `clear`; peeks, preview keys and ranges are other components') |
//! | The rest (entering and leaving, chrome, keymap ownership, movement, selection, sidebar) | **Strata**, C1c, C19 |
//!
//! Its sub-nodes' prompt and chord bullets:
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | search: `/` and `?` open a footer find prompt… | script `find-prompt` (finding is Strata's; the page finds names) |
//! | search: Enter on an empty find prompt changes nothing; Esc… cancels | script `find-empty-and-escape` |
//! | search: In the `/`, `?`, `f`, and `s` prompts, Up and Down move the listing cursor… | script `prompt-arrows`; unit `up_and_down_step_the_listing_choose_a_candidate_or_do_nothing` |
//! | search: When another program deletes the cursor's file while the `f` or `s` prompt has focus… | test `a_deleted_cursor_entry_leaves_the_field_and_the_prompt_focused` |
//! | search: `f` opens a `filter:` prompt…; Enter commits the filter… | script `filter-prompt` |
//! | search: Pressing `f` again pre-fills… | script `filter-prompt-prefills`; unit `a_prefilled_prompt_keeps_its_text_unselected` |
//! | search: Esc from a nonempty prompt keeps the hits…; Pressing `s` while hits are showing pre-fills… | scripts `search-prompt-escape`, `search-prompt-keeps-hits` |
//! | places: `g Space` opens a footer `go ›` prompt…; Up and Down choose a candidate…; Tab writes the chosen folder… | script `go-prompt`; test `down_while_composing_chooses_no_candidate`; units `a_chosen_candidate_survives_later_batches_and_a_stale_answer_is_dropped`, `tab_writes_the_chosen_candidate_and_lists_again`, `an_edit_or_a_tab_forgets_the_choice`, `a_stale_batch_never_lands_in_a_later_prompt`, `shift_tab_completes_nothing` |
//! | places: The entry is cleared on submit, Esc, focus loss, a replacing prompt, and mode exit, and it keeps no undo history | script `go-prompt-clears`; units `a_new_prompt_replaces_the_open_one_without_its_text`, `a_prompt_without_undo_swallows_the_undo_keys`, `losing_the_keyboard_dismisses_the_prompt` |
//! | places: Enter on a typed path that exists acts on it at once; otherwise Enter waits… | test `an_early_enter_in_go_waits_for_the_picker_in_a_resting_window`; units `a_submit_can_wait_for_the_listing_to_finish`, `an_edit_drops_a_waiting_submit` (whether the path exists is Strata's) |
//! | places: Esc cancels an armed chord, and any other unknown second key cancels it with `Unknown chord` | script `chord-unknown`; tests `the_unknown_chord_flash_ends_in_a_resting_window`, `a_second_flash_restarts_its_time` |
//! | file-verbs: `r` and F2 open `rename ›`…; A file's stem… is selected | script `rename-prompt`; unit `rename_opens_with_the_range_the_host_names_selected` (the stem rule is the host's) |
//! | file-verbs: Enter renames the item…; a taken name keeps the prompt open… | scripts `rename-prompt`, `rename-refused`; golden `c18-prompt-refused` |
//! | file-verbs: Esc, a clicked row, another prompt, or leaving the mode discards… | script `prompt-discard`; units `losing_the_keyboard_dismisses_the_prompt`, `a_held_prompt_survives_losing_the_keyboard_until_released` (F1 keeps it: C19 holds it) |
//! | `jump ›`, `recent ›`, `create ›`, `move to ›`, `copy to ›`, `extract to ›` | presets only (owner's ruling): the same mechanism as `go ›` and `rename ›` |
//!
//! `browser/view-modes/columns`: "Right-clicking… the filter field opens no folder menu"
//! (script `field-right-click`), "Enter in a column's filter field with no pick… opens the top
//! result" (script `enter-opens-top`), "Down from a column's filter field selects the outlined
//! top result and makes it the pick" (script `down-and-up`). Which column holds the bar is C2's.
//!
//! `browser/context-menu` (C21's text menu over these fields, attached in the gallery):
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | While a text input has focus, Menu and Shift+F10 open the input's text-editing menu… | script `prompt-field-menu` (Shift+F10 in a prompt) |
//! | Escape closes the menu…, and returns focus to the entry… | scripts `field-menu-escape` (the filter field), `prompt-field-menu` (the prompt stays open: [`prompt::PromptState::hold_open`] while the menu has the keyboard), `prompt-field-menu-then-blur` (the hold ends with the menu); test `a_prompt_closed_under_its_field_menu_closes_the_menu`; unit `closing_a_prompt_drops_its_hold` |
//! | Text-input menu styling covers the filter and search fields… | scripts `field-right-click`, `field-menu-delete`, `prompt-field-menu-delete` (a menu edit reaches the query and the prompt: `text_edited`) |
//! | A menu opens with keyboard focus on its first visible action… | scripts `field-menu-keys-stay`, `prompt-field-menu-keys-stay` (a key the menu passes on reaches no key map under it) |
//!
//! # Deliberate changes
//!
//! - **Rune's core `text_input` in a text box frame of our own**, drawn with the Kit tokens
//!   rune-kit's `TextInput` uses, not `TextInput`: its inner field takes no
//!   `on_secondary_press` (U-C21-2), so its right-click opens Rune's native field menu, empty on
//!   Linux, and C21's `FieldMenu` can't attach to it (owner's ruling).
//! - **The field menu is an intent.** The bar and the prompt send `FieldMenu { at }`; the host
//!   answers it with C21's `FieldMenu::open`, calls `FieldMenu::check` from its poll, and passes
//!   the menu's edits back with `text_edited` (`FieldMenu::on_change`). For a prompt it holds
//!   the prompt open while the menu has the keyboard (`hold_open(true)` on the intent,
//!   `hold_open(false)` from `FieldMenu::on_close`; closing a prompt drops its hold too), so the
//!   menu's blur doesn't close it. **While a `FieldMenu` is open, the host's window-level key
//!   handler must stand down** (take every key and do nothing): the menu passes on the keys it
//!   doesn't use, and they would otherwise reach the host's key map under it (type to search,
//!   a prompt or a chord). The gallery does all of this (`scenes/c18`, `on_key`).
//! - **Keycaps are our own chip**, on Strata's keycap look, not rune-kit's `Keybind`, which
//!   upper-cases `g` to `G` (U-C18-1; owner's ruling).
//! - **No slide-down**: the bar appears and goes at once, where Strata's revealer slides (owner's
//!   ruling).
//! - **Live regions beyond Strata**: the results' status line, the chord pill and the flash are
//!   polite live regions (Strata's are plain labels; owner's and coordinator's rulings).
//! - **Down from the field moves the keyboard to the bar's `focus_return`** element; without
//!   one, by a Tab sent to the field (P1), so the bar must then come right before the results'
//!   focus stop ([`filter::FilterBar::tab_to_results`] turns it off).
//! - **Losing the keyboard is noticed a frame late**, in `tick` (U-C6-3).
//! - **The host reports picks** ([`filter::FilterState::pick`]): the bar can't see the results'
//!   selection change or focus enter them.
//! - **Enter after a pick follows Strata per mode** (owner's ruling): the cursor in search
//!   results (`inline_search.rs`), the first selected result in place
//!   (`filter_activation_position`). Strata keeps the two rules in two code paths; here one
//!   [`filter::FilterState::target`] picks by [`filter::Shows`].
//! - **Our own candidate list**, not rune-kit's `PopupList`, whose rows can't draw matched
//!   ranges (U-C6-4), as C6's.
//!
//! # What stays in Strata
//!
//! Matching and ranking (the filter, find, `f`, `s`, the folder picker), which locations narrow
//! in place, the 100-result cap and status words, opening and previewing, the name checks and
//! their messages, the stem rule, existence checks before Enter, the key map (Ctrl+F, `/`, Type
//! to search, `f`, `g`, the chords' second keys and their labels), the Escape ladder, the flash's
//! words, the sidebar's `g` keycaps (C8), the `10X` pill and footer marks (C19).
//!
//! # Rune gaps
//!
//! Drafts, not filed (the owner files). Probes are in `strata-spikes/c18-probes`.
//!
//! - **U-C18-1 (draft, not filed): `Keybind` upper-cases single characters, with no way to keep
//!   the case.** `rune_kit::Keybind::new(&["g"])` draws "G" (`keycap` in `keybind.rs` upper-cases
//!   any single character that maps back). Vim-style key maps tell `g` from `G` (Shift+g), so a
//!   chord panel or a sidebar keycap drawn with it shows the wrong key. Repro, headless:
//!   `--tree` on a view holding `Keybind::new(&["g"]).build(&kit)` shows a text "G". A fix: an
//!   opt-out (`.keep_case()`), or upper-case only when the key is named as a letter key rather
//!   than a typed character. Searched upstream issues ("keybind", "uppercase", "keycap"): none.
//! - Reused: U-C21-2 (rune-kit's text inputs take no secondary press), #106 and U-C6-3 (no
//!   focus-change hook), U-C6-1 (an input is always a text box: the combobox group), U-C6-4
//!   (`PopupItem` has no rich label), U-C6-7 (scripts have no input-method event: tests use
//!   `Runtime::ime`).

pub mod chord;
pub(crate) mod field;
pub mod filter;
mod flash;
mod keycap;
pub mod prompt;

#[cfg(test)]
mod tests;

pub use flash::{FLASH_FOR, Flash, flash};
pub use keycap::keycap;
