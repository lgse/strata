# V8 Read-path parity

| | |
| --- | --- |
| Status | Stopped on 2026-11-11 at [the stop condition](#the-stop-condition), after the survey and the sort. Nothing was fixed, and the ledger is unchanged. It waits on owner decision 19 |
| Depends on | V7 |
| Unblocks | V9 |
| Owns | `tests/qa/parity/slice.toml`, `tests/qa/parity/mutations/`, fixes in `crates/strata-app` |
| Lands as | Commits on `strata-next`: fixes, ledger updates, mutation patches |

## Where it stopped

The first survey ran, every run was sorted, and the stop condition was counted before any
fix, as the plan asks. **20 of G2's 42 runs would end as `component` or `rune` pending,
which is more than a third (14).** The work stopped there.

What the count does and does not say:

- **It is a count of scope, not of readiness.** All 20 need a part of the interface the
  slice marks Out: the sidebar, the location bar, menus, the preview, dialogs, tabs, the
  List view, 10xer mode, a second window. Every one of those components is `merged` in
  [`STATUS.md`](../work/STATUS.md). `strata-app` does not mount them, by the slice's own
  scope. No run of the survey failed at a step where a component in `strata-components`
  lacks a behaviour or behaves differently, unless the row's missing label node is ruled
  one (decision 20). What those 20 runs do past their first failing step is unseen.
- **The codes are the plan's.** [The table above the scope](#the-set) says a run that needs
  one of those parts is likely `component`, and that is how the 20 were counted. The
  README defines `component` as "strata-components lacks or differs in the behaviour". By
  that wording alone the count is 1: the one run that is also `rune`. The six codes have
  none for "the slice does not mount it", and which reading the stop condition means is
  the owner's to say (decision 19 in the [README](README.md#owner-decisions)).
- **What does run, passes.** 17 of the 42 pass on the Rune target as V7 left it, G1's run
  among them, with nothing changed. The 16 new ones passed in three surveys out of three,
  and each agrees with the GTK transcript in every verdict field of every line.
- **Decision 11 holds.** The risk this plan names, many small selection failures, did not
  show. Of the 11 runs of `selection`, `escape_selection` and `background_selection` that
  need nothing outside the slice, 10 pass, and the eleventh fails where the harness looks
  for a row's label, before it selects anything.

If the work goes on as scoped, G2 ends with 20 or 21 runs `required` and 21 or 22
`pending`: the 17 that pass, three that need a fix in `strata-app`, and one that needs a
ruling.

### The survey

At `2acdb32e8`, in the pinned image, from the worktree at `~/dev/strata-next-verify`:
`./scripts/qa-parity.sh g2 --pending`, three times. Each time 16 of the 41 pending runs
pass and 25 fail, the same 16 and the same 25, each failing at the same line. Logs and
kept artifacts are in that worktree's `target/v8-2acdb32e8/`: `survey-{1,2,3}.log` and
`survey-{1,2,3}-rune-artifacts/<scenario>/<run>/`.

The 16 transcripts were compared with the GTK ones V7 kept from its full run at
`e97b84e0b` (`target/v7-e97b84e0b/qa-full-artifacts/`): the same number of lines, and the
same `ok`, `error`, `failures` and `criticals` on each. No scenario file changed between
the two commits.

### The sort

Line numbers are the scenario file's, as `failures.txt` gives them. "Counted" is the stop
condition's count.

**Pass on Rune as it is (17).** Each goes to `required` once it has passed on both targets
from `./scripts/qa-parity.sh g2`.

| Folder | Runs |
| --- | --- |
| `keyboard_navigation` | `arrow-keys-move-focus/columns-arrows` (G1, already `required`), `columns-home-end-mirror/columns`, `select-all-selects-every-entry/columns`, `shift-arrow-extends-selection/columns` |
| `selection` | `control-click-sets-shift-anchor/columns`, `home-end-page-move-shift-anchor/columns`, `shift-click-range/columns` |
| `escape_selection` | `enter-after-escape-opens-folder/columns`, `escape-only-clears-active-column/columns`, `shift-after-escape-starts-on-focused/columns-no-range`, `shift-after-escape-starts-on-focused/columns-had-range` |
| `background_selection` | `background-click-clears-selection/columns`, `background-press-clears-before-drag/columns`, `beside-last-column-clears/columns` |
| `startup_arguments` | `startup-with-closed-pipes/columns-stdout`, `columns-stderr`, `columns-both` |

**Fail for something the slice should have (3).** Each is a fix in `strata-app` with an
in-process test. None was made.

| Run | Fails at | Cause | Fix |
| --- | --- | --- | --- |
| `alt-up-and-history/columns` | Line 7: after Alt+Up the directory is still `documents` | The window handles no Alt+Up, Alt+Left or Alt+Right | Take the three keys to `Browser::parent`, `back` and `forward`. The run then checks that the child column stays closed and the folder left is selected, which is `Browser`'s to place |
| `arrow-keys-move-focus-remaining/columns-hjkl` | Line 7: `j` moves nothing | The header sets `type_to_search` to false, where `h j k l` are the arrows. The component has that (`Letters::Arrows`); the adapter passes the view no key settings | Read the saved `type_to_search` and pass `KeySettings` through `ColumnsView::collection` |
| `hidden-files-persist-across-restart/columns` | Line 8: `show_hidden` is not saved | `strata-rune` saves nothing ([V6](V6-columns-adapter.md#open)) | Save `show_hidden` when it is toggled, through `strata_desktop::settings` |

**Needs a ruling (1).**

| Run | Fails at | Cause | Ways on |
| --- | --- | --- | --- |
| `leftover-click-honors-modifiers/columns` | Line 3: "no name label on pictures" | A click on an entry's `part: label` looks for a `label` named by the entry under the row. GTK's row has one. Rune's row is one accessible node, and its name part is hidden from the tree | `harness` is not open: the backend reads `Runtime::accessibility_json`, which has no box for the name. Either the row exposes its name as a label node (a C1e change that touches every collection golden, as decision 5's did for C2), or the run is `toolkit` pending: it aims two points inside the right edge of GTK's label, and could be reworded to aim at the row's end. Two of the counted runs below use the same target |

**Needs a real clipboard (1).** `platform`, not counted.

| Run | Fails at | Needs |
| --- | --- | --- |
| `keyboard-only-copy-paste/columns` | Line 13: the pasted file never appears | Ctrl+C and Ctrl+V. The clipboard is Out, and the offscreen host has none (P2 and X3 hold the pieces) |

**Needs a part the slice marks Out (20, all counted).** The code is the plan's likely one,
`component`, pointing at the work package whose component `strata-app` does not mount.
"Also" lists what the rest of the file needs, since the first failing step hides it.

| Run | Fails at | Needs first | Also |
| --- | --- | --- | --- |
| `arrow-keys-move-focus/columns-hjkl-tenxer-ru` | Line 7: `j` under `ru` moves nothing | 10xer mode (`tenxer_mode`; C1c's 10xer map) | `rune` as well: a plain-letter binding under a non-Latin layout needs a key press with its physical key, and `Runtime::physical_key_input` is not public at the pinned rev ([V7](V7-rune-qa-backend.md#decision-6-a-layout-on-a-target-with-no-display)). The request upstream is not written yet |
| `tenxer-keeps-location-edit/columns` | Line 9: no field after Ctrl+L | The location field (C6) | 10xer mode |
| `tenxer-escape-closes-peek/columns` | Line 6: Space does not move the cursor on | 10xer mode (C1c) | Nothing else in a Columns run |
| `tenxer-escape-filter-preview-fill/columns` | Line 4: no field after `f` | 10xer mode and the filter (C18) | The preview (C9) |
| `back-then-file-keys-target-focused-folder/columns` | Line 10: no sidebar button "Home" | The sidebar (C8) | Alt+Left, F2 (V9's editor), Delete to the trash |
| `sidebar-navigation-range-anchor/columns` | Line 5: no sidebar button "Home" | The sidebar (C8) | Nothing |
| `columns-return-selects-source-folder/columns` | Line 15: no breadcrumb button | The location bar's crumbs, pressed (C6). The slice allows a location bar in its static form only | BackSpace to the parent |
| `focus-usable-after-view-change/columns` | Line 5: Ctrl+3 does not switch | The List view (C4) | Tabs (C7), the Icons view (C3) |
| `control-click-toggles/columns` | Line 2: no toggle button "Appearance" | The Appearance menu (C21 over X1's header, whose button is inert and a plain `button`) | The preview's toggle in it (C9), the location field (C6) |
| `parent-pane-anchors-first-entry/columns` | Line 2, the same | The same | The same |
| `empty-column-clears-other-columns/columns` | Line 2, the same | The same | The same |
| `shift-click-revisits-after-folder/columns-name` | Line 2, the same | The same | The same, and `part: label` |
| `shift-click-revisits-after-folder/columns-row-space` | Line 2, the same | The same | The same, and `part: label` |
| `right-click-selects-entry/columns-file` | Line 5: no menu after a right click | The context menu (C21) | Nothing |
| `right-click-selects-entry/columns-folder` | Line 5, the same | The same | Nothing |
| `escape-dismisses-transient/columns` | Line 4: no menu after a right click | The context menu (C21) | The properties dialog (C15), the location field (C6), the filter (C18), the rename editor (V9), the preview (C9), a new folder |
| `disabled-transport-click/columns-audio` | Line 4: no transport button in a preview | The preview drawer and the media player (C9, C13) | Nothing |
| `disabled-transport-click/columns-video` | Line 4, the same | The same | Nothing |
| `preview-blank-click-keeps-selection/columns` | Line 5: no preview | The preview drawer and the image viewer (C9, C14b) | Nothing |
| `multiple-arguments-non-utf8/columns` | Line 10: the `launch` of five arguments exits with 2 | A window for each argument, a file argument revealed, and the session bus (P8). `strata-rune` takes one directory | Nothing |

The five runs that open the Appearance menu do so only to close the preview panel and
then put the keyboard back in the listing, before the clicks they are about. On a target
with no preview panel that preamble has nothing to do, and those five and the two
right-click runs are where a reworded scenario or a small menu would buy the most runs.
Neither is this package's to do.

### The count, three ways

| Reading | Counted | Of 42 | Over a third |
| --- | --- | --- | --- |
| The plan's likely codes: a run that needs a part the slice leaves out is `component` | 20 | 48% | Yes |
| The same, with the runs that need only a menu as `harness`, which the plan's table allows: the two right-click runs. The other six menu runs need the preview, a dialog or the location field as well | 18 | 43% | Yes |
| The README's wording: `component` only where `strata-components` lacks or differs in the behaviour, or `rune` | 1 | 2% | No |

The label node of the run that needs a ruling would add one to each row if it were ruled a
component change.

### Not done

Everything after the count: no fix in `strata-app`, no backend addition, none of decision
17's six component changes (the owner ruled on 2026-11-11 that all six match GTK; their
GTK behaviour is not verified yet), no ledger change, no mutation patch and no `mutations`
selection, no upstream request for the physical key, no run of `./scripts/qa-parity.sh g2`
on both targets. `tests/qa/parity/slice.toml` still lists G2's 41 runs as `harness`, "not
yet attempted", which is no longer true of any of them: the sort above is what replaces
it when the work resumes. No file outside `docs/rune/slice/` changed, so nothing was built
or linted beyond the three surveys.

### To resume

The owner rules on decision 19. If the slice goes on as scoped, the order is the plan's:
promote the 17, make the three fixes, rule on the label, write the 21 or 22 pending
entries from the sort, then decision 17, the two mutation patches and the three clean
runs.

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
| A context menu (`open_menu`, `menu_items`, `menu_item`) | `component` or `harness`, pointing at the menu work package |
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

**Counted on 2026-11-11: hit, on the plan's own codes.** 20 of 42. See
[where it stopped](#where-it-stopped) for what the 20 are and for the reading under which
the count is 1.

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
