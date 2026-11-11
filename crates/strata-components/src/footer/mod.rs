//! Footer and ShortcutReference (C19): the window footer, with its F1 Shortcuts button, count,
//! Files on clipboard badge, 10X pill and marks, and the F1 key map panel
//! (`app/shortcut-reference`).
//!
//! Work package: C19, `docs/work/phase-2-composites/C19-footer-shortcut-reference.md`.
//! Behaviour follows lgse/strata `upstream/main`: `src/ui/shortcut_footer/` (`build.rs`,
//! `status.rs`, `indicators.rs`, `catalog.rs`, `reference.rs`, `reference_keys.rs`),
//! `src/ui/shortcut_reference.rs` and `src/style.css`.
//!
//! **Rune 2a44156: partly.** `DialogShell` (C15: Rune's modal, backdrop and Tab trap), core
//! `text_input` and `TextEdit` (editing, undo, word moves), focus groups with `tablist` and `tab`
//! roles, `virtual_rows` with `on_visible_rows`, `Tx::scroll_page`, `scroll_into_view` and
//! `scroll_to_instant`, the table, status and live-region roles, `El::checked` (a toggle's pressed
//! state) and `Chord::matches` beside `matches_tx` and `Tx::focus` cover it. Missing: a line scroll
//! for a focused viewport (draft U-C19-1), a `Keybind` that keeps case (U-C18-1), a hover border
//! colour (#328), window focus in scripts (U-X1-1) and input-method events in scripts (U-C6-7).
//!
//! # Hosting
//!
//! ```ignore
//! // State: s.reference: ReferenceState, s.shortcuts: ShortcutData (bump its revision on change).
//! let mut layer = Vec::new();
//! // The footer first: C18's prompt puts its candidates, and the chord its panel, on the layer.
//! layer.extend(filter_bar::chord::panel(&s.look, "chord", &s.chord));
//! let footer = Footer::new("footer")
//!     .shortcuts(s.show_f1.then_some((|s: &App| &s.reference, |s: &mut App| &mut s.reference)))
//!     .count(Some(footer::summary(&s.listing(), &s.words)))
//!     .clipboard(s.files_on_clipboard)      // from strata-platform's clipboard notices (X3)
//!     .tenxer(s.tenxer)
//!     .mark(s.tenxer_mark())                // filter: or search:, Strata's choice when
//!     .current_hit(s.hit_path())
//!     .range(s.range_kind())
//!     .flash(filter_bar::flash(&s.look, "flash", &s.flash))     // C18
//!     .chord(filter_bar::chord::pill(&s.look, "chord", &s.chord))
//!     .jobs(s.jobs_element())               // C16
//!     .prompt(prompt.build(s, &s.look, cx, &mut layer))          // C18's FooterPrompt
//!     .build(s, &s.look);
//! // The reference last, after prompt.build and the chord panel: a modal above every overlay.
//! ShortcutReference::new("reference", |s: &App| &s.reference, |s: &mut App| &mut s.reference,
//!         |s: &App| &s.shortcuts)
//!     .keys(if s.tenxer { ReferenceKeys::tenxer() } else { ReferenceKeys::default() })
//!     .reduced_motion(cx.is_reduced_motion())
//!     .build(s, &s.look, &mut layer);
//! // The window's on_key, in this order:
//! // 1. stand down while a C21 FieldMenu is open (C18's host rule: take every key, do nothing);
//! // 2. keys.toggles(ev, tx) toggles s.reference (F1 anywhere; `~` in 10xer only arrives with
//! //    no text field focused, as a field types it);
//! // 3. only then the armed chord (s.chord.key), then the rest of the key map.
//! // Each poll, before s.prompt.tick(): s.prompt.hold_open(s.reference.is_open() ||
//! // s.fields.is_open()) keeps a footer prompt open while the reference (or the prompt's field
//! // menu) has the keyboard, and while s.reference.is_open(), s.chord.clear(): no chord stays
//! // armed under the reference, whichever way it opened (the F1 button too). Strata runs the
//! // reference's keys before its chord consumer and cancels a chord on any key under the
//! // reference (`input_owner.rs`, `footer_key`).
//! // The gallery's page (`scenes/c19`) does all of this with C18's prompt, chord and flash.
//! ```
//!
//! # The footer
//!
//! - Left to right: the F1 Shortcuts button, the search hit under the cursor, then at the right
//!   the clipboard badge, the `filter:` or `search:` mark, the `VISUAL` or `UNSET` mark, C18's
//!   flash, the count, C16's jobs element, C18's chord pill and the `10X` pill (Strata's order,
//!   `build.rs`). An open C18 prompt takes the whole row.
//! - Its height is the look's `frame.control` plus `space.xs` above and below, whatever it shows:
//!   the pills share one look ([`pill`]), and the button and C18's prompt field are a bar control
//!   tall. The button's left inset equals its top and bottom ones (`space.xs`), so its hover
//!   highlight sits equally in from the footer's left, top and bottom edges.
//! - With the button hidden and nothing else to show, [`Footer::build`] returns `None` and the
//!   footer goes.
//! - The count comes from [`summary`] with Strata's rules and the host's words: a displayed
//!   filter or search counts its results; with nothing selected "N items"; a selection of up to
//!   [`DETAILED_SELECTION`] lists its folders and files and the selected files' size (known,
//!   incomplete or unavailable); every entry selected gives the breakdown without a size; past
//!   the limit, "N items selected". Folder contents are never counted.
//! - What fills the marks, and when, is Strata's: the default map passes no `filter:` or
//!   `search:` mark, and the badge flag follows the clipboard (a non-empty file list).
//!
//! # The reference
//!
//! - **Opening and closing.** [`ReferenceState::open`] (F1, `~` in 10xer, the button) shows the
//!   panel over the blurred window (C15) with the search field focused and the results at the
//!   top; the query and category stay from last time, and nothing closes it when the window
//!   loses focus. F1 again, Escape (with any modifiers, Strata's `ESCAPE_ANY`) or a click on
//!   the backdrop closes it; Rune's modal gives the keyboard back to what had it. Inside the panel the toggle chords close it: in the search
//!   field by the typed character alone, so a typed `~` (with Shift on US layouts, without it
//!   through AltGr on German and French ones) closes while Russian ё, which reads as `~` only
//!   through the layout fallback, types itself.
//! - **Keys.** Rune hands a focused field's keys to the field, then the window, never to its
//!   ancestors (probe P1), so the search field routes its own and the content routes the
//!   categories' and the results'. Ctrl+B, Ctrl+F and Ctrl+L move focus among the three stops
//!   (search, the chosen category, the results) with `Tx::focus`, from anywhere in the panel;
//!   Tab and Shift+Tab cycle them. In the categories the arrows,
//!   Home and End choose (a tab list), `h j k l` as the arrows, and Shift changes nothing. In
//!   the results, the arrows, `j`, `k`, Page Up, Page Down, Home and End scroll; in the search
//!   field Up, Down, Page Up and Page Down scroll the results and focus stays; Shift held too
//!   (Strata's `scroll_motion` ignores it). Every other key is taken, so no browsing shortcut
//!   runs under the panel, and the search field keeps only its editing keys: typing, Ctrl+A, C,
//!   X, Y and Z, Ctrl+Shift+Z, and the caret and word keys (not Ctrl+Shift+A, Strata's Unselect
//!   all, nor Alt+Space). The gallery logs any key that reaches its window hook while the panel
//!   is open, and a sweep of 66 chords from each focus stop finds none.
//! - **Search**, as Strata's: a row stays when its action, its keys with their context or one
//!   of its search terms contains the query, in lower case, with − and – read as `-`. The match
//!   is drawn in the action and the context with W1's [`MatchedText`](crate::contract::MatchedText).
//! - **Categories.** All, then each section, with its row count beside it; compact, wrapping
//!   pills without counts, at most a quarter of the panel's height (60 to 160 pt) before they
//!   scroll. A category the data no longer has shows All, and is forgotten. Choosing one is
//!   heard once: from the tab, which carries its count, while the announced result count stays
//!   silent until a query is typed.
//! - **Layout.** Below [`COMPACT_BELOW`](reference::COMPACT_BELOW) of panel width the header
//!   stacks, the categories wrap above the results and the results take one column; otherwise
//!   the categories sit in a 225 pt column and the results take two.
//! - **Data.** Rows arrive as [`ShortcutData`](reference::ShortcutData) with a revision; a change
//!   shows at once in an open panel. [`keymap_rows`](reference::keymap_rows) builds rows from a
//!   C1c `KeyMap`, aliases as one row of their own.
//!
//! # What assistive technology hears
//!
//! - The footer: the F1 button with its pressed state and description; the count as a polite
//!   status with its breakdown or selection note as the description; the badge, the range mark
//!   and the 10X pill as statuses, the pill named "10xer mode (experimental feature, under
//!   active development)"; the marks with the query as their description; the hit by its path.
//! - The reference: a modal dialog named "Keyboard shortcuts"; the search field named "Search
//!   actions or keys…"; a tab list of categories, each described by its count; a table of rows,
//!   each section's title as a row header and each shortcut a row of two cells (action, keys as
//!   text, the keycaps hidden) with its place among the rows shown; a hidden polite status with
//!   the number of rows shown or "No matching shortcuts"; the note.
//!
//! # Coverage
//!
//! Scripts are `tests/scripts/c19/<name>.json`; "test" is in `crates/strata-gallery/tests/c19.rs`;
//! "unit" is in `tests.rs` and `reference/tests.rs`; "golden" is a scene's a11y or PNG golden.
//!
//! `app/shortcut-reference`:
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | F1 opens the "Keyboard shortcuts" panel…; F1 again closes it | scripts `f1-opens`, `f1-toggles`, `chord-then-f1` (an armed chord doesn't eat F1); goldens `c19-reference` |
//! | F1 with Ctrl, Alt, Super, or Shift held does not toggle | script `f1-modifiers`; unit `toggle_chords_match_exactly_and_tilde_only_where_typed` |
//! | In 10xer mode, `~` also toggles…; in the default map `~` does not | scripts `tilde-tenxer` (ends on F1 finding the kept query), `tilde-altgr` (no Shift: AltGr), `tilde-default`, `chord-then-tilde` (after `g`); `yo-russian` (the Shift+backquote chord alone matches through the layout fallback: a typed `~` with Shift also matches the plain `~` chord, Rune's shifted-character rule); unit `a_tilde_typed_without_shift_toggles_in_tenxer` (which keys toggle is the host's: [`ReferenceKeys::tenxer`]) |
//! | In 10xer mode, `~` in a focused footer prompt or text field… types | scripts `tilde-in-field`, `tilde-in-prompt` (C18's prompt; `~` with and without Shift) |
//! | F1 still opens the reference from a footer prompt or text field | scripts `f1-from-field`, `f1-from-prompt` (C18's prompt, held open while the reference has the keyboard, keeps its text and gets the keyboard back) |
//! | In 10xer mode, `~` typed in the open reference's search closes | scripts `tilde-in-search`, `tilde-altgr-closes` (each ends on F1 finding the kept query) |
//! | In 10xer mode with Russian active and a text field focused, Shift and the ё key type Ё | scripts `yo-closes-nothing`, `yo-russian` (probe P5: a script's `code` event) |
//! | In 10xer mode, Russian ё reads as `~` only through the layout fallback… | script `yo-russian` |
//! | The footer's "F1 Shortcuts" button toggles… shows as pressed | **partial**: script `button-toggles` (opens; Escape closes and the button, unpressed, has focus back). The pressed state is drawn (golden `c19-reference`, behind the backdrop) but hidden from the tree under the modal, and a second press can't reach the inert button: it lands on the backdrop, which closes too |
//! | Escape or a click on the backdrop closes the reference | scripts `escape-closes`, `escape-modified`, `escape-modified-results` (Ctrl+ and Alt+Escape: Strata's `ESCAPE_ANY`), `backdrop-closes` (a tap by position: the backdrop has no spot of its own) |
//! | Focusing another application window leaves the reference open… | test `the_window_losing_focus_leaves_the_reference_as_it_was` |
//! | Closing the reference returns focus to the widget that had it… | scripts `f1-toggles`, `f1-from-field`, `button-toggles`, `trapped` |
//! | While the reference is open, Delete, Ctrl+V, and other browsing shortcuts do nothing… | test `no_key_reaches_the_window_while_the_reference_is_open` (66 chords from the search, the results and the categories; the page logs any leak); scripts `trapped`, `unused-chords-nothing`, `no-leaks`; `tab-cycle` (focus stays) |
//! | In the reference's search field, typing, Delete, and Ctrl+A, Ctrl+C, Ctrl+V, and Ctrl+X edit the query | scripts `search-edit`, `search-cut` (headless runs have no clipboard: the paste is a script `paste` event, and what Ctrl+C copies isn't checked); unit `the_search_field_keeps_its_editing_keys_and_takes_the_rest` |
//! | In the reference's search field, Ctrl+Z undoes and Ctrl+Y or Ctrl+Shift+Z redoes | script `search-undo` |
//! | …holding `open tab`, Ctrl+Left then typing `new `…; Ctrl+Backspace deletes a word | script `search-words` |
//! | With fcitx5…, Ctrl+Space… switches the input method, and Chinese can be typed there | **partial**: test `an_input_method_types_into_the_search`; the switch key is the input method's |
//! | In the reference's search, a Ctrl, Alt, or Super chord the field does not use reaches nothing else | test `no_key_reaches_the_window_while_the_reference_is_open`; scripts `unused-chords`, `unused-chords-nothing`, `no-leaks`; units `the_search_field_keeps_its_editing_keys_and_takes_the_rest`, `shift_is_an_editing_chord_only_with_z_and_alt_space_is_taken`. Alt with a letter types the letter (Rune's editor: draft U-C19-2) |
//! | Opening the reference focuses "Search actions or keys…" and scrolls the list to the top | script `open-focus`; test `reopening_scrolls_the_results_to_the_top_and_keeps_the_query` (the panel mounts afresh at the top) |
//! | The reference's search field is named "Search actions or keys…" | script `open-focus`; a11y goldens |
//! | Reopening the reference keeps the previous search text and selected category | script `reopen-keeps`; unit `the_state_keeps_its_query_and_category_across_closing` |
//! | Typing keeps rows whose keys, key context, or action contain the text…; no match shows "No matching shortcuts" | scripts `search-narrows`, `search-context`, `search-none`; unit `a_row_stays_when_its_action_keys_context_or_terms_contain_the_query_in_any_case` (translations are Strata's) |
//! | With no match, "No matching shortcuts" wraps and sits centered | script `search-none`; golden `c19-reference-empty` (PNG) |
//! | Key names match in English and in the interface language… | script `search-locale` (the German words and English terms are data) |
//! | Searching `ctrl+0` finds the "Ctrl++ / − / 0" row, and `1-9` finds… | scripts `search-ranges`, `search-digits`; unit `minus_and_en_dash_read_as_a_typed_hyphen` |
//! | The category list shows All and each section with its row count… | script `categories-filter`; test `a_category_change_is_announced_once`; golden `c19-reference-category` |
//! | Tab cycles search, the selected category, and the results… | scripts `tab-cycle`, `tab-cycle-back` |
//! | Ctrl+B focuses the categories, Ctrl+F the search field, and Ctrl+L the results | scripts `area-keys`, `area-keys-search` |
//! | In the categories, arrows and h/j/k/l select…, and Home and End… | scripts `category-keys` (its end depends on each of `h j k l`), `category-end`, `category-home`, `category-shift` (with Shift, as Strata's `category_step`) |
//! | In the results, arrows, j/k, Page Up, Page Down, Home, and End scroll the list | tests `the_results_keys_scroll_the_list_and_the_search_field_scrolls_it_in_place`, `shifted_keys_and_left_and_right_scroll_the_results_as_strata_does` (scroll offsets aren't in script trees) |
//! | With the search field focused, Up, Down, Page Up, and Page Down scroll the results without moving focus | script `search-scrolls-results`; the same tests (Shift held too) |
//! | In windows narrower than 1480 px, the header stacks, categories become wrapping pills… | **partial** (deviation: the panel's width): goldens (compact at 960); review scene `c19-reference-wide` |
//! | A note at the bottom reads "Ctrl+B categories · …"; The note wraps only after a "·" | goldens (a11y name, PNG); unit `the_note_names_the_bound_keys` |
//! | After the reference's category, search, or list key is rebound, the note names the new key | script `note-rebound` |
//! | Switching views with Ctrl+1, 2, or 3 replaces the navigation section… While open, Ctrl+1, 2, and 3 do nothing | scripts `live-view`, `trapped` |
//! | Turning 10xer mode on or off… updates an open reference | script `live-mode` |
//! | With F1 open, unbinding Ctrl+K… removes its row without reopening | script `live-view` |
//! | A row lists every key the active map gives its command…; a command with no key has no row | unit `every_chord_of_a_command_is_listed_and_a_command_without_one_has_no_row` (the C1c helper; Strata's catalog is its own) |
//! | Movement keys are described directly; the `h / j / k / l` row reads "Same as ← ↓ ↑ →…" | unit `aliases_make_one_row_in_arrow_order_after_the_first_command_they_stand_for`; golden `c19-reference` (the words are Strata's) |
//! | The rest of Contents (which sections and rows each map, view and chooser request lists), Menu hints | **Strata** (its catalog and C21): the gallery shows a copy |
//! | In Columns, Icons, and List, the footer shows "F1 Shortcuts" at the left and the clipboard badge and item count at the right | goldens `c19-footer`, `c19-footer-selection` (the view is the host's) |
//! | The hovered F1 Shortcuts button's highlight is inset equally… | **partial**: by construction (the insets above); the PNG goldens show the button at rest |
//! | With nothing selected, the count reads "N items"… | script `count-items`; unit `with_nothing_selected_the_count_reads_n_items_described_by_the_breakdown` |
//! | With 2 of 5 items selected, the count's accessible description reads… | script `count-selection` (its golden tree holds the description); unit `a_selection_lists_folders_then_files_with_the_selected_files_size` |
//! | With a selection, the count shows a folder and file breakdown with the selected files' total size… | scripts `count-selection`, `count-unknown`; unit, as above |
//! | When some selected file sizes are unknown… "(size unavailable)" | scripts `count-unknown`, `count-unavailable`; unit `unknown_sizes_read_incomplete_or_unavailable_and_folders_alone_have_no_size` |
//! | With a file symlink selected, the selection size counts its target's size | **Strata** (the sizes it passes) |
//! | Selecting every item shows the folder and file breakdown without a size | script `count-all`; unit `selecting_every_item_shows_the_breakdown_without_a_size` |
//! | Selecting more than 64 items, but not all, shows "N items selected" | unit `past_the_detailed_limit_the_count_names_only_how_many_are_selected` |
//! | An empty folder shows "0 items", and clearing the selection restores the directory total | script `count-clear`; unit, as for "N items" |
//! | In the default map, a Ctrl+F query shows the displayed result total… | scripts `count-filter`, `count-filter-miss` |
//! | Selecting results keeps the result total… | script `count-filter`; unit `displayed_results_count_themselves_whatever_is_selected_and_a_pending_search_keeps_the_directory` |
//! | Escape clearing the filter restores the directory count or selection summary | script `count-filter-escape` |
//! | A 10xer footer prompt's entry is named after the prompt label | **C18** |
//! | While a typed query's search has not reported back, the count keeps describing the directory | script `count-pending`; unit, as above |
//! | In the default map, the footer shows no `filter:` or `search:` mark… | script `marks-default` (which marks show is the host's) |
//! | Copying or cutting files shows a "Files on clipboard" badge…, whose description reads… | script `clipboard-badge` |
//! | The badge also appears for files copied in other applications, and disappears… | **partial**: script `clipboard-cleared` (the flag); which clipboards count is Strata's and X3's |
//! | A clipboard offering only `x-special/gnome-copied-files`…; The badge stays after pasting a copy… | **Strata** and X3 (the flag's rules) |
//! | The badge has the 10X tag's pill shape and height… | goldens `c19-footer-selection`, `c19-footer-tenxer` (PNG) |
//! | The badge stays visible when keybinding hints are hidden | script `hints-hidden` |
//! | With the F1 button hidden and no other footer status visible, the whole footer hides | scripts `footer-hides`, `footer-stays` |
//!
//! `integration/10xer-mode` and its `search` node:
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | The footer shows a `10X` pill in its far right corner…, and the footer height does not change | script `tenxer-pill`; goldens `c19-footer-tenxer`, `c19-footer-search` |
//! | The `10X` pill's accessible name is… | script `tenxer-pill`; a11y goldens |
//! | F1, or `~` outside text fields, toggles the shortcut reference, which lists only the commands of the active map and switches live | scripts `tilde-tenxer`, `live-mode` |
//! | `v` starts a range…; the footer shows `VISUAL` or `UNSET` while it is active | **partial**: script `range-mark` (the mark); ranges are C1c's |
//! | Arming a chord… shows its pill… beside the `10X` pill | scripts `chord-pill`, `chord-completes`, `chord-unknown-flash` (the flash slot); golden `c19-footer-tenxer` (C18's pill and panel in the footer's chord slot) |
//! | The panel lists each valid second key…, and closes when the chord completes or Esc cancels it | **C18**; with the reference: scripts `chord-then-f1`, `chord-then-f1-closed`, `chord-then-button`, `chord-then-button-closed` (opening the reference cancels the chord, so a later `h` completes nothing); test `the_reference_is_the_top_overlay` |
//! | search: While an `f` filter is active, the footer shows `filter: …` and the displayed result count, including zero | scripts `filter-mark`, `filter-mark-zero` |
//! | search: The footer shows `search: …`, the hit count including zero, and the hit under the cursor… | script `search-mark` |
//! | search: With some hits filled, `j`, `k`, `G`, `g g`, and Space move the footer path… | **partial**: script `hit-follows` (the path follows what the host passes) |
//! | search: While focus is outside the hits…, the footer keeps the last hit… | **Strata** (the hit it passes) |
//!
//! `browser/context-menu`, in the prompt: "While a text input has focus, Menu and Shift+F10 open
//! the input's text-editing menu…" and "Escape closes the menu…": script `prompt-menu-keys-stay`,
//! which also checks C18's host rule, that keys under the menu (F1, `~`) reach no key map.
//! `integration/10xer-mode`, "In the location field or an inline rename, letters… are typed and
//! no file command runs": script `prompt-commands` (Ctrl+1, Ctrl+, Ctrl+V, Ctrl+K and F5 from the
//! prompt run no page command; the prompt's keys are C18's).
//!
//! `settings/preferences`: "Turning off … Show F1 Shortcuts button hides the footer shortcuts
//! button…; F1 still opens the reference": script `button-hidden` (every window is Strata's).
//! `app/keyboard-shortcuts`, Labels: [`chord_label`](reference::chord_label) follows the order,
//! case, minus and arrow rules in English (unit `chords_read_as_strata_labels_them`); the
//! interface language's key names are Strata's.
//!
//! # Deliberate changes
//!
//! - **Keycaps, one per chord step** (`Ctrl+Shift+T`; `g` then `h`), with "/" between
//!   alternatives, where Strata draws one text box per row (owner's ruling; the plan asks for
//!   chips). The chip is C18's ([`keycap`](crate::filter_bar::keycap)).
//! - **Matched text in the search.** Strata only filters; the WP asks for W1's matched text, so
//!   the match is drawn in the action and the context (never on keycaps: Strata has no such
//!   highlight).
//! - **The compact switch reads the panel's width** (below 1400 pt), not the window's 1480 px: a
//!   Rune view knows the width it was laid out at. Goldens at 960 are compact.
//! - **Live regions beyond Strata**: the count (a polite status, as the WP asks), the badge, the
//!   range mark and the 10X pill (statuses), and the reference's announced row count (as C17's
//!   palette). Strata's are plain labels.
//! - **The results scroll a line at a time** (a heading or a row of the results), the next line
//!   out of view brought in, where GTK steps its adjustment (U-C19-1).
//! - **No crossfade** between the status row and C18's prompt (Strata's 120 ms stack
//!   crossfade), as C18 drops its slide.
//! - **No hover border on the F1 button** (Strata's accent at 42 %): Rune has no hover border
//!   colour (#328). The hover fill is drawn.
//! - **The words are data.** Counts, plurals and sizes come from [`CountWords`], the footer's and
//!   the panel's words from [`FooterLabels`] and [`ReferenceLabels`], in English by default.
//!
//! # Follow-ups
//!
//! - A W2 `footer.*` slot for the Strata alphas held as constants here (`style.css`).
//!
//! # Rune drafts (not filed)
//!
//! - **U-C19-1 (draft): no line scroll for a focused viewport.** Up and Down don't scroll a
//!   focused scroll view (key routing step 9 takes only Page Up, Page Down, Home and End), and
//!   `Tx` has no `scroll_by(key, dx, dy)`: a list that scrolls by arrow keys without a row
//!   cursor must reveal rows itself. Repro, headless: a focusable `.scroll(..)` column of 100
//!   20 pt rows; press Down; `--tree` shows the viewport's `scroll` offset still 0. A fix:
//!   `Tx::scroll_by`, or arrows scrolling the focused viewport that overflows, as Page Down does.
//!   Searched upstream ("scroll_by", "arrow keys scroll", "scroll line"): none.
//! - **U-C19-2 (draft): Alt with a letter types into a text field on Linux.** Rune's editor
//!   inserts any `Key::Text`, Alt held or not (`edit/input.rs:92` at the pin), so Alt+d types
//!   "d" in a focused field, where GTK types nothing and leaves the chord to the window's
//!   accelerators; an app can't tell an Alt+letter chord from a character Alt (or AltGr)
//!   produced. Repro, headless: a focused single-line `text_input`; `Runtime::key` with
//!   `Key::Text("d")` and `Mods { alt: true, .. }`; the field's text is "d". A fix: insert only
//!   text the platform produced with Alt spent on it (#250's `consumed_mods` would tell), and
//!   leave an Alt chord whose text is the unmodified key to the app. Searched upstream ("alt
//!   letter", "Alt+", "alt typing", "mnemonic", "option key text field"): #250 is the nearest.
//! - Reused: U-C18-1, #328, U-X1-1, U-C6-7, and #203
//!   (a count change in `virtual_rows` reads every row's key; with 5,000 rows a keystroke and
//!   its frame took 1.5 to 3 ms in a release build, test
//!   `a_keystroke_in_a_5000_row_reference_builds_only_the_rows_on_screen`).

mod bar;
mod count;
mod marks;
pub mod reference;

#[cfg(test)]
mod tests;

pub use bar::{
    BUTTON_CHECKED_ALPHA, BUTTON_HOVER_ALPHA, BUTTON_RADIUS, BUTTON_RING, Footer, FooterLabels,
};
pub use count::{
    Count, CountWords, DETAILED_SELECTION, FilterCounts, ListWords, Listing, Plural, Selected,
    SelectionDetails, Template, Template2, decimal_size, grouped, summary,
};
pub use marks::{FilterMark, PILL_BORDER_ALPHA, PILL_FILL_ALPHA, pill};
pub use reference::{ReferenceKeys, ReferenceLabels, ReferenceState, ShortcutReference};
