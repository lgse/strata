# V6 Columns adapter

| | |
| --- | --- |
| Status | Done, but for the owner's look at the window (decision 7, moved to V10). Decisions 5 and 11 are settled |
| Depends on | V5 |
| Unblocks | V7 |
| Owns | `crates/strata-app/src/columns/` (the adapter and its tests), the row descriptions and the pane group in `strata-components` |
| Commits | `90fdcb2f7` (the component), `24a04ec0b` (the adapter), `25d40091f` (one more case in a test), then this record |

`strata-rune <directory>` shows the directory in strata-components' Columns view, fed by
Strata's `Browser`, with the accessible names the QA harness looks for.

## The two sides

**Strata.** `Browser` (`crates/strata-fs/src/app/browser.rs`) owns the columns, their
entries, the cursor and the selection. Views call `Browser::observe` and receive
`BrowserEvent`s, and call back with intents such as `navigate`, `activate`, `select`,
`set_selection`, `move_selection`, `set_active_column`, `descend`, `show_child`,
`close_column`, `toggle_hidden` and `rename`.

**Components.** `ColumnsView` reads the host's listings through `ColumnSources` and its
own `ColumnsState`. The host opens and closes columns with `push`, `swap_from`, `truncate`,
`replace`, `return_to` and `sync`. The view handles keys and the pointer itself and reports
`ColumnsIntent`s: `Cursor`, `Selection`, `EnterFolder`, `LeftOfRoot`, `CloseFrom`,
`Collection` (activate, click, rename, context menu), `Width`, `HeaderMenu`, `Dropped`.

The adapter is the only code that knows both.

## Decision 11: who owns the cursor and the selection

**The view interprets input, and `Browser` holds the result.** Owner ruling of 2026-11-11:
the probe decides. This is the probe.

### What the GTK application does

It does both, by input (`src/ui/window/keyboard/items.rs`,
`src/ui/browser/columns/append/models.rs`):

| Input | In the GTK Columns view | Design |
| --- | --- | --- |
| Arrows, Shift+arrows, Home, End, the page keys, Left, Right, Return, Escape | The window's key dispatcher calls `Browser`: `move_selection`, `extend_selection`, `page_along`, `extend_page_selection`, `focus_parent`, `enter_focused_directory`, `activate_focused`, `clear_active_selection`, `escape` | `Browser` moves the cursor |
| Every click, Ctrl+A, the marquee | GTK's own selection model changes, and `SelectionSync` writes it to `Browser::set_selection`, which answers `SelectionSynced` | The view interprets, `Browser` holds |
| Every key in Icons and List | GTK's native arrows, then `set_selection` and `set_selection_anchor` | The view interprets, `Browser` holds |

So `Browser` already takes a selection a view worked out, as a first-class path. Neither
design is foreign to it.

### The probe

Two measurements, both on the offscreen host over the QA harness's canonical tree.

**1. The G2 scenarios' own expectations, against the view-interprets design.** The key
sequences of ten Columns runs under `keyboard_navigation`, `selection` and
`escape_selection`, 43 key presses in all, with what each scenario file waits for after each
key (`focused`, `selected`, `panes`, `directory`). Where a scenario clicks its way to a
state, the keys that reach the same state stand in. Every expectation is met, and after every
key `Browser` holds what the view shows. G2's other Columns runs need what later packages
bring (the sidebar, the breadcrumbs, history keys, the location field, the clipboard, 10xer
mode) or are the pointer's, and were not driven. This is now the test
`keys_move_as_the_g2_scenarios_expect_and_the_browser_holds_the_result`
(`crates/strata-app/src/columns/tests/keys.rs`), which names each scenario file.

**2. A differential run against the `Browser`-moves design.** A second `Browser` over the
same folder took the same keys through the calls the GTK dispatcher makes, with the same
mirroring policy on its `FocusChanged`. After each key the two browsers were compared:
columns, active column, cursor and selection of each. The keys were random walks over the
arrows, Shift+arrows, Left, Right, Return, Escape, Home, End, Shift+Home and Shift+End, each
walk stopping at its first difference. The last of four runs, made as the adapter's
mirroring trigger was corrected, compared 315 key presses in 50 walks of up to ten keys, and
33 walks ended in a difference. The first run also pressed Ctrl+A, and ten of its 40 walks
ended on it. The probe was a temporary test and is not in the tree; the second design was
not built beyond it. The GTK application itself was not run: the "GTK" column below is what
`Browser` did under the dispatcher's calls, and row d is read from `SelectionSync`.

