# V8 Read-path parity

| | |
| --- | --- |
| Status | Not started |
| Depends on | V7 |
| Unblocks | V9 |
| Owns | `tests/qa/parity/slice.toml`, `tests/qa/parity/mutations/`, fixes in `crates/strata-app` |
| Lands as | Commits on `strata-next`: fixes, ledger updates, mutation patches |

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
| A non-US layout (`layout`) | `harness`, per owner decision 6 |
| A drag (`drag`) | `platform` when it is a file drag; a marquee drag is in scope |

## Scope

1. **Run the set on Rune.** `./scripts/qa-parity.sh g2`, with artifacts kept.
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
   passes unpatched and fails patched.
6. **Three clean runs.** The full `required` set passes three times in a row on both
   targets before the gate is called.

## The stop condition

If more than a third of G2's 42 runs end as `component` or `rune` pending, stop and report.
That is a finding about how ready the components are, not a reason to patch around them.
Count it as soon as the first sort is done, before fixing anything.

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
