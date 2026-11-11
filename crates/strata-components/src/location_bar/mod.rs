//! LocationBar (C6): breadcrumbs that switch to an entry with path completion, in the header's
//! middle (`browser/navigation/location-bar` and `.../completion`).
//!
//! Work package: C6, `docs/work/phase-1-leaves/C6-location-bar.md`. Behaviour follows the
//! feature map, against lgse/strata `upstream/main` 35ee677ab (`src/ui/browser/location.rs`,
//! `location/breadcrumbs.rs`, `location/completion.rs`, `view_setup.rs`, `style.css`).
//!
//! # Parts
//!
//! - [`LocationBar`] builds the bar from a [`Trail`] (the crumbs of the folder the host names,
//!   plain data rebuilt each view) and its [`LocationBarState`] (held by the host, reached by
//!   accessor). Its suggestions and hierarchy menu go onto the overlay layer.
//! - Everything it does is a [`LocationIntent`]: open a crumb (click, menu or spring), a new tab
//!   or window, submit typed text, cancelled, list suggestions or stop listing, copy the path,
//!   drops. The host owns every policy, and calls back into the state:
//!
//! ```ignore
//! // Each frame, from the host's poll:
//! let polled = s.loc.tick(now, tx);              // the poll's clock
//! s.route(polled, tx);
//! if let Some(answer) = s.latest.take() {        // rune::Latest, fed by LocationIntent::Complete
//!     let polled = s.loc.deliver(answer);        // a stale generation is dropped
//!     s.route(polled, tx);
//! }
//! // The window's root: forward its activation (U-C6-6).
//! root.on_window_focus(|s, on, tx| { let p = s.loc.set_window_focused(on); s.route(p, tx) })
//! // Ctrl+L, from the host's key map:      s.loc.begin_edit(&s.trail.text)
//! // After any navigation:                 s.loc.navigated()
//! // A typed location that can't open:     s.loc.end_edit()   (then C15's dialog)
//! // One that mounts first:                s.loc.resume_edit(&sanitized)   (from Submit)
//! // A tab hidden mid-edit:                s.loc.dismiss_completion()
//! ```
//!
//! # The crumbs
//!
//! - A sideways viewport (`{key}-crumbs`) holds the list: each ancestor is a button
//!   ([`crumb_key`], a Tab stop that Return and Space press) inside a list item, with the
//!   folder's full path as its description and no tooltip; the current folder is a label in the
//!   accent with the Copy path button. Labels past [`CRUMB_CHARS`] are shortened in the middle
//!   and keep their natural width, so a deep path scrolls rather than squeezes.
//! - A press on a separator, the current label or the empty space after the crumbs opens the
//!   entry; a secondary press anywhere opens the hierarchy menu under the bar at the press's x.
//!   The Menu key or Shift+F10 on a focused crumb opens it too: Rune hands the key to the
//!   strip's secondary-press handler at the crumb's left, so the menu opens there, under the bar.
//!   An ancestor's label is dim, and takes the text colour under the pointer or a drag.
//! - The wheel scrolls the crumbs sideways and the edges fade over hidden crumbs; C24's overflow
//!   bar sits in its own row under them, shown on hover or scroll.
//! - After a navigation (`navigated`, which remounts the crumb row) and whenever the crumbs come
//!   back from the entry, the strip scrolls to its end, so the current crumb shows.
//!
//! # The entry and completion
//!
//! - A combobox group (`{key}-entry`, expanded while suggestions show) around rune-kit's text
//!   input, whose active descendant is the highlighted suggestion, then Navigate and Cancel.
//! - The completion model is Strata's `PathCompletion` (`completion.rs`): each text change asks
//!   for a new generation, the previous list stays until the latest text's list lands, and a
//!   stale answer is dropped. Tab (and each [`LocationBar::complete_keys`]) inserts the
//!   highlighted suggestion, the only one, or a longer common prefix and closes the list, else
//!   highlights the next; Down reopens it. Up, Down, Page Up and Page Down step and wrap. Tab
//!   during a listing, the arrow and paging keys while the list shows, and Enter behind them
//!   wait for the list, then run in order. A click on a suggestion takes it at once, dropping
//!   the listing and any waiting keys, as Strata's `row_activated` does. Ctrl+L while editing
//!   keeps the typed text, selects it and lists it afresh.
//! - The pointer only tints a row: it never moves the keyboard's highlight, which Enter takes.
//!   Rune reports a hover when a row scrolls under a still pointer, so a hover highlight would
//!   follow Down's scrolling (review round 1). Strata's list has none either.
//! - The suggestions hang under the entry at its width. Rows are memoised by their list and
//!   highlight: Rune builds every row's element on each commit, and only the mounted rows lay
//!   out. Strata caps a list at 50; `tests/perf.rs` holds 1,000 to its bars and reports 10,000.
//!
//! # Drops and spring-loading
//!
//! Each ancestor crumb is a W1 drop zone through the host's `Dnd` ([`LocationBar::dnd`],
//! [`LocationBar::drop_action`]), keyed [`crumb_key`] for X2's binder, with Strata's drop look
//! (the highlight tint and an inset accent ring). The current crumb takes no drop. Spring-loading
//! is the bar's own: see `drops.rs`. An accepting hover arms it at the handler's clock and asks for a
//! wake when [`SPRING_DELAY`] (0.75 s, [`LocationBar::spring_delay`]; `None` turns it off) has
//! passed; the [`LocationBarState::tick`] then opens the crumb ([`Via::Spring`]). A leave, a drop or a navigation disarms it; the
//! same hover reported again (X2's binder may, on each motion) keeps its start. The
//! Copy path feedback ([`COPIED_FOR`]) runs the same way. Both hold in a window at rest, which
//! polls only at the wakes it asked for (`tests/c6.rs` steps from wake to wake). C8 and X2 can
//! adopt the model for rows and places.
//!
//! # Coverage
//!
//! Scripts are `tests/scripts/c6/<name>.json`; "test" is in `crates/strata-gallery/tests/c6.rs`;
//! "unit" is in `tests.rs` or `completion/tests.rs`; "golden" is a scene's a11y or PNG golden.
//!
//! `browser/navigation/location-bar`:
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | Ctrl+L replaces the breadcrumbs with an entry… whole text selected | script `ctrl-l-selects-path` (the page binds Ctrl+L; the key map is Strata's); unit `ctrl_l_while_editing_keeps_the_typed_text` |
//! | Clicking the current crumb, a separator, or empty breadcrumb space opens the entry… | scripts `edit-from-current`, `edit-from-separator`, `edit-from-space`; crumbs and Copy path keep their own: `crumb-click-opens`, `copy-path-feedback` |
//! | While editing, a click outside… cancels… and still performs the click's normal action | script `click-outside-cancels`; unit `a_press_elsewhere_cancels_on_the_next_tick_only_after_the_entry_had_focus` |
//! | Enter or the Navigate button opens…; Escape or the Cancel button restores… | scripts `enter-navigates`, `navigate-button`, `cancel-button`, `escape-restores-folder` |
//! | Escape in the entry restores the folder the breadcrumbs name | script `escape-restores-folder` |
//! | With an input method composing…, Escape cancels only the composition | test `escape_while_composing_keeps_the_entry_open` (scripts have no IME event: U-C6-7) |
//! | The entry and its buttons have the accessible names… | golden `c6-editing` (a11y); names are [`LocationLabels`], the host's to follow its key map |
//! | While the entry has focus, Ctrl+K and Ctrl+1–3 reach the entry… | **Strata** (key dispatch): [`LocationBarState::entry_focused`] tells the host |
//! | A location that cannot be opened shows an "Unable to open location" dialog… | **partial**: [`LocationBarState::end_edit`] here; the dialog (C15) and validation are Strata's |
//! | Escape cancels the edit even while suggestions are showing… | script `escape-closes-suggestions`; test `escape_leaves_nothing_focused` |
//! | Closing and reopening the editor shows no stale or flashing suggestion list | test `a_reopened_entry_never_shows_the_old_list` (every frame) |
//! | A folder load that ends while the entry is open and focused… | script `load-keeps-entry`; unit `navigating_keeps_an_entry_being_typed_in` |
//! | Typed paths (28 bullets) | **Strata policy**: the bar submits the raw text; [`LocationBarState::resume_edit`] keeps the entry on a sanitised URI while it mounts |
//! | Each ancestor crumb is a button that opens that folder | scripts `crumb-click-opens`, `crumb-return-opens` |
//! | Clicking an ancestor crumb… selects the folder on the path you came from | **Strata** (navigation): [`Via`] tells it a click or the menu from a spring |
//! | Right-clicking the breadcrumb bar opens a menu of the current folder and its ancestors… | scripts `hierarchy-menu`, `hierarchy-menu-opens` |
//! | The hierarchy menu has no arrow. It opens below the crumb bar at the click's x… | scripts `hierarchy-menu`, `hierarchy-menu-opens` open it with a real secondary press (its placement there isn't asserted: geometry); golden `c6-menu` (PNG) draws it opened where such a press opens it, which the scene sets, since scenes can't press |
//! | The hierarchy menu marks as current the folder the bar names | script `hierarchy-menu`; golden `c6-menu` (a11y: checked) |
//! | In Columns…; Switching from Columns…; Switching back… (3) | **Strata**: which folder it passes as the trail |
//! | Middle-clicking an ancestor crumb… opens it in a new, active tab | script `middle-click-new-tab` (the intent; tabs are Strata's and C7's) |
//! | Ctrl+middle-click or Shift+middle-click… opens that folder in a new window | scripts `ctrl-middle-new-window`, `shift-middle-new-window` (a script holds the modifier with `{"hold": "ctrl"}`) |
//! | In a file chooser, middle-clicking a crumb opens nothing | test `a_chooser_s_middle_click_opens_nothing` ([`LocationBar::open_in_new`]) |
//! | Middle-clicking the current crumb opens no tab | script `middle-click-new-tab` |
//! | Parent breadcrumbs accept file drops; Dropping… before 750 ms transfers… without navigating | script `drop-on-crumb` |
//! | Holding a file drag over a parent breadcrumb for 750 ms navigates there, and the drag stays held | script `spring-open`; test `the_spring_opens_in_a_resting_window`; units `a_resting_drag_springs_once_after_its_delay`, `a_repeated_hover_on_the_same_crumb_keeps_the_spring_s_start` |
//! | A breadcrumb under a file drag shows an accent tint and an accent inset ring | golden `c6-drop-hover` (PNG) |
//! | For paths under home, the crumbs and the hierarchy menu start at `~` | **Strata** (its trail); the gallery's trail shows it |
//! | When crumbs overflow, the wheel scrolls them horizontally and edge fades mark the hidden part | test `the_wheel_scrolls_overflowing_crumbs`; golden `c6-deep` (PNG, the fades) |
//! | The overflow scrollbar sits in its own row below the crumbs, hidden at rest… | test `the_overflow_bar_shows_on_hover_and_scroll_only`; golden `c6-deep` (PNG, where it sits) |
//! | Crumb labels longer than 32 characters are shortened in the middle | unit `long_labels_shorten_in_the_middle`; golden `c6-deep` |
//! | Deep paths keep each crumb label at its text width… | golden `c6-deep` (PNG) |
//! | After navigation, or after submitting or cancelling the entry, the crumbs scroll to show the current crumb | test `the_current_crumb_shows_after_a_navigation_a_submit_and_a_cancel` |
//! | The current crumb's Copy path button copies the path and shows a check and "Path copied" for 2 seconds | scripts `copy-path-feedback`, `copy-path-resets`; test `the_copy_feedback_ends_in_a_resting_window`; unit `a_second_copy_restarts_the_feedback`; golden `c6-copied` (the check) (the copy itself is the host's, on [`LocationIntent::CopyPath`]) |
//! | The Copy path button is named "Copy path"; after a click… "Path copied", then "Copy path" again 2 s later | the same |
//!
//! `browser/navigation/location-bar/completion`:
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | While editing, a popover under the entry, matching its width, lists child folders… | **partial**: script `completion-lists`, golden `c6-completion`; what it lists is Strata's planner |
//! | A relative prefix…; Typing only `~`…; When the typed text names an existing folder…; Hidden folders… | **Strata** (its planner; the gallery fakes it) |
//! | Each suggestion shows a folder icon, the name with the typed prefix highlighted, and its parent folder… | **partial**: golden `c6-completion` draws [`Candidate`]'s icon, matched ranges and hint; their content is Strata's |
//! | Down and Up move through suggestions, wrapping…, and Page Down and Page Up move by 5 | script `arrows-wrap-and-page`; unit `arrows_wrap_and_paging_moves_five`; test `a_resting_pointer_never_moves_the_highlight`; test `down_while_composing_moves_no_highlight` |
//! | Page Down and Page Up still move by 5 after `page_down` or `page_up` is rebound… | by design: the bar reads the keys, not the host's key map |
//! | Tab inserts the highlighted suggestion, the only suggestion, or a longer common prefix and closes the popover; Down reopens it | scripts `tab-inserts-highlighted`, `tab-inserts-only`, `tab-common-prefix-down-reopens`; unit `tab_takes_the_common_prefix_then_cycles` |
//! | A key added to `focus_next` or `focus_previous`… completes the path as Tab does | script `complete-key` (the page passes F7 to [`LocationBar::complete_keys`]) |
//! | When Tab has nothing longer to insert, it highlights the next suggestion instead | script `tab-cycles` |
//! | Enter on a highlighted suggestion opens that folder | script `enter-on-suggestion`; test `a_resting_pointer_never_moves_the_highlight` |
//! | The popover does not appear while the breadcrumbs are showing | script `no-popover-on-crumbs`; unit `a_late_or_stale_answer_is_dropped` |
//! | While the typed folder is being listed, the previous suggestions stay… | scripts `previous-stay-while-listing`, `stale-result-dropped`; unit `an_answer_for_another_generation_or_text_is_dropped` |
//! | Tab pressed during the listing waits for its list… they then run in order | scripts `deferred-keys`, `deferred-down-then-enter`; units `keys_during_a_listing_wait_and_run_in_order`, `a_tab_that_relists_carries_the_rest_to_the_new_listing` |
//! | Switching to another window, minimising, or hiding Strata while suggestions show closes the popover… | test `losing_the_window_closes_the_suggestions_and_keeps_the_edit` (on a host whose root hears activation) |
//! | A listing still pending when the window loses focus is cancelled… | test `a_listing_pending_at_deactivation_is_cancelled_and_refocus_reopens_nothing` |
//! | Editing the entry text while the window is not focused shows no suggestions | test `editing_while_the_window_is_inactive_asks_for_nothing` |
//! | Refocusing the window does not reopen suggestions; the next text change shows them again | tests `refocusing_keeps_a_list_closed_by_deactivation_closed`, `a_listing_pending_at_deactivation_is_cancelled_and_refocus_reopens_nothing` |
//! | Hiding the location entry closes the popover and cancels a pending listing | **partial**: test `dismissing_closes_the_list_and_cancels_the_listing`; when it hides is Strata's |
//! | Switching tabs closes the previous tab's suggestions…; In the portal file chooser, focusing another window… | **Strata** (one state per tab; the chooser host forwards its window's activation) |
//!
//! A click on a suggestion opens it at once (test
//! `a_kept_suggestion_row_reports_through_the_latest_handler`, unit
//! `a_click_takes_its_suggestion_even_while_keys_wait`); no bullet names it. A hovered
//! crumb takes the text colour (test `a_hovered_crumb_takes_the_text_colour`), as Strata's
//! `.breadcrumb:hover` does; the Menu key and Shift+F10 open the hierarchy menu from a focused
//! crumb (test `the_menu_key_on_a_crumb_opens_the_hierarchy_menu`); neither has a bullet.
//!
//! Also `app/accessibility`: crumbs carry no tooltip, and a crumb's full path is its accessible
//! description (golden `c6-short`, a11y). A paste asks for suggestions as typing does (test
//! `a_paste_lists_suggestions`).
//!
//! # Deliberate changes
//!
//! - **Its own suggestion list**, on rune-kit's public `popover::anchored`, `popover::raised`
//!   and virtual rows, not rune-kit's `PopupList` as plan §4 names: a `PopupItem` can't draw
//!   matched ranges (U-C6-4). The owner's ruling.
//! - **Matches are coloured, not bold** (W1's `MatchedText`, so a label never reflows as the
//!   query changes).
//! - **The current crumb is weight 500** (`kit.ty.ui_medium` in `text_accent`), not Strata's
//!   700: no slot token for it (owner's ruling).
//! - **The hierarchy menu marks the current folder with rune-kit's check** (assistive
//!   technology hears "checked"), not accent bold text. One menu for the whole bar, opened by a
//!   secondary press, as Strata does, rather than one per crumb as the work package words it.
//! - **The entry is a combobox group around a text box**, since Rune makes every input a text
//!   box (U-C6-1).
//! - **What assistive technology hears twice.** A crumb's list item and its button both read
//!   the crumb's name: Rune names an unlabelled list item from its content, so leaving its
//!   label out changes nothing. The combobox group and its text box both read "Location
//!   (Ctrl+L)": the group needs a name of its own (Rune names containers by their label alone),
//!   and the text box's name is what a reader says as it enters it (U-C6-1 would make them one
//!   node). Buttons are the header's actions, named by their tooltip with no description.
//! - **The hierarchy menu has no name, and long names read shortened.** rune-kit's `Menu` takes
//!   no accessible name, and a `MenuItem`'s name is the text it draws, which the bar shortens
//!   in the middle past [`CRUMB_CHARS`] as Strata draws it (U-C6-8).
//! - **Navigate and Cancel are pointer-only**: pressing a focusable button would take the
//!   keyboard from the entry, and nothing tells the bar where it went (U-C6-3), so the edit
//!   would cancel. Enter and Escape do the same from the keyboard. Strata's are Tab stops.
//! - **The Copy path button always shows**; Strata reveals it while the pointer is over the
//!   current crumb.
//! - **A press outside cancels one frame later**, through `tick`, which sees the entry lost the
//!   keyboard (U-C6-3). It isn't visible.
//! - **The host forwards the window's activation** (U-C6-6).
//! - **Spring-loading lives here**, where C8 leaves it to its host (owner's ruling).
//! - **The 32-character cap counts characters**, not GTK's average character width.
//! - **No crossfade between crumbs and entry**: they swap at once, where GTK fades for 100 ms.
//!
//! # What stays in Strata
//!
//! Parsing, resolving and validating typed locations; credentials and mounting; listing
//! suggestions (prefix rules, `~`, relative paths, hidden folders, the 50 cap) and feeding them
//! through `rune::Latest`; which folder the bar names (Columns' mirrored child) and the trail
//! starting at `~`; navigation (came-from selection) and the "Unable to open location" dialog;
//! tabs and windows; the clipboard; transfers and their action; the key map (Ctrl+L, the
//! shortcuts the entry keeps from the view, `focus_next`); window activation.
//!
//! # Rune gaps
//!
//! Drafts, not filed (the owner files). Headless repros are in `strata-spikes/c6-probes`
//! (`RESULTS.md`).
//!
//! - **U-C6-1 (draft, not filed)** An editable combobox can't be expressed: `a11y/roles.rs` maps
//!   any input to a text box (or search box) and ignores `role("combobox")`, so ARIA 1.2's
//!   pattern (the role, `expanded` and `controls` on the input) isn't possible. Here: a
//!   `combobox` group around the input, expanded while the list shows (probe P4).
//! - **U-C6-2 (draft, not filed)** No `aria-current`: the current crumb and the menu's current
//!   item can't say so. Here: the current crumb is a label last in the list; the menu item is
//!   checked.
//! - **U-C6-3 (draft, not filed; upstream #106 asks the same)** A text field has no focus-change
//!   hook: the runtime clears `TextEdit::focused` without telling anyone (probe P5). Here:
//!   [`LocationBarState::tick`] notices, a frame late.
//! - **U-C6-4 (draft, not filed)** `PopupItem` has no rich label (spans, or `label_with(El)`),
//!   and `PopupList::build_list` is private. Here: the bar's own list.
//! - **U-C6-5 (draft, not filed)** A vertical mouse wheel over a sideways-only viewport scrolls
//!   nothing (probe P3); GTK and AppKit turn it sideways there. Here: `on_wheel` maps it, with
//!   C24's `ScrollFeed` for the offset.
//! - **U-C6-6 (draft, not filed)** `on_window_focus` is heard only on the app's root, so a
//!   component can't hear its window activate, and a gallery page can't either. Here: the host
//!   forwards it. Scripts *can* deactivate a window, though: `{"t": .., "focus": false}`
//!   reaches the root's `on_window_focus` (probe P2b), so X1's U-X1-1 is wrong for a root
//!   handler.
//! - **U-C6-7 (draft, not filed)** Headless scripts have no input-method event, so a
//!   composition is testable only in process (`Runtime::ime`).
//! - **U-C6-8 (draft, not filed)** rune-kit's `Menu` takes no accessible name (`.label`), and a
//!   `MenuItem` can't give an accessible name apart from the text it draws, so an item shortened
//!   to fit reads shortened. Here: the menu reads unnamed, and its items read as drawn.
//! - **A right-click on the entry (since Rune 5d421fd).** The text field has no
//!   `on_secondary_press` of its own, so Rune asks for its native field menu (Cut, Copy, Paste,
//!   Select All; `--trace` logs `popup-menu:open`). `popup_menu` isn't supported on Linux, so
//!   it answers None and nothing shows; the entry stays open with its selection. Strata's GTK
//!   entry shows its menu there. An in-app field menu is C21's to give (known; probe in
//!   `strata-spikes/c6-probes/r1/w4/rclick.*`).
//! - Reused: U3 (a drag's paths only at the drop). U-C1b-1 (script pointer events dropped
//!   modifiers, probe P6b at 78d22ed) is fixed at the current pin, 5d421fd (4805095): scripts hold
//!   them with `{"hold": "ctrl"}`.

mod build;
mod completion;
mod crumbs;
mod data;
mod drops;
mod entry;
mod intent;
mod menu;
mod popup;
mod state;

#[cfg(test)]
mod tests;

pub use build::{Access, LocationBar, crumb_key, field_key, option_key};
pub use data::{
    CRUMB_CHARS, Candidate, CompletionQuery, Completions, Crumb, LocationLabels, Trail,
    elide_middle,
};
pub use intent::{LocationIntent, Via};
pub use state::{COPIED_FOR, LocationBarState, Polled, SPRING_DELAY};