One caveat on the count: the probe sent Up on the first row to `Browser::move_selection`,
where the GTK dispatcher first tries the pane's header, so it overstates rows b and c for
that key.

Every difference was one of six, each traced to its cause:

| # | Keys | GTK (`Browser` moves) | The component | Whose |
| --- | --- | --- | --- | --- |
| a | Return on a folder whose column mirroring already opened | The column takes the keyboard with the cursor on its first entry (`activate_focused` then `focus_child`) | The column takes the keyboard with no cursor; the next Down lands on the first entry | Component (C2: `column_intent` handles the open folder itself, as it does for a click, where GTK also places no cursor) |
| b | An arrow at the list's end, with several entries selected or none | The cursor's entry becomes the selection (`move_selection` re-places the cursor) | Nothing changes (C1c reports an edge) | Component (C1c) |
| c | A move key that leaves the cursor where it is: Home on the first entry, End on the last | `FocusChanged` is raised again, so the folder under the cursor is mirrored if it was not | The view reports no cursor change, so nothing is mirrored | Component (no intent for a key that changed nothing) |
| d | Ctrl+A, then an arrow | The model's cursor goes to the last entry, though GTK's focus ring stays; the arrow moves from the last entry | The cursor stays; the arrow moves from it | Component (a decision C1a records: `Selection::select_all` keeps the cursor) |
| e | Left, or Right into a column that is already open, after the pointer last moved | Mirrors what the cursor is on | Mirrors nothing: the view tells its host of no key that only changes the active column, so the adapter cannot tell it from a click on a column's header | Component (no intent for the active column changing). Reasoned, not measured: the probe moved no pointer |
| f | Right, when the open child column is not the cursor's folder: Home mirrors `archive`, Shift+Down moves the cursor to `documents`, which mirrors nothing | Opens the cursor's folder in the child column's place (`enter_focused_directory`) | Enters the column that is open, another folder's | Component (C2: `enter_right` asks only whether a column is open) |

None of the six is checked by an expectation of those ten runs; the rest of G2 was not read for them. Two more differences were the
adapter's own and are fixed. A Shift range growing or shrinking no longer mirrors, as
`Browser::extend_selection` raises no `FocusChanged`. And a pointer that is parked over the
view no longer ends keyboard navigation when the toolkit reports it again after a redraw:
the window test below failed on that before the fix, and the offscreen host could not have
shown it.

### The choice

Both designs meet what those ten runs expect. The view-interprets design ports less
policy, so it is the one built:

| | The view interprets, `Browser` holds | `Browser` moves the cursor |
| --- | --- | --- |
| G2 key expectations | All met (measurement 1) | Met by construction, if the routing is ported faithfully |
| Policy to port | Mirroring's trigger (the timer is needed either way) | The key routing of `src/ui/window/keyboard/items.rs`, with its bindings, aliases and layout rules, which live in `src/ui/keys` on GDK types; the page size; and the pointer still needs the first design's path |
| What it gives up | The six differences above | The component's key handling, which then has no caller: keyboard mode, reveals, the 10xer range handle. And `Selection::restore` puts the anchor on the cursor, so a Shift range `Browser` made would have to be replayed in the view for a later Shift+click to start from the right entry |

