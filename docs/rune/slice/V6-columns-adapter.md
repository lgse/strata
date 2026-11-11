# V6 Columns adapter

| | |
| --- | --- |
| Status | Not started. Two owner decisions are open |
| Depends on | V5 |
| Unblocks | V7 |
| Owns | `crates/strata-app` (the adapter and its tests) |
| Lands as | Feature commits on `strata-next` |

## Goal

The Rune window shows a real directory in strata-components' Columns view, fed by
`Browser`, with the accessible names the QA harness looks for.

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

The adapter is the only code that knows both. V0's probe crate on the local branch
`spike/v0-rune-probes` is a 400-line sketch of it. Use it to read, not to merge.

## Decide first

| Decision | Why it is open | Options |
| --- | --- | --- |
| 11. Who owns cursor and selection | `Browser` and the component each keep one. In V0 the GTK app launched with `archive` focused and selected, because `Browser` says so at load. The component's cursor started empty, so the first Down landed on `archive` where GTK lands on `documents` | **The component interprets input, `Browser` holds the result:** every `Cursor` and `Selection` intent is written to `Browser`, and every focus or selection event `Browser` raises by itself is written to `ColumnsState`. Or **`Browser` moves the cursor:** keys go to `Browser::move_selection` and friends, and the component only draws. The first keeps the component's tested key and pointer handling. The second keeps Strata's exact semantics. A short probe of the G2 keyboard scenarios against both should decide |
| 5. Row descriptions and the pane group | `ColumnsView` sets each row's description to the size badge's words after the host's hook has run, so a host cannot supply "Folder" or "File". The view also has no node for GTK's pane group, named by the directory and described "Columns view" | Change strata-components so a host can describe rows and name a column's group; or rule that the badge's words are the description and the list is the pane. After V4b the component change is a commit on this branch |

## Scope

### Listings

- **Events to state.** Handle `Reset`, `ColumnsTruncated`, `ColumnAdded`,
  `ColumnsRelocated`, `EntriesInserted`, `EntriesReplaced`, `EntriesPublished`,
  `EntriesSpliced`, `SortingStarted` and `SortingFinished`, `MetadataFilled`,
  `ColumnReloading`, `ColumnReloaded`, `ColumnRefreshing`, `LoadFinished`, `LoadFailed` and
  `HiddenToggled`. Every other event is ignored on purpose, with a comment naming the
  package that will take it.
- **Copy only what changed.** `EntriesPublished` names a range. V0's probe read the whole
  column on every event: 80,000 row copies for a 10,000-entry folder, 14.7 ms in release.
  Copy the range.
- **Borrowing.** `EntrySource::entry` returns names borrowed from the source, and
  `Browser` keeps its entries behind a `RefCell`. Either the adapter keeps a copy of each
  row's display fields, or `Browser` gains an accessor that can lend them. The slice plan
  asked to borrow published ranges as the GTK views do. Measure the copy at 100,000 entries
  before adding to `Browser`'s API.
- **Entry ids.** `EntryId` must be unique across every column and stable for the life of
  the process. Intern `Location` to id in one table. A rename changes the location: move
  what is held by the old id with the selection model's `relocate`.
- **Hidden entries.** A `Browser` column holds them; the GTK views filter them out of the
  model (`entry_matches` in `src/ui/browser/entry.rs`). The adapter filters by
  `FileEntry::is_hidden` unless `show_hidden` is on, and keeps a map from each shown row to
  its `Browser` position. Every intent that names a row goes through the map.
- **Loading and errors.** `ColumnSources::loading` follows the column's load state. A
  failed load fills the column's empty part with `LoadFailed`'s message.
- **Titles.** `Location::display_name()`, as `src/ui/browser/columns/append.rs` passes to
  the GTK pane.

### Intents

| From the view | To `Browser` |
| --- | --- |
| `Collection { Activate }` on a folder or file | `activate(depth, position)` |
| `EnterFolder` | Open the folder as the next column with the cursor on its first entry |
| `Cursor`, `Selection` | Per decision 11 |
| `CloseFrom(depth)` | `close_column(depth)` |
| `LeftOfRoot` | Nothing in the slice: there is no sidebar |
| A press that makes a column active | `set_active_column(depth)` |
| The hidden-files shortcut | `toggle_hidden()` |
| `Collection { BeginRename }` | Recorded for V9 |
| `Width`, `HeaderMenu`, `Dropped`, context menus, drags | Ignored in the slice |

`Browser` raises `SelectionSynced` for a selection a view has already applied; observers
must not apply it again. Use it to stop the adapter echoing its own writes.

### Policy that lives in the GTK views

V0 found that some behaviour the scenarios test is neither in `Browser` nor in the
component. It has to be ported into `strata-app`.

- **Mirroring.** Moving the cursor onto a folder opens its contents as the next column
  without moving focus, after a 75 ms wait. `Browser::select` raises no `ColumnAdded`; the
  GTK views call `show_child`. The code is under `src/ui/browser/` (`view_selection.rs`,
  `columns/append/models.rs`).
- **Initial focus and selection** at launch, per decision 11.
- **Escape.** `Browser::escape` and the component's own Escape handling both exist. Decide
  which runs, and check it against `escape_selection`.

Port each as a function with a test, and name the GTK file it came from.

### Names

- A row is named by its file name. Its description is the entry's kind, as
  `entry_kind_name` in `src/ui/accessibility.rs` words it, through `strata_core::i18n`.
  This needs decision 5.
- A column's list is named by its directory and described "Files".
- The pane is named by its directory and described "Columns view". This needs decision 5.
- The strip's "Columns", "Close this pane", "Loading" and "Column width" come in through
  `ColumnsLabels`, localized.

## Out of scope

- Size badges, unless decision 5 makes them the description. `ColumnSources::badge` returns
  `None`.
- Thumbnails and file-type icons beyond the component's folder and file fallbacks.
- Rename. V9.
- Sorting controls, the filter field, header actions, context menus, drag and drop.
- Icons and List.

## Tests

In-process tests in `strata-app`, on the offscreen host over a temporary directory. They
need no display.

- **Listing.** A fixture tree lists in `Browser`'s order, without hidden entries.
- **Large listing.** 10,000 entries arrive complete, and only the published ranges are
  copied.
- **Keys.** Down, Up, Right and Left move as the G2 scenarios expect, and `Browser` agrees
  with the view after each.
- **Mirroring.** The child column appears after the wait and not before.
- **Hidden toggle.** Rows appear and disappear, and positions still map.
- **Live change.** A file created on disk appears; one removed disappears.
- **Names.** Role, name and description of the pane, the list and each row, compared with
  what V0's name probe captured from GTK.

`AGENTS.md` forbids layout tests: assert names, order, selection and focus, never
geometry.

## Done when

- The in-process tests pass.
- The name probe's diff against GTK is empty for panes and rows, or each remaining
  difference has an owner ruling.
- `cargo tree -p strata-rune -i gtk4` matches no package.
- The owner has seen the window on a real directory (decision 7).

## Risks

- **Two models of selection.** The largest design risk in the slice. A wrong choice shows
  up as many small G2 failures in V8.
- **Ported policy drifts.** Mirroring exists twice until GTK is deleted. Each port names
  its source so a fix on `main` can be carried over.
- **Component gaps.** A behaviour the component lacks becomes a `component` pending run in
  V8, not a workaround here.
