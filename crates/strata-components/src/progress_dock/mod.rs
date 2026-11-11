//! ProgressDock (C16): running operations as cards in the bottom-right dock, brief notices
//! beside them, the blocking progress dialog, and the Jobs dashboard's footer element
//! (`operations/progress`, `.../dock`, `.../jobs`). The component owns no operation:
//! operations arrive as data, and Cancel is an intent.
//!
//! Work package: C16, `docs/work/phase-2-composites/C16-progress-dock.md`.
//! Behaviour follows lgse/strata `upstream/main`: `src/ui/progress_dock.rs`,
//! `src/ui/progress_dock/completion.rs`, `src/ui/notice.rs`, `src/ui/browser/progress/*`,
//! `src/ui/jobs.rs`, `src/i18n.rs` and `src/style.css`.
//!
//! **Rune dev 5d421fd: partly.** `Live`, `LiveText` and `Tween` draw the bars and live labels
//! without running the view; `El::on_layout_tx` with `Tx::live_only` gives the track's size
//! without a commit; `role("status")` is a polite live region (`a11y/node.rs:181`); progress
//! bars take range values; `El::modal` with `focus_return`, `popover::anchored` and
//! `ScrollSpec { stick_bottom }` make the jobs list; C15's `StrataDialog` makes the blocking
//! dialog. rune-kit's ToastRegion, Toast and Card are too fixed for the dock (below); its
//! Progress draws the jobs rows. Missing: an accessibility value that follows a `Live`
//! (U-C16-3), a fill width as a live fraction (U-C16-2), a callback when focus enters or
//! leaves an element (U-C16-4). Not adopted: rune-kit's `ToastStack` (1633f8d6), Sonner's
//! stack that merges toasts by key and pauses every timer while expanded, where Strata pauses
//! a card and replaces notices by its own rule; and `Tx::clock` (c650fe0c), which could retire
//! the stamp-at-the-next-tick pattern below once C15's dialog buttons carry a `Tx`.
//!
//! # Hosting
//!
//! ```ignore
//! // State: s.dock: Dock, s.jobs_ui: JobsState, s.jobs: Vec<JobRow> (from the job service).
//! // Starting an operation (from a handler):
//! s.dock.start(op.id, OpSpec { placement, reveal_after: reveal_after(Some(n)), .. }, tx);
//! // Each poll, with the host's clock:
//! for event in s.operation_events.drain(..) {
//!     match event {
//!         Progress(id, sample) => changed |= s.dock.progress(id, sample, now, tx),
//!         Done(id) => { s.dock.complete(id, "Copy complete", tx); }
//!         Cancelled(id, counts) => { s.dock.remove(id); s.open_cancelled_dialog(counts); }
//!         Failed(id, error) => { s.dock.remove(id); s.open_error_dialog(error); }
//!     }
//! }
//! changed |= s.dock.tick(now, tx);
//! changed |= s.jobs_ui.tick(now, &s.jobs, tx);
//! // The view:
//! let mut layer = Vec::new();
//! let jobs = Jobs::new("jobs", |s: &App| &s.jobs_ui, |s: &mut App| &mut s.jobs_ui, |s: &App| &s.jobs)
//!     .on_intent(|s: &mut App, intent, tx| s.job_service.handle(intent, tx))
//!     .build(s, &s.look, &mut layer);
//! let footer = Footer::new("footer") /* … */ .jobs(jobs).build(s, &s.look); // C19
//! DockView::new("dock", |s: &App| &s.dock, |s: &mut App| &mut s.dock)
//!     .on_intent(|s: &mut App, intent, tx| s.operations.handle(intent, tx))
//!     .build(s, &s.look, &mut layer);
//! if let Some(id) = s.blocking_op {
//!     ProgressDialog::new("progress", |s: &App| &s.dock, |s: &mut App| &mut s.dock, id)
//!         .return_focus(cursor_row_key)
//!         .on_intent(|s: &mut App, intent| s.operations.handle_untimed(intent))
//!         .build(s, &s.look, &mut layer);
//! }
//! ```
//!
//! Which operations dock and which hold the dialog, the titles ("Copying items", "Copy
//! complete"), the destination words ("→ Trash", "Deletion without Trash"), and what follows a
//! cancel or a failure are Strata's. Words are data: [`Words`] and
//! [`JobsWords`](jobs::JobsWords) default to Strata's English, and [`format`](mod@format) writes sizes,
//! rates and durations as Strata's `i18n` does ("64 MB", "1.5 MB/s", "2m 5s").
//!
//! # The dock
//!
//! - Cards and notices stack in the order they appeared, in a region pinned bottom-right
//!   (`space.lg` from the right, [`DOCK_BOTTOM`] from the bottom) that scrolls past
//!   [`DOCK_MAX_HEIGHT`]. It is no modal and draws no backdrop, so the window stays usable; an
//!   empty dock isn't built.
//! - A card: icon, title, X; the current file (or the spec's info) with "3/10" files; the
//!   destination; the bar; the speed or status at the left of the footer and the percentage
//!   once, at its right. A pulsing bar reads "…". Cards are 280 to 340 pt wide
//!   ([`CARD_MIN_WIDTH`], [`CARD_MAX_WIDTH`]: Strata's `min-width: 280px` and 42-character
//!   labels).
//! - X on a running card reads "Cancelling transfer…" (with the device warning in place of the
//!   file) or "Cancelling operation…", disables itself and sends [`DockIntent::Cancel`]. A
//!   transfer that hasn't stopped [`STALL_AFTER`] seconds later reads "Device not responding".
//! - Ending: [`Dock::complete`] gives a check, 100 %, Complete and the pin, and a
//!   [`COMPLETE_FOR`] countdown with a shrinking bar and "closing in 4s". The pointer over the
//!   card pauses it ("paused"), the pin keeps it ("pinned"); leaving resumes the time left. The
//!   dock tracks the pointer over a card in every phase, so a card that ends under a resting
//!   pointer starts its countdown paused, as Strata's does (Rune reports a hover only when it
//!   changes). At zero it fades for [`FADE`] (none with animations off), then goes. Complete
//!   or X dismisses it at once ([`DockIntent::Dismissed`]); a second ending is ignored.
//! - **Cancel's end.** Strata's path, which the gallery shows: the host removes the card
//!   ([`Dock::remove`]) and opens its "Operation cancelled" dialog with the counts (C15). The
//!   WP's path, [`Dock::close_as_notice`], turns the card into a notice that closes itself
//!   (review scene `c16-cancel-notice`). Both are supported; the WP asked for the second.
//! - Notices ([`Notice`]): a check (or none, for a status notice such as "Text size: 16px"),
//!   a bold heading and text cut to three lines; no controls, and a press does nothing. A new
//!   one replaces the notices showing unless it [`joins`](Notice::joins) (a transfer report);
//!   cards stay. Each goes after its `shown_for` ([`NOTICE_FOR`] by default).
//! - The reveal: an operation's `reveal_after` ([`reveal_after`]: 350 ms under 16 items, else
//!   none) holds it back, and it never shows if it ends first.
//!
//! # The progress dialog
//!
//! [`ProgressDialog`] is C15's [`StrataDialog`](crate::dialog::StrataDialog) without a close
//! button (`closable(false)`, a sanctioned C15 addition), blocking (a press outside leaves it),
//! with Cancel focused first. It shows the icon, title and subtitle, "Transferred" with the
//! bytes and percentage, the bar, "5 of 30 files · name" and the speed with the time left.
//! Cancel turns into "Cancellation requested", disabled; the title reads "Cancelling
//! transfer…" and then "Device not responding". [`StrataDialog::return_focus`](crate::dialog::StrataDialog::return_focus)
//! sends focus to the host's cursor row when it closes, and a dialog the host chains on it
//! keeps its own focus (C15).
//!
//! **Escape cancels** ([`EscapeDoes::Cancel`]), as the feature map's bullet ("Escape in the
//! progress dialog acts as Cancel") and Strata's code (`install_progress_keys`) say (OWNER
//! ruling). Two sources disagree and are left as they are: Strata's QA probe ("Press Escape on
//! the progress dialog mid-operation; the operation must keep running and stay reachable from
//! the dock") and C15's docs ("C16 docks its progress dialog"). [`EscapeDoes::Nothing`] leaves
//! Escape to the host.
//!
//! # The Jobs element
//!
//! [`Jobs`](jobs::Jobs) returns the footer button, or `None` with no jobs: "2 jobs running ·
//! 1 queued", else "3 jobs finished", with "· failures". It is `frame.control` tall in C19's
//! F1 look (constants copied from C19's `footer::BUTTON_*` until C19 shares its button), so it
//! sits in C19's footer (`frame.control + 2·space.xs` tall, centred) as the F1 button does,
//! through `Footer::jobs`. While open, its list hangs above it, aligned to its end
//! ([`LIST_WIDTH`](jobs::LIST_WIDTH) by at most [`LIST_MAX_HEIGHT`](jobs::LIST_MAX_HEIGHT)):
//! a header with the summary, Clear finished and Minimize, then at most
//! [`MAX_ROWS`](jobs::MAX_ROWS) rows and "N older jobs are hidden". A row: icon, name, status
//! with the elapsed time, a check or a cross, Cancel or Remove, Details or Hide, Dismiss; the
//! meta line; a bar while active (pulsing without a total); Details shows the output with
//! terminal sequences stripped, or "No output yet.", following its end. Escape, a press
//! outside and Minimize hide it; no job is cancelled. The buttons send
//! [`JobsIntent`](jobs::JobsIntent)s. While open, the button shows C19's checked tint.
//!
//! Clear finished on a list of finished jobs empties it: the list stays open on "No background
//! jobs." (Rune moves focus from the vanished Clear finished to Minimize), and the button stays
//! while it is open, reading "Jobs". Closing it then sends focus to
//! [`Jobs::return_focus`](jobs::Jobs::return_focus), the host's fallback (Strata's file list cursor
//! row): the button goes with the last job, so it can't take focus back; a `focus_return` covers
//! every way the list goes. A closed list stays closed when a job comes later.
//!
//! # Live values and commits
//!
//! Every sample goes to the card's [`CardLive`] at once: the fill (`live_rect`, its width from
//! the track's `on_layout_tx`), the percentage, the count, the speed or status and the
//! dialog's bytes (`LiveText`, each in a box its widest words reserve). What is committed (the
//! file line, the accessibility values and descriptions) follows at most every
//! [`COMMIT_EVERY`] (0.25 s, OWNER ruling), dock-wide, and at once when a card changes shape or
//! phase. A held change asks for a wake, so it lands without another sample. Twenty cards whose
//! counters move every frame commit 4 times a second (`tests/c16.rs`).
//!
//! # Timers
//!
//! Every timer runs from [`Dock::tick`] or [`JobsState::tick`](jobs::JobsState::tick): the
//! reveal, the stall, the countdowns and fade, the notices, held commits and the jobs list's
//! seconds. A call from a handler stamps its time at the next tick and asks for a wake; each
//! tick asks for the next one due, so a resting window polls only then. C15's buttons and
//! Escape carry no `Tx`, and Rune runs no poll on its own after the commit such a handler
//! makes: a cancel from the dialog ([`Dock::cancel_untimed`]) would wait for whatever moved
//! next. So until the next tick the dialog mounts a zero-size wake beacon whose
//! `on_layout_tx` asks for the wake when that commit lays it out; layout runs whatever the
//! window's motion does, so the stall starts on time with animations off and under
//! `Cx::time_scale(0)` too. `tests/c16.rs` steps each timer from wake to wake, with no
//! frames between, with animations on and off.
//!
//! # What assistive technology hears
//!
//! - The dock: a region named "Operations". A card: a group named by its title and described
//!   by the destination's path; X named by the host ("Cancel copying", disabled while
//!   cancelling) or "Dismiss notification"; the bar, a progress indicator named by the title
//!   with its value ("42%") and the speed or status as its description (the dialog's adds the
//!   bytes), updated at most four times a second; a hidden polite status that says the phase
//!   when it changes ("Copy complete", "Cancelling transfer…"), never the percentages. A
//!   finished card: the pin "Keep notification" with its checked state, Complete "Dismiss
//!   completed notification", and the bar described "Time remaining before notification
//!   closes". The live labels are hidden: the bar says what they show.
//! - A notice: a polite status named by its heading or text, described by the text under a
//!   heading.
//! - The dialog: C15's modal dialog, named by the title and described by the subtitle, with
//!   the bar as above.
//! - Jobs: a button with its expanded state, described "Show background jobs"; a modal
//!   dialog "Jobs"; a group per row, named by the job and described by its status and meta
//!   line, with its buttons and bar.
//!
//! # Tokens
//!
//! Kit and Look only: the card's and notice's fill `layer_01`, border `border_base` (Strata's
//! border at 60 %), focused-within ring `text_accent`; the glow from `look.glow` scaled to
//! Strata's `0 0 10px alpha(glow, 0.18)`, else the kit's overlay shadow; titles `ty.ui` at 700,
//! meta `ty.small` in `text_muted`, the percentage in `text_accent`; the trough
//! `alpha(bg_base, 0.72)` with `alpha(std_border, 0.42)`, the fill `border_active`; spacing
//! from `space`. Strata's metrics are constants named after its CSS; a `progress` slot file is a
//! later W2 follow-up (OWNER ruling).
//!
//! # Differences from the WP and from Strata
//!
//! - **Built on Rune primitives**, not rune-kit's ToastRegion, Toast and Card: at the pin
//!   ToastRegion fixes its key, name, insets and width and has no height cap or scroll; Toast
//!   fixes its 320 pt width, radius and padding; Card has a left accent rule and no header
//!   actions; none binds a `Live` value. rune-kit's Progress draws the jobs rows (they change
//!   coarsely). C15's precedent.
//! - **No bytes on cards** (OWNER ruling): Strata's card shows the speed, not the bytes; the
//!   dialog shows them. The WP asked for them on the card.
//! - **Cancel's end** follows Strata by default (above); the WP's notice is an option.
//! - **Committed text at most 4 times a second** (OWNER ruling); Strata renders at 33 ms. The bars
//!   and live labels follow every sample.
//! - **Focus doesn't pause a countdown**: Rune has no way to tell a card that keyboard focus
//!   came in (U-C16-4; `hover_group_focus` draws the hover but `on_hover_change` doesn't hear
//!   it). Hover and the pin pause it; the card's border shows focus within.
//! - **Check notices are a card wide** (340 pt): a wrapping text in a row without a width lays
//!   out at its narrowest in Rune, so a notice can't size to its words up to Strata's 60
//!   characters. A status notice is one line and fits its words.
//! - **The Jobs button with no jobs.** Clear finished on a list of finished jobs leaves the list
//!   open on "No background jobs.", and its button stays, reading "Jobs", until the list closes.
//!   Strata hides the button as soon as no job is left (its popover goes with it). The list
//!   needs its anchor while it shows, and closing it sends focus to
//!   [`Jobs::return_focus`](jobs::Jobs::return_focus) rather than nowhere.
//!
//! # Coverage
//!
//! Scripts are `tests/scripts/c16/<name>.json`; "test" is in `crates/strata-gallery/tests/c16.rs`;
//! "unit" is in `tests.rs`; "golden" is a scene's a11y or PNG golden.
//!
//! `operations/progress/dock`:
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | Copying, compressing, and deleting … show a card … the window stays usable | script `window-usable`; which operations dock is **Strata policy** ([`Placement`]) |
//! | Moving, extracting, restoring, emptying Trash, and undo or redo keep the blocking progress dialog | **Strata policy** ([`Placement::Dialog`]); script `delay-none` shows the dialog |
//! | A card shows the operation title, percentage, current file …, destination, speed or status, and a progress bar | script `card-updates`; golden `c16-card` |
//! | A card shows its percentage once, at the right of its footer | golden `c16-card` (PNG); script `card-updates` |
//! | The footer's left label shows a copy's speed, a compression's count …, or a status | unit `a_transfer_card_shows_percentage_speed_and_time_left_and_counts_files`; golden `c16-stack` |
//! | During a deletion, the footer's left label is empty, except … | unit `a_one_item_delete_counts_and_pulses_and_a_larger_one_shows_its_percentage`; script `deletion` |
//! | During a one-item permanent local delete, the left label shows the count | script `deletion`; golden `c16-stack` |
//! | While that count pulses the bar, the footer's right label reads "…" | unit (as above); golden `c16-stack` (PNG) |
//! | A docked copy also shows files done of total; a compression card names the archive | script `card-updates`; golden `c16-stack` |
//! | The destination reads "→ `<path>`" … | **Strata policy** (the words are the spec's); golden `c16-card` |
//! | A permanent deletion of a remote selection reads "Deletion without Trash" | **Strata policy** |
//! | Long file names, statuses, and destinations wrap | golden `c16-cancelling` (PNG: the warning wraps) |
//! | Several cards stack and update independently; the stack scrolls once it reaches 320 px | script `stack` (copies, compressions and deletes started apart); review scene `c16-many`; test `scrolling_the_dock_past_counting_cards_commits_only_for_their_seconds` |
//! | A docked card does not blur or unblur the window | script `window-usable` (no backdrop) |
//! | A card's X cancels only that card's operation | script `cancel-one` |
//! | After X, the card retitles … and X is disabled | scripts `cancel-one`, `cancel-operation` |
//! | Cancelling a docked transfer shows "Device may still be writing…" inside the card | script `cancel-one` |
//! | When a docked operation is cancelled, its card closes and an "Operation cancelled" dialog … | script `cancel-closes` (Strata's path, the gallery's host); script `cancel-notice` (the WP's path) |
//! | When a docked operation fails, its card closes and an "Unable to complete operation" dialog … | **Strata policy** ([`Dock::remove`] and a C15 dialog, as `cancel-closes`) |
//! | Choosing Delete Permanently after a docked deletion partly fails … "Another operation is active" | **Strata policy** |
//! | Closing a tab with a docked operation running … "File operations are still active" | **Strata policy** |
//! | A finished copy, compression, or deletion retitles its card … check icon, 100%, and a Complete button | script `card-completes`, `deletion`; unit `a_completed_card_closes_after_five_seconds_from_wake_to_wake` |
//! | A completed card counts down 5 seconds with a shrinking bar and "closing in" text, then fades out and closes | test `a_completed_card_closes_after_five_seconds_in_a_resting_window`; script `countdown`; golden `c16-complete` |
//! | Hovering or focusing a completed card pauses its countdown … leaving resumes | **partial**: hover by tests `hover_pauses_the_countdown_and_leaving_resumes_the_rest`, `a_card_that_finishes_under_a_resting_pointer_stays_paused`, scripts `countdown-pause`, `countdown-resume`, `countdown-resume-kept` (closed by 8 s, where a restart of the full 5 s would still show); focus is a **Rune gap** (U-C16-4) |
//! | The pin button, "Keep notification", keeps a completed card open and shows "pinned" | tests `a_pinned_card_stays_until_dismissed`, `unpinning_resumes_the_countdown_in_a_resting_window`; script `pin` |
//! | Complete or X on a completed card dismisses only that card | scripts `dismiss-one`, `dismiss-x`, `pin` |
//! | A docked transfer reports its renamed and left-out items once, after it ends | **Strata policy** (the report is a joining [`Notice`], script `notices`) |
//! | If another operation starts before a docked copy … does not reveal or select its result | **Strata policy** |
//! | A notice shows as a row in the bottom-right dock; notices and progress cards stack in the order they appear | script `notices`; golden `c16-notices` |
//! | A notice shows a check icon before its text and has no controls; a click on it does nothing | script `notices`; golden `c16-notices` |
//! | A size notice … has no check icon and no minimum width | golden `c16-notices` ([`Notice::status`]) |
//! | Showing a notice while another notice is visible … removes the old one; progress cards stay | script `notices`; unit `a_notice_replaces_the_notices_showing_unless_it_joins_and_cards_stay` |
//! | A transfer report notice is the exception: it joins the dock | script `notices` |
//! | Notice text, not its heading, wraps and is cut to 3 lines with an ellipsis | golden `c16-notices` (PNG) |
//! | When a notice expires and the dock holds no other row, the dock closes | tests `a_notice_expires_from_its_own_wake_and_the_dock_closes`, unit `a_notice_shows_for_its_own_time`; script `notices-expire` |
//!
//! `operations/progress`:
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | An operation on fewer than 16 items shows no progress dialog for its first 350 ms, and none … if it finishes sooner | test `a_small_operation_shows_after_350_ms_from_its_own_wake_and_a_fast_one_never`; scripts `delay-fast`, `delay-slow`; the count rule is [`reveal_after`] |
//! | An operation on 16 or more items, or with an unknown item count, shows … immediately | script `delay-none`; unit `a_small_operation_waits_350_ms_and_never_shows_if_it_ends_first` |
//! | The progress dialog's subtitle reads "Cancelling will not undo completed changes" | script `dialog-blocking`; golden `c16-dialog` (the words are the host's) |
//! | Clicking outside the progress dialog leaves the dialog, its progress, and Cancel visible | script `dialog-blocking` |
//! | Escape in the progress dialog acts as Cancel | script `dialog-escape` |
//! | When the progress dialog closes, keyboard focus goes to the file list's cursor row | script `dialog-focus-return` (`return_focus`; which row is Strata's) |
//! | An error dialog that opens as the progress dialog closes keeps its own focus, with Close focused | script `dialog-chained` |
//! | A transfer whose item total is not yet known shows "Preparing items…" | script `transfer-preparing`; unit `transfer_words_follow_what_is_known` |
//! | During a copy or move with a known byte total, the bar advances as bytes are written | test `the_fill_and_labels_move_every_frame_and_commits_stay_throttled`; scripts `card-updates`, `transfer-dialog` |
//! | When the byte total is unknown, the bar pulses and reads "Preparing…" until bytes move, then "Transferring…" | unit `transfer_words_follow_what_is_known`; script `transfer-preparing` |
//! | A tree of only empty files or folders advances by item count … | unit `transfer_words_follow_what_is_known` |
//! | The transfer view shows the percentage, transferred and total bytes, "N of M files" …, and the speed with time left | script `transfer-dialog`; golden `c16-dialog` |
//! | Until a speed is measured, the speed field reads "Calculating speed…" | unit `a_transfer_card_shows_percentage_speed_and_time_left_and_counts_files` |
//! | A copy onto removable media shows "Writing to device…" and is not reported done until the flush returns | **partial**: the words and the pulse by unit `transfer_words_follow_what_is_known` ([`Sample::busy`]); when it is done is **Strata policy** |
//! | A move that copies shows "Writing to device…" during its final flush … | **partial**, as above |
//! | While one local item is permanently deleted, the progress status reads "N items deleted" … and the bar pulses | script `deletion`; unit (as above) |
//! | A permanent delete of two or more items keeps the item percentage | unit `a_one_item_delete_counts_and_pulses_and_a_larger_one_shows_its_percentage` |
//! | After Cancel, removed-entry counts no longer change the progress status or bar | **Strata policy** (the host stops sending them; a cancelling bar pulses) |
//! | When the deleted item finishes, the bar stops pulsing and shows 100% | script `deletion` |
//! | A permanent delete still running when its card appears … keeps its dissolve | **not applicable here** (C23 and Strata) |
//! | Cancel during a transfer retitles the dialog "Cancelling transfer…", warns …, and disables the button | scripts `dialog-escape`, `dialog-cancel-stall`, `cancel-one` |
//! | If the transfer has not stopped 8 seconds after Cancel, the title becomes "Device not responding" | test `a_stalled_cancel_in_the_dialog_reads_device_not_responding_after_8_s`; script `dialog-cancel-stall`; unit `cancel_disables_itself_and_a_transfer_stalls_after_eight_seconds` |
//! | While a transfer cancellation is pending, starting another file operation shows "Transfer cancellation pending" | **Strata policy** ([`Dock::cancel_pending`] answers it) |
//! | Cancel on an operation that is not a transfer retitles the dialog "Cancelling operation…" | script `cancel-operation` |
//! | After cancellation, an "Operation cancelled" dialog gives the counts … | **partial**: script `cancel-closes` (the host's C15 dialog); the counts are Strata's |
//! | After cancellation, every affected source and destination folder refreshes | **Strata policy** |
//! | An item that GIO completes as Cancel arrives is counted as completed; an item the cancel cut off …; In merges … | **Strata policy** (counting) |
//!
//! `operations/progress/jobs`:
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | Queue, Invocation and Run directory (all) | **not applicable**: Strata's job service |
//! | Remove on a queued job ends it as Cancelled with "Removed before starting" | **partial**: script `jobs-cancel` (Remove sends the intent); the outcome is the service's |
//! | Cancel on a running job shows "Cancelling…" and sends SIGTERM … | **partial**: script `jobs-cancel` (Cancel sends the intent; the row then has neither Cancel nor Dismiss, and its golden tree pins the row's description "Cancelling… · 4/10 · ~/Pictures"); unit `jobs_words_follow_strata` (a cancelling job still counts as running); the signals are the service's |
//! | A job still cancelling after 10 seconds …; Closing the last application window … | **not applicable**: the service's and Strata's |
//! | The footer shows a Jobs button only when jobs exist … | scripts `jobs-none`, `jobs-button`; unit `jobs_words_follow_strata` |
//! | Launching an action opens Jobs in the launching window with the new job first | **partial**: script `jobs-launch` ([`JobsState::open`](jobs::JobsState::open); the order is the host's rows) |
//! | Minimize, Escape, or a click outside hides the dashboard without cancelling jobs | scripts `jobs-hide-minimize`, `jobs-hide-escape`, `jobs-hide-outside` |
//! | A row shows the action name and icon, a status with elapsed time …, item counts, and the invoking folder | script `jobs-rows`; test `the_jobs_list_ticks_each_second_while_open_and_stops_when_hidden`; golden `c16-jobs` |
//! | An active whole-selection job whose script reports no total shows a pulsing progress bar | script `jobs-rows` |
//! | An active per-item job's bar fills by completed items plus the current item's reported fraction | script `jobs-rows`; unit `jobs_words_follow_strata` ([`per_item_fraction`](jobs::per_item_fraction)) |
//! | Details expands a job's captured output and Hide collapses it; … "No output yet." | script `jobs-details` |
//! | Details on a running job shows output as it arrives, follows the bottom … | **partial**: script `jobs-details` (it shows the rows' output); following the end is Rune's `stick_bottom` |
//! | Terminal colour and control sequences are stripped from the shown output | unit `terminal_sequences_are_stripped_from_output`; script `jobs-details`; golden `c16-jobs` |
//! | Dismiss removes one finished job; Clear finished removes every finished job | scripts `jobs-dismiss`, `jobs-clear-all`; test `clearing_every_job_leaves_a_note_then_focus_on_the_fallback_and_the_list_shut` (the intents; the gallery's host drops the rows). **Difference**: while the emptied list is open the button reads "Jobs" with no jobs, where Strata hides it at once (below) |
//! | The dashboard lists at most 12 jobs, then shows "`<n>` older jobs are hidden" | script `jobs-limit` |
//!
//! # Rune drafts (not filed)
//!
//! - **U-C16-1, ToastRegion and Toast are fixed.** At 5d421fd ToastRegion keys itself
//!   `toast-region`, names itself "Notifications", pins 32 and 48 points in and has no height
//!   cap or scroll; Toast is 320 points wide with a fixed radius and padding. Ask: a key, a
//!   name, insets, a maximum height that scrolls, and the width from the caller.
//! - **U-C16-2, no live fractional width.** A bar's fill can't follow a `Live<f32>` fraction:
//!   `live_rect` is in points, so the track's width has to come back through `on_layout_tx`,
//!   and rune-kit's Progress takes no live value. Repro: a 200 pt track whose fill should follow
//!   `Live<f32>` 0 → 1 without commits needs the width bookkeeping above. Ask:
//!   `El::live_w_frac(&Live<f32>)` or `Progress::live`.
//! - **U-C16-3, accessibility values don't follow `Live`.** A progress indicator's range value
//!   is set at build, so assistive technology hears a moving bar only on commits (here at most
//!   four a second). Probe: `LiveText` does reach the tree (its text replaces the placeholder in
//!   `accessibility_json`), but nothing binds `range_value`. Ask: `El::live_range_value`.
//! - **U-C16-4, no focus-within callback.** No element hears keyboard focus enter or leave it: a
//!   poll can read `Tx::focused` (Rune cd2bd55a) but only by comparing frames, Tab never reaches
//!   the window's `on_key` once a control has focus, and `on_hover_change` doesn't fire for
//!   `hover_group_focus`. Repro: a row with `hover_group_focus().on_hover_change(..)` around a
//!   focusable button, Tab into it: the hover visual turns on, the handler hears nothing. Ask:
//!   `El::on_focus_within(|s, bool, tx|)`.

mod countdown;
mod dialog;
pub mod format;
pub mod jobs;
mod model;
mod rate;
mod view;
mod words;

#[cfg(test)]
mod tests;

pub use countdown::{COMPLETE_FOR, Countdown, FADE};
pub use dialog::{EscapeDoes, ProgressDialog, RATE_WIDTH};
pub use model::{
    COMMIT_EVERY, CardLive, Dock, DockIntent, DockNotice, EndKind, EndShown, IMMEDIATE_ITEMS,
    NOTICE_FOR, Notice, Op, OpId, OpSpec, Placement, Progress, REVEAL_DELAY, STALL_AFTER, Sample,
    Shown, Transfer, reveal_after,
};
pub use rate::{RateMeter, SAMPLE_EVERY, SMOOTHING};
pub use view::{
    BAR_RADIUS, CARD_BAR, CARD_MAX_WIDTH, CARD_MIN_WIDTH, CARD_RADIUS, DIALOG_BAR, DOCK_BOTTOM,
    DOCK_MAX_HEIGHT, DockView,
};
pub use words::{Phrase, Phrase2, Words};