**What this leaves open.** Differences a to c, e and f are component behaviour, not
design: each can be closed in `strata-components` without touching the adapter (for a,
treat an activation by key as Right; for b, re-select at an edge; for c and e, report the
key; for f, enter the child column only when it is the cursor's folder). That is a
component change, which decision 5's ruling does not cover, so none was made. Difference d
is a decision the component already records. The six are owner item 17 in the
[README](README.md#owner-decisions). If a G2 run in V8 trips on one, it is a `component`
pending run until that is ruled.

The adapter is built so that the other design stays reachable: `Browser`'s own cursor
moves already reach the view (`Columns::adopt`) and already mirror (`Change::Selection`'s
`landed`). Routing a key to `Browser` instead would be a change in one place.

## Decision 5: row descriptions and the pane group

**The component changed** (owner ruling of 2026-11-11), in `90fdcb2f7`:

- `ColumnSources::description(depth, index)` gives a row's accessible description in the
  host's words. It stands in for the size badge's words, which stay the default when the
  host gives none. It reads whatever "Show sizes" says and while the row is renamed.
- Each column's shell is a group named by its folder (`ColumnSources::title`) and described
  by the new `ColumnsLabels::pane`. Its list is described by the new `ColumnsLabels::list`,
  through the new `Collection::description`.
- The gallery's C2 page can describe its rows by kind: scenes `c2-kinds-light` and
  `c2-kinds-dark`, and two page tests.

Goldens, blessed with `mise run bless c2` and `mise run bless c22` and reviewed: 20
accessibility and 25 script goldens of `c2` and 3 script goldens of `c22` gain the group's
name and the two descriptions, three added lines for each of the 146 columns in them, and
nothing else. No existing PNG changed. The two new PNG goldens show the badges scene
again, and their accessibility goldens differ from its only in the rows' descriptions.

The adapter passes Strata's words: a row's kind as `entry_kind_name` in
`src/ui/accessibility.rs` words it, "Columns view" and "Files", all through
`strata_core::i18n` and all already in the thirteen catalogs. No string was added.

## What landed

`crates/strata-app/src/columns/`:

| Module | Holds |
| --- | --- |
| `mod.rs` | `Columns`: the listings, the view's `ColumnsState`, and `apply`, which puts one queued change into both |
| `listing.rs` | `Row`, `Column` (an `EntrySource`) and `Listings` (the `ColumnSources`); the hidden-row map; the words for a row's kind |
| `ids.rs` | `Ids`: the one table from `Location` to `EntryId` |
| `selection.rs` | Both directions of the cursor and the selection: `adopt` (from `Browser`) and `publish` (to it) |
| `intents.rs` | What the view asks, taken to `Browser` |
| `mirror.rs` | Mirroring, ported from the GTK views |
| `view.rs` | The `ColumnsView` over the adapter's state, with the translated labels |

`crates/strata-app/src/events.rs` grew from one kind of change to ten: the observer now
copies out of `Browser` what each event changed.

### Listings

- **Events to state.** `Reset`, `ColumnsTruncated`, `ColumnAdded`, `ColumnsRelocated`,
  `EntriesInserted`, `EntriesReplaced`, `EntriesPublished`, `EntriesSpliced`,
  `SortingStarted` and `SortingFinished`, `MetadataFilled`, `ColumnRefreshing`,
  `ColumnReloaded`, `LoadFinished`, `LoadFailed`, `HiddenToggled`, `FocusChanged` and
  `SelectionSetChanged` are handled. Every other event is ignored, with a comment in
  `events.rs` saying which package takes it. `ColumnReloading` is among those: it comes
  before the column is emptied, and `ColumnReloaded` is the one that changes what shows.
- **The observer copies, `poll` applies.** An observer runs inside GLib's dispatch and may
  not touch the application state. But by the time the queue is applied, later events have
  moved `Browser` on, so reading it then would apply the first event with the last event's
  entries. The observer therefore copies each event's rows, and `Browser`'s cursor and
  selection for that column, into the queue as it hears the event. It reads `Browser`
  during dispatch, as the GTK list models do.
- **Only what changed is copied.** `EntriesReplaced { count }` copies the first `count`
  entries: `Browser` holds the whole listing by then and publishes it in parts.
  `EntriesPublished` copies its range, `EntriesInserted`, `EntriesSpliced` and
  `MetadataFilled` their payloads. A 10,000-entry folder copies 10,000 rows, which the
  large-listing test asserts.
- **Borrowing.** A column keeps a copy of each row's display fields. `Browser` gained no
  accessor, because the copy is a tenth of a listing's time. Measured on the host with
  the large-listing test, four runs each in a release build: 10,000 entries copy in 1.3 to
  1.5 ms of the 16 to 17 ms from `navigate` to the finished load, and 100,000 in 14.7 to
  15.3 ms of 148 to 155 ms. A debug build copies 100,000 in 84 ms of 709 ms. V0's probe,
  which read the whole column on every event, took 14.7 ms for 10,000.
- **Entry ids.** `Ids` interns `Location` to `EntryId`, shared by the observer and the
  adapter, and never forgets a pair. A rename gives an entry a new location and so a new
  id; `Browser` says which entry is selected after one, and the adapter writes that to the
  view, so nothing needs `Selection::relocate`. No test renames anything before V9, so
  that path is unproven. [V9](V9-rename-parity.md#ids-the-cursor-and-the-selection-after-a-rename)
  proved it wrong for a range, and relocates. The table only grows: one entry for every location listed in the
  life of the process.
- **Hidden entries.** A column holds a row for every entry `Browser` holds, at `Browser`'s
  position, and a list of the positions it shows. A view index goes through that list to
  become a position (`Column::position_at`), and an id through a second table
  (`Column::position_of`). `HiddenToggled` recomputes the list and copies nothing.
- **Loading and errors.** `ColumnSources::loading` is true while the folder is read, sorted
  or refreshed. A failed load shows `LoadFailed`'s message in the column's empty part.
- **Titles.** `Location::display_name()`.

### The cursor and the selection

- **To `Browser`.** `Columns::publish` compares each column's selection revision with the
  one `Browser` last agreed with, and writes what moved: `commit_selection`,
  `set_selection`, `set_selection_anchor`, then `set_active_column`. It runs after every
  intent, in `poll`, and before a host lets GLib dispatch, so an event's copy of `Browser`'s
  selection is never older than the view's.
- **From `Browser`.** `FocusChanged` and `SelectionSetChanged` always win: the cursor a
  load places, a reveal, the neighbour a deletion moves the cursor to. After a listing
  change the view first lets go of entries that left (`CursorGone::Clear`: where a cursor
  goes is `Browser`'s to say), and then takes `Browser`'s selection if it differs.
- **One exception.** A cursor the view was waiting to place until a folder was listed
  (Right into a column still loading) is the user's, and goes to `Browser`.
- **No echo.** `SelectionSynced` is ignored. A write that changes nothing in the view is
  not made, so a Shift range in progress keeps its anchor through a listing event.

### Intents

| From the view | To `Browser` |
| --- | --- |
| `Collection { Activate }` by key | `activate_focused()`: a folder opens with the cursor on its first entry |
| `Collection { Activate }` by click | `activate(depth, position)` |
| `Collection { Clicked }` on a file in a column before the last | `close_column(depth + 1)` |
| `EnterFolder` | `enter_focused_directory()` |
| `Cursor`, `Selection` | `publish` |
| `CloseFrom(depth)` | `close_column(depth)` |
| Ctrl+H, Ctrl+. (the window's `on_key`) | `toggle_hidden()` |
| `Collection { BeginRename }` | Logged. V9 opens the editor there |
| `LeftOfRoot`, `Width`, `HeaderMenu`, `Dropped`, context menus, drags | Ignored in the slice |

The saved click counts for Columns (`list_file_clicks`, `list_folder_clicks`) reach the
view's pointer settings.

### Policy ported from the GTK views

| Policy | From | To |
| --- | --- | --- |
| Mirroring: the 75 ms wait, a folder opens with `show_child`, a file closes the next column, only after keys or a deletion, never while `Browser` suppresses it or validates a folder | `src/ui/browser/events/selection.rs` | `columns/mirror.rs`; its trigger in `selection.rs` (`follow`) and `mod.rs` (`landed`) |
| The pointer moving ends keyboard navigation, and a parked pointer reported again does not | `src/ui/input_ownership.rs` | `shell::body`, `Mirror::note_pointer` |
| A plain click on a file closes the columns after it | `src/ui/browser/columns/rows/click.rs` | `columns/intents.rs` |
| Return against Right against a click on a folder | `src/ui/window/keyboard/items.rs`, `src/ui/browser/columns/state.rs` | `columns/intents.rs` |
| A new listing takes the keyboard | `src/ui/browser/events/selection.rs` | `Columns::add` |

Not ported, with what brings each back: a rename or a new entry being edited blocks the
mirror (V9); a 10xer visual range, a filtered column and the automatic preview of a file
(out of the slice).

**Initial focus and selection** need no policy: `Browser` places a load's cursor and says
so, and the adapter writes it to the view. The V0 symptom is gone: the first Down lands on
`documents`.

**Escape** is the component's: it clears the active column's selection, which `publish`
hands to `Browser`, and with nothing selected asks to close the last column, which goes to
`Browser::close_column`. `Browser::escape` is not called. The `escape_selection` sequences
pass that way.

## Tests

In `strata-app`, each offscreen test in a process of its own, with no display:

| Test | Shows |
| --- | --- |
| `a_folder_lists_in_the_browsers_order_without_its_hidden_entries` | The fixture lists as `Browser` orders it, without `.hidden.txt`, with `archive` focused and selected |
| `a_large_folder_arrives_whole_with_each_row_copied_once` | 10,000 entries arrive, 10,000 rows are copied, and the list mounts a part of them |
| `keys::keys_move_as_the_g2_scenarios_expect_and_the_browser_holds_the_result` | Decision 11's first measurement |
| `the_child_column_follows_the_cursor_after_its_wait_and_not_before` | Nothing opens in the turn that takes the key; the column opens no sooner than 75 ms later; two keys in one turn open nothing on the way |
| `the_hidden_files_shortcut_shows_and_hides_rows_and_positions_still_map` | Ctrl+. and Ctrl+H; Return opens the folder under the cursor though `Browser` holds a hidden one before it |
| `a_change_on_disk_reaches_the_listing` | A file created appears in order; one removed goes and the cursor stays; when the cursor's own entry is removed, the cursor goes where `Browser` puts it |
| `a_click_selects_in_the_browser_and_opens_or_closes_columns_as_the_gtk_views_do` | The pointer's path into `Browser`, and the click policy |
| The same mirroring test, further on | A parked pointer reported again does not stop the mirror; the pointer moving before the wait is over does |
| `a_folder_that_cannot_be_read_says_why` | `LoadFailed`'s message is in the tree |
| `panes_lists_and_rows_carry_the_names_the_gtk_views_give_them` | The name probe, below |
| `listing::tests` (three) | Edits, the hidden-row map and refills, without a host |

The tests read the view from the accessibility tree, as the QA harness will.

In `strata-rune`, in a private headless sway (`mise run test-platform window::`):

| Test | Shows |
| --- | --- |
| `window::a_folder_change_reaches_the_window` | The windowed binary's Columns view lists its folder, and a file created in it |
| `window::a_key_moves_the_cursor_and_the_child_column_follows` | A key sent through the compositor reaches the view, which has the keyboard from the start; the mirror's GLib timer, started in a handler, ends while Rune's loop is idle |
| `window::a_cover_does_not_slow_a_folder_change` | As after V5 |

They read the binary's trace lines. A screenshot taken once in that session, with `grim`,
showed the two columns with `beta` under the cursor and its `inside.txt` beside it. It is
not kept and nothing asserts it: the picture is the owner's check.

### The name probe against GTK

V0 captured the GTK tree for the canonical fixture two Downs in
(`.v0/gtk/tree-after.txt` on the spike branch). The names test builds the same state and
compares the whole of both panes:

| Element | GTK harness | Rune tree now | Same |
| --- | --- | --- | --- |
| Pane | `grouping`, named by the directory, described "Columns view" | `group`, same name, same description | Yes, through V7's role map |
| List | `list`, named by the directory, described "Files" | `list-box`, same name, same description | Yes, through the role map |
| Row | `list item`, named by the file, described "Folder" or "File" | `list-box-option`, same name, same description | Yes, through the role map |
| Selected row | `[selected] [focused]` on the row | `selected` on the row | The backend reads focus from the strip (V7) |
| Focus | On the row | On the group named "Columns", whose `activeDescendant` is the row | V7 |
| Open child pane | A second pane for the focused folder | The same | Yes |

The test adds a link to a folder and a broken link, described "Folder link" and "Broken
link". The diff for panes and rows is empty; what remains is the role map and where focus
is read, which are V7's.

## Done when, and the evidence

All on 2026-11-11.

Everything ran on the host. Nothing ran in the pinned image, which builds the GTK lane
only: the package changed no shared crate, no file under `src/`, no QA scenario and no
image input. `Cargo.lock` changed by one line, `strata-app`'s test dependency on
`serde_json`, which the lock already held.

- **The in-process tests pass.** `mise run test-rune columns::` runs the twelve new tests
  of `strata-app`: nine on the offscreen host and three of `Column`. At `25d40091f`, which
  adds a case to one of them, `mise run test-rune columns::tests::` passed its nine tests
  five times running.
- **The name probe's diff against GTK is empty for panes and rows**, in names and
  descriptions. Role names differ as the slice plans, and V7's role map carries them; no
  difference waits on a ruling.
- **`cargo tree -p strata-rune -i gtk4` matches no package:** "package ID specification
  `gtk4` did not match any packages". The same for `gdk4` and `sourceview5`.
  `cargo deny --workspace check` passes all four sections.
- **The owner has not seen the window.** Decision 7 is not met here; the owner moved it to
  V10. In its place, `mise run test-platform window::` passes its three tests, at
  `24a04ec0b`.
- **The Rune side.** `mise run test-rune` at `24a04ec0b`, before that last case was added:
  71 test binaries, 2,312 tests passed, none failed, 253 ignored. That is V5's 2,298 and 252, plus two gallery page
  tests, the twelve tests above, and one more compositor test among the ignored. After the
  component commit alone the same run gave 2,300 and 252.
- **Lint.** `cargo fmt --all --check`, Clippy on the Rune lane with warnings denied
  (`cargo clippy $(python3 scripts/lanes.py rune) --all-targets --all-features --locked --
  -D warnings`), `typos`, `git diff --check` and `scripts/test_lanes.py` pass.

Not run, and why:

- `./scripts/quality.sh`, `./scripts/qa.sh`, `./scripts/qa-mutation.sh` and
  `./scripts/test-headless.py`: the GTK lane, which this package does not touch.
- Clippy on the GTK lane, for the same reason.
- The rest of `mise run test-platform` (`wake::` and the shims' tests): `strata-platform`
  did not change.
- `scripts/code-health.sh`: `CS_ACCESS_TOKEN` was not set where this ran. The new functions
  were kept inside the limits `AGENTS.md` names by reading them, which is not a score.
- CI: the commits were not pushed when this was written.

## Open

- **Decision 7.** The owner has not seen the window. Moved to V10 by the owner.
- **Owner item 17.** The six component differences of decision 11.
- **Not exercised.** `ColumnsRelocated` (a rename of an open folder) has code and no test:
  nothing in the slice renames before V9, which tests it. A remote listing, which publishes through
  `EntriesInserted`, is covered by the unit tests of `Column` only.
- **The settings are still read-only.** Ctrl+. changes what shows and does not save
  `show_hidden`: `strata-rune` saves nothing (V5b). `hidden-files-persist-across-restart`
  waits on that in V8.
- **An empty folder shows nothing** where GTK shows its empty page.

## What it left for V7

- **Marks.** A pane is the `group` described "Columns view" (key `columns-slot-N`). Its
  entry container is the `list-box` inside it (key `columns-col-N`), and its entries are the
  `list-box-option` rows under that (key `row-<EntryId>`). The names test's `seen` function
  walks the tree exactly so. `N` is a serial that goes up with every folder a column shows,
  not a depth: find panes by their description and their order, not by key.
- **Focus.** The strip is the node with `activeDescendant`, named "Columns". Its active
  descendant is a row's key, or the list's key while the active column has no cursor. Row
  keys are unique across columns, because entry ids are.
- **The active pane** is the one that holds the active descendant; `Browser::active_depth`
  agrees after every settle.
- **Settling.** `shell::settle` runs in `poll`. A turn of the offscreen host is: publish,
  dispatch GLib, one frame. The mirror's wait is a GLib timer, so a host that stops turning
  as soon as nothing is ready returns before a mirrored column opens, as the GTK harness
  does; scenarios wait for it.
- **Animations.** The look still has them on. With them on, the host reports work for some
  hundreds of milliseconds after a key, and a column that is closing stays in the tree while
  it fades. The tests call `Runtime::settle_all` after each turn that did something; the
  keys test takes 2.5 s that way and took 32 s without.
- **Modifiers have to be held, not only named.** The adapter reads `Tx::mods` to tell a
  Shift range from a move, since the view's intent carries no key. `Tx::mods` is what
  `Runtime::physical_input` holds, not the `KeyEvent`'s own modifiers, so send a chord as
  the tests' `send` does: modifiers down, the key, modifiers up. Without that the adapter
  falls back on the range's anchor, which misses one case (a range shrunk back to its
  anchor mirrors).
- **The pointer.** A pointer move to a new place ends keyboard navigation for the mirror,
  as in GTK. Rune reports a parked pointer again after a redraw; the adapter ignores a
  report from the same place.
- **A debug warning on stderr.** Keys that come faster than a quarter of a scroll's frames
  make Rune print "`on_key` … committed the view on 6 of 21 frames of a scroll". It is the
  strip's own key handler, in debug builds only, and `RUNE_PERF_WARN=0` silences it.
- **The host's accessors** (`Offscreen::runtime`, `tree`, `run_until`) are test-only until
  V7 gives the host the `qa` feature.
