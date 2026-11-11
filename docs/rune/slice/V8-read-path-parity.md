# V8 Read-path parity

| | |
| --- | --- |
| Status | Done, but for one part that stopped: the Appearance popover is not mounted ([why](#the-appearance-popover-stopped)). Gate G2 passes: 23 of its 42 runs are `required` on both targets, three times running |
| Depends on | V7 |
| Unblocks | V9 |
| Owns | `tests/qa/parity/slice.toml`, `tests/qa/parity/mutations/`, fixes in `crates/strata-app` |
| Commits | `dc1f7ed9c` (the survey and the stop), then `aa3960033` to the record's own commit: the `scope` code, three fixes, the row's label, the upstream request, C2d in four commits, the context menus, the backend's menu support, the mutations, the ledger |

## The result

| | Runs | Of 42 |
| --- | --- | --- |
| `required`, passing on GTK and on Rune, three times running | 23 | 55% |
| `scope` pending: a part the slice leaves out | 18 | 43% |
| `platform` pending: the clipboard | 1 | 2% |
| `component`, `rune`, `harness`, `toolkit`, `deviation` | 0 | |

One of the 18 names a `rune` blocker as well: it needs 10xer mode first.

The package stopped once, at its stop condition, and went on by an owner ruling
([below](#the-stop-condition)). The record of that stop is in `dc1f7ed9c`; this file now
records the whole package.

## Owner rulings in this package

All of 2026-11-11, relayed by the orchestrator.

- **Decision 19: go on, and widen the slice to menus.** The context menu and the
  Appearance menu are mounted in `strata-app`; every other Out part stays out. A run that
  gets past its menu and then needs one is `scope` pending.
- **A seventh reason code, `scope`,** for a run that needs a part the slice marks Out.
  `component` keeps the README's meaning: strata-components lacks or differs.
- **Decision 20: backend-only first** for a row's name label, with no change to the
  accessibility tree. It was found: [the row's label](#the-rows-name-label-decision-20).
- **Decision 17: match GTK on all six,** each verified against the GTK code first.
- **Decision 6** as before: layout runs are required.

## Goal

Gate G2: every Columns run of the read-path scenario folders is either `required` and
passing on both targets, or `pending` with a reason and a blocker.

## The set

Six folders under `tests/qa/scenarios/`: `keyboard_navigation`, `selection`,
`escape_selection`, `background_selection`, `hidden_files` and `startup_arguments`.

- 39 files, 35 of them with Columns runs, 42 Columns runs in all. All 42 passed on GTK at
  the V3 baseline.
- The scenarios are taken as they are. None is edited.

What those files ask of a backend, counted over their steps at `65eb2da03`:

| Kind | Used, most common first |
| --- | --- |
| Steps | `wait`, `key`, `fs`, `expect`, `click`, `select`, `remember`, `type`, `select_with_keyboard`, `note`, `layout`, `open_menu`, `restart`, `drag`, `resize`, `launch`, `proc`, `sleep` |
| Predicates | `field`, `selected`, `focused`, `panes`, `entry`, `directory`, `entries`, `present`, `absent`, `menu_items`, `all`, `preview`, `pref`, `focused_pane`, `dialog`, `editor_within_viewport`, `all_selected`, `some_window`, `editable_fields`, `view_mode`, `peek`, `pane`, `state`, `window_width`, `entry_gone`, `not`, `blank`, `description` |
| Targets | `field`, `entry`, `role`, `pane`, `menu_item`, `dialog`, `sidebar`, `dialog_button` |
| `prefs` headers | `single_click_previews`, `type_to_search`, `tenxer_mode`, `filter_include_subfolders`, `sort_key`, `sort_direction`, `group_by_type`, `arrow_navigation_scoped` |

The counts cover G2 and G3 together; `field` is mostly G3's.

Expect these to end as `pending`, because the slice leaves them out:

| Needs | Likely code |
| --- | --- |
| A context menu (`open_menu`, `menu_items`, `menu_item`) | `component` or `harness`, pointing at the menu work package. Since decision 19 the context menu is in the slice |
| The preview drawer or folder peek (`preview`, `peek`) | `component` |
| A dialog or a second window (`dialog`, `some_window`) | `component` |
| The sidebar (`sidebar`) | `component` |
| 10xer mode (`tenxer_mode`) | `component` |
| The filter (`filter_include_subfolders`) | `component` |
| A non-US layout (`layout`) | None for the layout itself: the owner ruled these runs required (decision 6), and [V7](V7-rune-qa-backend.md#decision-6-a-layout-on-a-target-with-no-display) built how a layout reaches the Rune target. A run that binds a plain letter under a non-Latin layout is `rune`: Rune's public input carries no physical key |
| A drag (`drag`) | `platform` when it is a file drag; a marquee drag is in scope |

## Scope

1. **Run the set on Rune.** `./scripts/qa-parity.sh g2 --pending`, the survey of the gate's
   pending runs on the Rune target. Artifacts are kept, in `target/qa-artifacts-rune/`.
2. **Sort every failing run.** Read its transcript, tree and state. Then one of:
   - the adapter or the ported policy is wrong: fix it in `strata-app`;
   - the backend lacks a step, target or predicate that the slice should have: add it,
     behind the engine, with a unit test;
   - the run needs something outside the slice: mark it `pending` with a reason code and a
     blocker that someone can act on.
3. **Never edit a scenario to pass.** A scenario that checks something GTK-specific gets
   reason `toolkit` and a proposed rewording, written up as its own GTK-side change.
4. **Keep the ledger honest.** A run is `required` only once it has passed on both
   targets. A `pending` run's blocker names a step, a work package id, a Rune draft id or
   an owner ruling, not "fails".
5. **Add two Rune mutation patches,** one for keyboard navigation and one for selection.
   Each is a small patch against `crates/strata-app` that breaks one behaviour, stored
   under `tests/qa/parity/mutations/` and named for the scenario that must catch it.
   `qa-parity.sh` checks them the way `scripts/qa-mutation.sh` checks GTK's: the scenario
   passes unpatched and fails patched. V7 left the `mutations` selection unbuilt, so it is
   added here, with the `slice` selection running it.
6. **Three clean runs.** The full `required` set passes three times in a row on both
   targets before the gate is called.

## The stop condition

If more than a third of G2's 42 runs end as `component` or `rune` pending, stop and report.
That is a finding about how ready the components are, not a reason to patch around them.
Count it as soon as the first sort is done, before fixing anything.

**Counted on 2026-11-11, both ways.**

| Coding | `component` or `rune` pending | Of 42 | Over a third |
| --- | --- | --- | --- |
| As first counted, by this plan's table of likely codes: a run that needs a part the slice leaves out is `component` | 20 | 48% | Yes: hit, and the package stopped (`dc1f7ed9c`) |
| As the owner then ruled: such a run is `scope`, and `component` is a component that lacks or differs | 0 | 0% | No |

Under the ruled coding no run is `component` pending, and one `scope` run names a `rune`
blocker behind its first one. That is why the package went on. All 20 runs of the first
count needed a part whose component is `merged` in [`STATUS.md`](../work/STATUS.md) and
that `strata-app` does not mount; none failed at a step where a component lacks a
behaviour or behaves differently. Two of the 20 are `required` now, with the context menu
mounted.

## Out of scope

- Anything the slice marks Out: menus, preview, dialogs, sidebar, tabs, search, filter,
  10xer.
- Icons and List runs.
- Making the Rune look match GTK's geometry. A run that fails only on a pixel offset is
  `toolkit`.

## Tests

| Check | Command | Expect |
| --- | --- | --- |
| G2 on both targets | `./scripts/qa-parity.sh g2`, three times | Every `required` run passes each time |
| The ledger | The runner's unit tests | Every run listed; no `pending` without reason and blocker |
| Mutations | `./scripts/qa-parity.sh mutations` | Both patches fail their scenarios |
| GTK unchanged | `./scripts/qa.sh` for the six folders' scenarios | As at the baseline |
| Unit tests | `cargo test -p strata-app --features qa` | Pass, including a test per fix |

Each fix in `strata-app` comes with an in-process test that names the scenario it came
from, so the behaviour is held without the container.

## Done when

- The ledger covers all 42 runs.
- The `required` runs passed in three consecutive full runs on both targets.
- Both mutation patches fail their scenarios.
- The stop condition was counted and is recorded, hit or not.

## Risks

- **Flaky passes.** A run that passes two times in three is not `required`. Find the race
  first; it is usually a settle that returned early.
- **`pending` as a dumping ground.** Every `harness` pending run is work this slice chose
  not to do. If most pending runs are `harness`, the backend is too thin to judge the
  components by.
- **Selection ownership.** If V6's choice was wrong, it shows here as many small failures
  across `selection` and `escape_selection`. Revisit the choice rather than fix them one
  by one.

## The record

### The surveys

`./scripts/qa-parity.sh g2 --pending`, in the pinned image, from the worktree at
`~/dev/strata-next-verify`:

| At | Pass on Rune | What changed |
| --- | --- | --- |
| `2acdb32e8`, three times | 16 of 41, the same 16, each agreeing with V7's GTK transcript in every verdict | Nothing: V7's backend as it was |
| `b0c2910f8` | 19 | The three fixes |
| `0ef577ff8` | 20 | The row's label; C2d |
| `991e931c8` with the sorted ledger | 22 required pass on both targets; none of the 19 pending passes | The context menu |

The first three surveys' logs and artifacts are in that worktree's `target/v8-2acdb32e8/`;
the later ones in `target/v8-work/`.

### The sort

**Required (23).** The 17 that passed untouched, and six that pass now.

| Folder | Runs |
| --- | --- |
| `keyboard_navigation` | `arrow-keys-move-focus/columns-arrows` (G1), `columns-home-end-mirror`, `select-all-selects-every-entry`, `shift-arrow-extends-selection`; and now `alt-up-and-history`, `arrow-keys-move-focus-remaining/columns-hjkl` |
| `selection` | `control-click-sets-shift-anchor`, `home-end-page-move-shift-anchor`, `shift-click-range`; and now `leftover-click-honors-modifiers`, `right-click-selects-entry/columns-file` and `columns-folder` |
| `escape_selection` | `enter-after-escape-opens-folder`, `escape-only-clears-active-column`, `shift-after-escape-starts-on-focused/columns-no-range` and `columns-had-range` |
| `background_selection` | `background-click-clears-selection`, `background-press-clears-before-drag`, `beside-last-column-clears` |
| `hidden_files` | now `hidden-files-persist-across-restart` |
| `startup_arguments` | `startup-with-closed-pipes/columns-stdout`, `columns-stderr`, `columns-both` |

**Pending (19).** The ledger has each run's blocker in full. Line numbers are the scenario
file's, where the run stops on Rune now.

| Run | Code | Stops at | Blocker |
| --- | --- | --- | --- |
| `arrow-keys-move-focus/columns-hjkl-tenxer-ru` | `scope` | Line 7 | 10xer mode (C1c). Then `rune`: a plain-letter binding under a non-Latin layout ([the request](../upstream/rune/97-key-press-with-physical-key.md)) |
| `tenxer-keeps-location-edit` | `scope` | Line 9 | The location field at Ctrl+L (C6); the run is 10xer mode's |
| `tenxer-escape-closes-peek` | `scope` | Line 6 | 10xer mode (C1c) |
| `tenxer-escape-filter-preview-fill` | `scope` | Line 4 | 10xer mode's `f` and the filter (C1c, C18), then the preview (C9) |
| `back-then-file-keys-target-focused-folder` | `scope` | Line 10 | The sidebar (C8), then V9's editor and Delete |
| `sidebar-navigation-range-anchor` | `scope` | Line 5 | The sidebar (C8) |
| `columns-return-selects-source-folder` | `scope` | Line 15 | A breadcrumb, pressed (C6) |
| `focus-usable-after-view-change` | `scope` | Line 5 | The List view (C4), then tabs (C7) and Icons (C3) |
| `control-click-toggles`, `parent-pane-anchors-first-entry`, `empty-column-clears-other-columns`, `shift-click-revisits-after-folder/columns-name` and `columns-row-space` | `scope` | Line 2 | The Appearance popover, then the location field at Ctrl+L (C6) |
| `escape-dismisses-transient` | `scope` | Line 10, past the context menu | The properties dialog (C15), then the location field, the filter, V9's editor, the preview |
| `disabled-transport-click/columns-audio` and `columns-video` | `scope` | Line 4 | The preview drawer and its transport (C9, C13) |
| `preview-blank-click-keeps-selection` | `scope` | Line 5 | The preview drawer (C9, C14) |
| `multiple-arguments-non-utf8` | `scope` | Line 10 | A window for each argument, a revealed file, the session bus (P8) |
| `keyboard-only-copy-paste` | `platform` | Line 13 | The clipboard |

No run is `toolkit` pending: nothing was reworded and nothing is proposed.

### The three fixes

Each is in `strata-app`, with an in-process test that names its scenario.

| Run | Was | Fix | Test |
| --- | --- | --- | --- |
| `alt-up-and-history` | The window took no history key | Alt+Left, Alt+Right, Alt+Up and Alt+Home go to `Browser::back`, `forward`, `parent` and `navigate`, as `history_jump` in `src/ui/window/keyboard/items.rs` has them (`2b7d89a38`) | `columns::tests::history::alt_up_and_the_history_keys_…` |
| `arrow-keys-move-focus-remaining/columns-hjkl` | The adapter passed the view no key settings | With "Type to search" off (or 10xer mode on) the view reads `h j k l` as the arrows: `KeySettings::letters(Letters::Arrows)` (`b56fb7d2e`) | `columns::tests::letters::with_type_to_search_off_…`, `…on_a_letter_moves_nothing` |
| `hidden-files-persist-across-restart` | `strata-rune` never saved | `settings::Saved` saves the hidden-files choice and the sort when the browser says they changed, merged key by key on the shared writer, and never over a file it could not read (`b0c2910f8`). [V5b](V5b-settings-store.md) has the detail | `columns::tests::saved::the_hidden_files_choice_is_saved_…`, `…an_unreadable_settings_file_is_never_saved_over` |

### The row's name label (decision 20)

**Found backend-only: no accessibility-tree change and no golden.** `70fbb110f`.

A scenario's `part: label` looks for a `label` named by the entry under its row. Rune's
row is one accessible node. The collection already keys its name element for its hosts
(`strata_components::collection::element::LABEL`, "so hosts, scripts and motion effects can
name a row, its icon and its label"), and `Runtime::elements()` lists every drawn element
with its key and box. The backend reads the name's box from there and presents it as a
`label` under its row in the tree the engine walks. Nothing is guessed: the box is the
element's own.

One difference from GTK stays, and is not hidden: Rune's label is as wide as its text, so
it has none of the leftover space GTK's label has. `leftover-click-honors-modifiers` aims
two points inside the label's right edge, which on Rune is on the name's last glyph. What
the run checks, a Shift or Ctrl click whose modifier is let go before the button, passes
there.

Tests: `qa::tree::tests::a_drawn_name_is_a_label_under_its_entry_and_under_nothing_else`,
`qa::tests::a_click_on_a_rows_name_label_lands_on_the_name_the_view_draws`.

### Decision 17: the six keys

Each was read in the GTK code before it was changed. "Verified" is the call chain from the
window's key dispatcher (`src/ui/window/keyboard/items.rs`) into `Browser`.

| # | Key | The GTK view, verified | What changed, and where |
| --- | --- | --- | --- |
| 1 | Return on the folder whose column is open | `activate_focused` ends in `Browser::focus_child` (`browser/activation.rs`, `navigation/cursor.rs`): the open column takes the keyboard, with the cursor on its own cursor, else its first visible entry, selected alone; `FocusChanged` follows, so that entry mirrors. A click goes through `Browser::activate` instead and places no cursor | C2 `columns/view.rs`: `reopen` goes in as Right does for `Activation::Key` (`c072c317f`) |
| 2 | An arrow at the list's end | `Browser::move_selection` always re-places the cursor (`place_committed_cursor`): several selected, or none after Escape, become the cursor's entry alone, and `FocusChanged` is raised. Up on the first entry first tries the pane header's actions when "Keep arrows in file list" is off and one takes the focus, and only otherwise does this | C1c `keyboard/machine.rs`: `select` re-selects at a vertical edge with `KeySettings::reselects_in_place`, which C2 turns on (`ad73edd15`). The edge is still reported |
| 3 | A key that leaves the cursor in place | Home, End and the page keys go to `Browser::page_along`, which also always re-places the cursor and raises `FocusChanged` again; the GTK views mirror from `FocusChanged` (`on_focus_changed`), so the folder under the cursor opens if it was not | C1c emits `Outcome::Rested` there; C2 reports it as `ColumnsIntent::Cursor` (`ad73edd15`); the adapter mirrors again (`6078b1350`) |
| 4 | Ctrl+A | In Columns it selects through GTK's selection model, and `SelectionSync` reports the newest selected row, the last, as `Browser`'s cursor (`columns/append/models.rs`); `set_selection` keeps the anchor; no `FocusChanged`, so nothing mirrors. The outline Strata draws follows the cursor | C1c: with `KeySettings::select_all_to_last` the cursor goes to the last entry, the anchor stays and nothing is revealed (`ad73edd15`) |
| 5 | A key that only changes the active column | `Browser::focus_parent` and `focus_child` raise `FocusChanged` for the column the keyboard lands in, so its cursor is mirrored. The dispatcher marks keyboard navigation before the key, so it mirrors though the pointer moved last. This was reasoned in V6 and is now read from the code | C2: new `ColumnsIntent::Active { depth }` from Left, from Right into an open column and from Return into one (`c072c317f`); the adapter publishes it as a key (`466097be0`) |
| 6 | Right when the open column is another folder's | `enter_focused_directory`: the open child is entered only when it is the cursor's folder (`is_open_child`); any other folder is descended into in its place; a file enters whatever column is open | C2: `enter_right` asks the host to open the cursor's folder unless the open column is its own (`c072c317f`) |

Where each lives, and why: the two rules of the keyboard machine are C1c's because the
machine alone knows a Select command from 10xer's cursor command, which must not collapse a
selection; they are settings that only C2 turns on, so Icons and List keep theirs. The
rest is C2's handling of a column's edge. Mirroring stays the adapter's: it is Strata's
policy.

Left out, and said in C2's module docs: Up reaching the header's actions (no host gives
them focus), and GTK's cursor on the entry before the last when Ctrl+A finds only the last
entry selected, which was inferred from GTK's selection model and not confirmed.

Goldens, by `mise run bless c2/<script>`, each diff read: `c2/ctrl-a-active-column` (the
strip's active descendant is the last row) and `c2/left-right-active` (the page shows
"Keyboard in column 0"), one line each. No accessibility golden and no PNG changed, so no
picture was there to look at. Tests: six page tests in `crates/strata-gallery/tests/c2d.rs`,
three unit tests in C1c, and in `strata-app`
`keys::keys_do_what_the_gtk_view_does_where_the_component_differed` and
`keys::a_key_between_open_columns_mirrors_though_the_pointer_moved_last`, which check the
browser holds what the view shows. [`STATUS.md`](../work/STATUS.md) has the row, C2d.

The six landed as two component commits and two adapter commits, not six: one function of
the machine decides 2 and 3 and one settings value carries 2, 3 and 4, and 1, 5 and 6
share the function that enters a column.

### The context menus (decision 19)

`c95434573`. A right click on an entry, on a pane's empty space or on its header opens
strata-components' C21 menu. The rows, their order, their sections and their hints are a
policy ported into `crates/strata-app/src/menus.rs` from `src/ui/browser/context_menu/`
(`item_options.rs`, `item_menu.rs`, `folder_menu.rs`) and `default_hint` in
`src/ui/shortcut_reference.rs`: no menu model exists outside `src/ui`, and nothing under
`src/ui` moved. Labels are the catalogs' through `strata_core::i18n`; no string was added.
Hints are rendered from chords with Rune's `Keybind::label`, not typed.

| Menu | Rows, a line for each section |
| --- | --- |
| A file | Its name and path; New Folder, New File; New Folder with Selection; Open, Open With…, Quick preview, Run (a program), Extract here and Extract to… (an archive); Cut, Copy, Duplicate, Rename; Move to…, Copy to…; Compress…; Customize…, Copy path, Copy name, Properties; Move to Trash, Permanently delete |
| A folder | The same, with Open in Terminal and Open in… (New Tab, New Window) for Quick preview, and Pin to sidebar before Customize… |
| Several entries | "N items selected" and their names; the creation rows; Open, Open With…, Open in… (with a folder among them); Cut, Copy, Duplicate; Move to…, Copy to…; Compress…; Copy paths, Copy names, Properties; the delete rows |
| A pane | New Folder, New File, Paste; Open With…, Open in Terminal; Select All, Refresh, Show or Hide Hidden Files; Customize…, Properties |

**Rows that act:** Open (as a click on the entry), Select All, Refresh, and Show or Hide
Hidden Files. Rename is wired to where V9 opens the editor, and logs until then.

**Rows that are shown as GTK shows them at rest and do nothing,** each waiting on a part
the slice leaves out: New Folder, New File, New Folder with Selection, Open With…, Quick
preview, Run, Open in Terminal, Open in… with New Tab and New Window, Extract here,
Extract to…, Cut, Copy, Duplicate, Move to…, Copy to…, Compress…, Pin to sidebar,
Customize…, Copy path, Copy name, Copy paths, Copy names, Properties, Move to Trash and
Permanently delete; in a pane's menu also Paste, which is greyed as GTK greys it with an
empty clipboard. Open on several entries opens the one under the pointer, where GTK opens
them all.

**Not ported,** and named in the module: the Trash and Recent variants, "Send to…", custom
actions and file-provider rows, the 10xer hints, each row's icon, and Open With…'s greyed
state while GTK looks for applications. **Not wired:** the Menu key and Shift+F10. C2's
columns install their own keyboard controller, and C21's `MenuKeys` has to wrap it there.

**The backend** (`cf936192a`). A menu is modal in Rune, and Rune's accessibility tree then
holds the menu alone, where the GTK tree keeps the panes: `right-click-selects-entry`
waits for the pane's selection with the menu open. While the tree leaves a column out, the
backend presents it from the drawn elements, name, selection and box, without the rows'
descriptions, which a drawn element does not carry. A menu row's keyboard shortcut is
presented as its description, as the GTK menu has it. The role names `menu` and `menu item`
were seen live; `check menu item` and `radio menu item` are still mapped by name only. The
engine did not change.

Tests: eight in `menus::tests` (the rows against the order two `dialogs_and_menus`
scenarios read, the hints, the catalogs), `qa::tree::tests::a_column_behind_an_open_menu_…`
and `qa::tests::a_right_click_opens_an_entry_menu_and_escape_gives_the_keyboard_back`,
which runs both variants of the scenario step for step.

### The Appearance popover: stopped

**Not mounted.** The ruling asked for it as a C21 menu and allowed a stop on this part if
mounting it properly needs an Out part or a Rune gap. It needs both.

- **It is not a menu.** `build_appearance_menu` (`src/ui/window/appearance_menu.rs`) fills
  a plain `gtk::Popover` with controls: three view-mode rows with the radio role, a
  "Preview panel" toggle button, two density sliders with their snap buttons, a text-size
  stepper, and three check rows. C21 draws rune-kit's `Menu`, whose rows are `menuitem` or
  `menuitemcheckbox` and nothing else: no radio row, no toggle button, no slider.
- **Its controls are Out parts' controls.** Of twelve, one acts on something the slice
  has, "Hidden files". The rest switch to Icons or List, toggle the preview panel, set the
  density and the text size of the look, group List's rows, or show Columns' size badges,
  which the adapter does not feed. Mounting them inert would be a stub that exists to
  satisfy two predicates, which the ruling rules out.
- **No work package composes it.** C20a has the slider "for the density sliders"; the
  header's button is X1's and inert. The popover itself is a host's composition that
  nobody has planned.
- **It would turn no run green.** All five runs that open it then press Ctrl+L and wait
  for the location field, which stays Out.

What it takes, for whoever plans it: the view switch (C3, C4), the preview toggle (C9), the
density and text size applied to the look, the popover with Escape and an outside press
closing it, a radio row (rune-kit's `MenuItem` has none), and in the backend the header
button as a `toggle button` with `pressed`, which Rune reports as `checked` on a button.
The five runs are `scope` pending with that blocker.

### The mutation patches

`991e931c8`. `./scripts/qa-parity.sh mutations` applies each patch under
`tests/qa/parity/mutations/` to this checkout, runs the required runs of the scenario the
patch is named for on the Rune target, and reverts it. A patch is caught when a run fails
a step and none fails to run, as `scripts/qa-mutation.sh` judges the GTK patches; it
shares that script's parsing and verdicts. `slice` runs them after its runs.

| Patch | Breaks, in `strata-app` | Caught at |
| --- | --- | --- |
| `columns-home-end-mirror.patch` (keyboard navigation) | `columns/mirror.rs`: the child column never follows the keyboard cursor | Line 3, "the child column to open" |
| `shift-click-range.patch` (selection) | `columns/selection.rs`: the cursor the browser places on a load never reaches the view, so a Shift click has no anchor to range from | Line 4, the range of three |

### The upstream request

[`docs/rune/upstream/rune/97-key-press-with-physical-key.md`](../upstream/rune/97-key-press-with-physical-key.md),
`1a70f7726`: a host that owns a `Runtime` cannot send a key press with its physical key.
Its headless repro was built against the pin and run. No shim is possible without
patching Rune: the entry that pairs the two is `pub(crate)`.

G2 has one non-US layout run, and it is `scope` pending for 10xer mode first. The `us`
layout step runs in three required runs.

## Done when, and the evidence

All on 2026-11-11. The pinned-image runs are from the worktree at
`~/dev/strata-next-verify`, in the unchanged base, with Docker; their logs and kept
artifacts are in its `target/v8-24fb0b9c6/` unless another folder is named.

| Done when | Met |
| --- | --- |
| The ledger covers all 42 runs | Yes: G1's run and G2's 41, each `required` or `pending` with a code and a blocker. The runner's ledger test passes in the full suite below |
| The `required` runs passed in three consecutive full runs on both targets | Yes: `./scripts/qa-parity.sh g2` three times running at `24fb0b9c6`, 22 of 22 on GTK and 22 of 22 on Rune each time (`qa-parity-g2-{1,2,3}.log`; bundles in `g2-{1,2,3}-gtk-artifacts/` and `g2-{1,2,3}-rune-artifacts/`, under `<scenario>/<run>/`). `./scripts/qa-parity.sh g1` passes on both (`qa-parity-g1.log`) |
| Both mutation patches fail their scenarios | Yes: `./scripts/qa-parity.sh mutations`, 2 of 2 caught (`qa-parity-mutations.log`, `qa-parity-mutation/summary.md`) |
| The stop condition was counted and is recorded, hit or not | Yes, [both ways](#the-stop-condition) |

| Test of the plan | Result |
| --- | --- |
| G2 on both targets, three times | Above |
| The ledger | Above |
| Mutations | Above |
| GTK unchanged, for the six folders' scenarios | The whole GTK suite ran instead, since the runner's ledger code changed: below |
| Unit tests, a test per fix | `mise run test-rune`, below |

**The GTK application and the runner.** `tests/qa/parity.rs` and its test changed, so the
shared-infrastructure checks ran, at `5118ca9b7` in `target/v8-5118ca9b7/` (the code under
`src/`, `crates/strata-qa` and `tests/` is the same at `24fb0b9c6`):

- `STRATA_QA_KEEP_ARTIFACTS=1 ./scripts/qa.sh`: 793 tests, 790 passed, 1 failed, 2 ignored,
  in 80 s (`qa-full.log`, `qa-full-artifacts/`). The one failure is
  `new-folder-with-selection-groups/columns`, at its wait for the opened folder's column to
  take focus: the flaky run [V3](V3-qa-backend-trait.md) and
  [V7](V7-rune-qa-backend.md) record, with the same failure. Run alone three times at `24fb0b9c6` it failed once the same way and passed twice (`target/v8-24fb0b9c6/flaky-new-{1,2,3}.log`); it was not chased.
- `./scripts/qa-mutation.sh` with the 21 patches that apply: 21 of 21 caught, in 11.6 minutes (`qa-mutation.log`, `qa-mutation/summary.md`).
- `./scripts/quality.sh fmt` and `./scripts/quality.sh clippy`: clean (`quality-fmt.log`,
  `quality-clippy.log`). `strata-desktop` lost one unused function and nothing else of the
  GTK lane changed, so `./scripts/quality.sh test` was not run.

**The Rune side,** on the host:

- `mise run test-rune` at `78ceea9e2`: 73 test binaries, 2,365 tests passed, none failed,
  253 ignored. That is V7's 2,337 and 253, plus 19 in `strata-app` (history 1, letters 2,
  saved 2, keys 2, menus 8, the snapshot 2, the backend 2), three in C1c and the six of
  the new `tests/c2d.rs`. After it, one snapshot test was split in two and a condition
  named (`24fb0b9c6`); `mise run test-rune qa::` (28 tests) and `mise run test-rune saved::` (2 tests) pass after it.
- Clippy on the Rune lane with warnings denied, with every feature and with none, and
  `cargo fmt --all --check`: clean at `24fb0b9c6`.
- `mise run test-platform window::`: three pass, at `78ceea9e2`. The windowed host takes its
  settings by value now and flushes a pending save when the window closes.
- `cargo tree -p strata-rune -i gtk4`, with and without `--features qa`: "package ID
  specification `gtk4` did not match any packages".
- Code health, through CodeScene's local tool since `CS_ACCESS_TOKEN` is not set: every new
  file scores 10; no touched file scores lower than at `2acdb32e8` (`columns/view.rs` 7.99
  and 7.99, `collection/keyboard/machine.rs` 8.64 and 8.69, `collection/keyboard/tests.rs`
  7.55 and 7.55, `collection/element/mod.rs` 6.31 and 6.31, the gallery's `scenes/c2.rs`
  7.2 and 7.2, `strata-app`'s `columns/mod.rs` and `columns/selection.rs` 9.68 and 9.68).
- `python3 -m unittest discover -s scripts -p 'test_*.py'`: 243 tests pass, four of them
  new. `typos` and `git diff --check` are clean.

Not run, and why:

- `./scripts/quality.sh test`: above.
- `./scripts/qa-parity.sh slice`: it runs what ran here one by one, and its whole-suite
  part inherits the flaky GTK run.
- `./scripts/qa-mutation.sh` with `cut.patch`: it does not apply, as before V3.
- A host QA run: no consent, and none needed.
- `mise run test-platform` beyond `window::`: `strata-platform` did not change.
- `cargo deny`'s advisories and licences: no package entered the lock (`strata-app` now
  names `toml`, which the workspace already had). Its `bans` and `sources`, which hold the
  layering, pass.
- CI: nothing was pushed by this package after `dc1f7ed9c`.

## Open

- **The Appearance popover** (decision 21) and **the Menu key** (decision 22).
- **18 `scope` runs** wait on parts of the interface, most of them on the location bar,
  the preview and 10xer mode.
- **The request upstream** for a key press with its physical key is drafted, not filed.
- **`check menu item` and `radio menu item`** are mapped by name and were not seen live.
- **A menu row's icon, and Open With…'s lookup state,** are not ported.
- **Descriptions behind an open menu.** The backend presents those rows without their
  "File" and "Folder" descriptions; a scenario that reads one there would not find it.
