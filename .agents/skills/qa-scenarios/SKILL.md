---
name: qa-scenarios
description: >
  Create, edit, debug, run, rename, split, or remove Strata's QA scenarios
  (tests/qa/scenarios/*.jsonl, run by tests/qa.rs through ./scripts/qa.sh),
  and work with the `strata --qa` JSON-lines protocol behind them. Use for any
  task that writes or changes a scenario file or its header (modes, variants,
  cost, quality, skip, known_bug), reads a QA failure or its artifacts in
  target/qa-artifacts, or changes the QA runner.
---

# QA scenarios

QA scenarios are Strata's GUI test suite. Each scenario is one `.jsonl` file
that drives the real app with real X input and checks what a user would see.
This file covers the common 80%; [reference.md](reference.md) has every
header key, step, target, predicate, and `fs` kind, and is the only catalogue
of them. The "QA scenarios (`--qa` mode)" section of `docs/testing.md` covers
the run environment, keyboard layouts, scheduling, visual baselines, and
mutations; its "Skipped scenarios" and "Waits for the app's own timers" lists
are part of the suite.

## How it fits together

- **The app side.** A build with the `qa` cargo feature (never released)
  accepts `strata --qa <directory>`: the normal window opens on the directory,
  then `src/qa.rs` reads one JSON command per line on stdin, runs it on the GTK
  main loop (`src/qa/dispatch.rs`), and writes one reply per line on stdout once the main context
  drains. Input is XTEST sent from inside the process; state is read from the
  widget tree by the accessible names `src/ui/accessibility.rs` sets.
  `strata --qa --portal` runs the FileChooser portal backend instead.
  `src/qa/protocol/` parses commands; `src/qa/targets.rs` and its
  `targets/` submodules resolve targets; `state.rs` builds the snapshot.
- **The reply.** `{"ok","app","changed","criticals","result","failures","error"}`.
  `app` is the state snapshot (view mode, directory, panes with selection and
  focus, focused entry, dialog, menu, preview, field, matches). Any GLib
  warning or critical in `criticals` fails the run.
- **The runner.** `tests/qa.rs` and its modules in `tests/qa/` (custom harness `tests/qa/harness.rs`,
  `harness = false`) loads every file, checks every step with the process's
  own parser before launching anything, and makes each run a test named
  `<scenario>/<run>`. Each run gets its own Xvfb, HOME, XDG directories, and
  fixture tree from `tests/fixtures/harness.json`. Runner-side steps (`fs`,
  `restart`, `layout`, peers in `tests/qa/peers.rs`) execute in the runner;
  the rest go to the process. Costliest runs start first.
- **The container.** `./scripts/qa.sh` runs
  `cargo test --features qa,invariant-checks --test qa` inside the pinned image.
  `tests/qa.rs` refuses to start without the `STRATA_QA_CONTAINER=1` it sets.
- **The files.** `tests/qa/scenarios/<folder>/<scenario>.jsonl`, one scenario
  per file. Line 1 is the header; every later line is one step.

## Run

```bash
./scripts/qa.sh                                     # every scenario, every run
./scripts/qa.sh hidden-files-persist-across-restart # all runs of one scenario
./scripts/qa.sh hidden-files-persist-across-restart/list   # one run
./scripts/qa.sh a b --test-threads 4                # several scenarios, 4 at once
./scripts/qa.sh --skip copy-leaves-source           # everything but one scenario
STRATA_QA_KEEP_ARTIFACTS=1 ./scripts/qa.sh <name>   # artifacts for passing runs too
```

- A name selects exactly that scenario (never another it prefixes);
  `<scenario>/<run>` selects one run. `_` in a name reads as `-`.
- `--skip <scenario>` or `--skip <scenario>/<run>` leaves exactly that out.
- `--list` prints the selected tests in start (cost) order. `qa.sh` then
  exits 1 with "The selection ran no QA scenario", because nothing ran.
- A run prints `RUN <scenario>/<run>` up front, then `test <name> ... ok
  <secs>` and `PASS <scenario>/<run>` (or `KNOWN BUG ...`) as each finishes,
  failure details at the end, and libtest's `test result:` line.

Container rules:

- Use Docker or Podman, whichever the scripts select (Podman whenever it is
  installed); `STRATA_CONTAINER_ENGINE` overrides it. Run container scripts (`scripts/qa.sh`, `scripts/qa-mutation.sh`, `scripts/quality.sh`,
  and the like) one after another, never concurrently: some share a build
  cache, and all of them load the whole machine.
- Never run scenarios on the host. `STRATA_QA_HOST=1` needs the owner's
  explicit consent for that task, is labelled unsupported, is never
  validation evidence, and goes in the handoff with the container run it
  replaced.

## Write a scenario

1. **Find the coverage owner.** `grep -rl` the scenarios for the feature's
   keys, labels, and preferences. Extend an existing scenario (a step, a
   mode, a variant) rather than adding a near-duplicate. Add a new file only
   for behaviour nothing covers.
2. **Choose the folder and name.** A new scenario lives in a feature-area
   folder named in snake_case without a `test_` prefix
   (`tests/qa/scenarios/hidden_files/`); reuse an existing folder when one
   fits. The file stem is the `scenario` name: lowercase words joined by
   hyphens, unique across all folders, saying the behaviour
   (`hidden-files-persist-across-restart`). The runner's
   `every_scenario_file_is_planned_and_well_formed` test enforces both.
3. **Write the header.** Minimum: `{"scenario":"<name>","modes":[...]}`. Add
   `modes` only where the view changes the behaviour; add `prefs`, `tree`,
   `setup`, `variants`, `session_bus`, `env` only as needed. Leave out
   `cost` until it is measured.
4. **Build the fixture.** The default tree (`canonical`) is:
   `archive/`, `documents/{notes.txt,report.md,spreadsheet.csv}`,
   `pictures/{diagram.txt,photo.txt}`, `readme.md`, `todo.txt`, `.hidden.txt`.
   Prefer it. Add files with a header `tree`/`images`, `setup` `fs` actions,
   or an `fs` step before the input that needs them. Paths are relative to the
   fixture; `$fixture`, `$root`, `$home`, `$config`, `$data`, `$cache`,
   `$environment`, `$python`, `$view`, and `$mode` are built in.
5. **Drive input, then wait on observable state, never on time.** After every
   key, click, or drag, `wait` on what the user would see change (entries,
   selection, focus, a dialog, a menu, a field, a preference on disk, a file
   on disk) before the next input. Never add a `sleep` to make a step pass.
6. **Give every wait a `message`.** Say what is awaited, in words, so a
   timed-out wait reads `timed out after 15000 ms: <message> expected
   <predicate>, actual <value>`: `"Ctrl+. to show hidden files"`, `"the rename editor to take focus"`. Give `expect` and `fs` checks
   a message too, unless it asserts at once (`"timeout_ms":0`) right after a
   wait that already explains the state.
7. **Assert behaviour, not layout.** Check names, selection, focus, dialogs,
   menus, files, preferences, and text. Do not assert widget geometry,
   alignment, spacing, sizes, wrapping, or pixel placement; geometry
   predicates are for behaviour that is positional (a scroll reached its
   target, a drag landed, a repaint happened, a committed visual baseline).
8. **Target by accessibility.** Copy roles and names from a `tree` query.
   Entry rows are labelled with their name and described `Folder` or `File`;
   panes are labelled with their directory; menu items carry the accelerator
   in their description; dialogs carry the dialog role and a name; a button
   with a text label is named by it. Tooltips never name a target. If a
   control has no usable name, name it in the app (`accessibility::set_label`
   for an icon-only button); never reach around the accessibility layer.
9. **Run it** by name until it passes, then run each of its runs a few times
   and once inside a full `./scripts/qa.sh`, which runs it under load.
10. **Prove it can fail.** Break the behaviour temporarily (change a step's
    expected value, or revert the app change) and confirm the failure names
    the right step; restore it.
11. **Set `cost` and `quality`** (see below).

`wait` stops the run on timeout; `expect` records a mismatch and continues
(use it for several independent assertions about one state). `select`
clicks until an entry is selected; `select_with_keyboard` walks the cursor to
it. `settle` waits for a target's box to stop moving before a click that must
land on it.

## Examples

All three pass in the pinned container.

### A QA-only scenario: a preference survives a restart

`tests/qa/scenarios/hidden_files/hidden-files-persist-across-restart.jsonl`:

```json
{"scenario":"hidden-files-persist-across-restart","modes":["columns","list"],"cost":{"columns":0.5,"list":0.6}}
{"wait":{"entries":{"contains":["todo.txt"],"excludes":[".hidden.txt"]},"dir":"$root"},"message":"the fixture listed without its hidden file"}
{"key":"ctrl+period"}
{"wait":{"entries":{"contains":[".hidden.txt"]},"dir":"$root"},"message":"Ctrl+. to show hidden files"}
{"wait":{"pref":{"key":"show_hidden","value":true}},"message":"show_hidden to be saved as true"}
{"restart":true}
{"wait":{"entries":{"contains":["todo.txt",".hidden.txt"]},"dir":"$root"},"message":"hidden files to stay shown after a restart"}
{"key":"ctrl+period"}
{"wait":{"entries":{"contains":["todo.txt"],"excludes":[".hidden.txt"]},"dir":"$root"},"message":"Ctrl+. to hide hidden files again"}
{"wait":{"pref":{"key":"show_hidden","value":false}},"message":"show_hidden to be saved as false"}
```

It waits for the listing before the first key, waits on the saved preference
before restarting (so the restart cannot race the write), and checks the
listing after the restart.

### Pointer, context menu, and an inline editor

The steps of `tests/qa/scenarios/entry_management/rename-from-context-menu.jsonl`:

```json
{"click":{"entry":"readme.md"},"button":3}
{"wait":{"menu_items":{}},"message":"the context menu for 'readme.md'"}
{"wait":{"present":{"menu_item":"Rename"}},"message":"the menu item 'Rename'"}
{"settle":{"menu_item":"Rename"}}
{"click":{"menu_item":"Rename"}}
{"wait":{"menu_items":null},"message":"the menu to close"}
{"wait":{"field":{}},"message":"an editable field to take focus"}
{"remember":{"field":true},"as":"field"}
{"key":"ctrl+a"}
{"type":"guide.md"}
{"wait":{"field":{"of":"field","text":"guide.md"}},"message":"the new name to be typed"}
{"key":"Return"}
{"fs":{"path":"guide.md","exists":true},"message":"the rename to apply"}
{"fs":{"path":"readme.md","exists":false},"timeout_ms":0}
```

It settles the popover's item before clicking it, remembers the
editor so later checks read that field, and checks the result on disk.

### Every view, and `expect` versus `wait`

The steps of `tests/qa/scenarios/selection/right-click-selects-entry.jsonl`
(modes `columns`, `icons`, `list`; variants `file` and `folder` set `$target`
and `$previous`):

```json
{"select":{"entry":"readme.md","dir":"$root"}}
{"wait":{"focused":"readme.md"},"message":"keyboard focus to reach 'readme.md'"}
{"click":{"entry":"$target","dir":"$root"},"button":3}
{"wait":{"menu_items":{}},"message":"the context menu for '$target'"}
{"wait":{"selected":["$target"],"dir":"$root"},"message":"the selection to become ['$target']"}
{"key":"Escape"}
{"wait":{"menu_items":null},"message":"the menu to close"}
{"wait":{"focused":"$target"},"message":"keyboard focus to reach '$target'"}
{"expect":{"panes":["$root"]},"message":"a folder context menu must not navigate"}
{"key":"Left","modes":["icons"]}
{"key":"Up","modes":["columns","list"]}
{"wait":{"focused":"$previous"},"message":"keyboard focus to reach '$previous'"}
{"wait":{"selected":["$previous"],"dir":"$root"},"message":"the selection to become ['$previous']"}
```

The `expect` asserts that the menu left the listing where it was without
stopping the run, so the steps after it still report; the `modes` modifiers
send the key that moves back one entry in each view.

## Header fields you will use

| Key | Use |
| --- | --- |
| `modes` | `columns`, `icons`, `list`; one run each. The run's mode is written as `browser_mode` over every `prefs`. |
| `prefs` | `settings.toml` overrides, for example `{"type_to_search":false}`. |
| `variants` | `[{"id":"ru","layout":"ru","vars":{"key":"value"},"prefs":{...},"modes":[...]}]`: reruns, labelled `<mode>-<id>`; steps opt in with `"variants":["ru"]`. |
| `tree`, `images`, `setup`, `teardown` | Fixture contents, written before launch; `teardown` runs however the run ends. |
| `session_bus`, `env`, `portal`, `fixture_root` | A private D-Bus; extra app variables; the portal backend; a fixed `/tmp/<name>` fixture path. |
| `input_method` | A private fcitx5 with that input method (host runs only; see `docs/testing.md`). |
| `known_bug`, `known_bug_step` | An issue and the line expected to fail. |
| `mutation` | A patch in `tests/qa/mutations/` the scenario must catch; check it with `./scripts/qa-mutation.sh <scenario>`. |
| `skip` | A reason; keeps the scenario out of the default run. |
| `cost` | Measured start-order weights per run label. |
| `quality` | The scenario's value rating. |

Step modifiers: `"modes":[...]`, `"variants":[...]`, `"when":PREDICATE`,
`"if"`/`"unless"` (flags kept by `expect`/`flag` with `"as"`),
`"message"`, `"timeout_ms"`.

## Keep `cost` and `quality` current

- **`cost`** orders the start of runs (costliest first; a run without one
  starts first). After adding scenarios or runs, refresh it:
  `python3 scripts/qa_cost.py run` (three full container runs, then rewrites
  only each header's `cost`), or `python3 scripts/qa_cost.py write
  target/qa-cost/run-*.log` from saved logs. The runner rejects a `cost` that
  misses a run label or names another, so update it when you add, rename, or
  drop a mode or variant.
- **`quality`** is
  `{"impact","coverage","criticality","diagnostic","score","note"}`; the
  runner checks the shape and recomputes `score`. Use the `qa-quality` skill
  to rate a scenario and compute the score, including its independent
  cheap-model grading; re-rate when the scenario's behaviour or coverage
  changes.

## Manage scenarios

- **Rename:** `git mv` the file, change its `scenario` key to the new stem,
  and replace the old name everywhere it appears (`grep -rn <old> docs tests
  .agents`), including the "Skipped scenarios" and "Waits for the app's own timers" lists.
  Cost labels are run labels, so they stay.
- **Split:** give each new file its own header and only the steps its
  behaviour needs; keep every meaningful assertion; refresh `cost`; rate each
  file's `quality`.
- **Remove:** only when another scenario covers the same behaviour (name it
  in the commit message) or the behaviour is gone. Delete the file and every
  mention of its name.
- **`skip`** is only for a scenario whose pass or fail the app leaves to
  chance, until the app is fixed: file the issue, put it in the reason, and
  add the scenario with its evidence to "Skipped scenarios". It still runs
  when named.
- **`known_bug`** is for a reproducible app bug that the scenario documents:
  set the issue and the exact failing line. The run then passes only while
  that step fails; once the bug is fixed the run fails with "remove the
  marker". Never use it for a flake.

## Flakes and races

- Never add `sleep` or a longer timeout to hide a race. Reproduce first:
  run the one run repeatedly (`for i in $(seq 20); do ./scripts/qa.sh
  <scenario>/<run> || break; done`) and inside full runs, which add load.
- If the scenario raced the app (input before the state it depends on), add
  a wait on that observable state.
- If the app itself drops or misroutes input for a short window (a race in
  the app), file an issue; do not work around it silently.
- A wait that exists only because QA input is faster than a person can be
  (for example waiting for the column the cursor mirrors before a shortcut)
  is allowed only with evidence: add the scenario, run, what it waits for,
  the app race, and failure counts with and without the wait to "Waits for the
  app's own timers", and file an issue for the race.

## Debug a failure

A failed run prints `<scenario>/<run> failed:` with each failure as
`line N {step}: <error>` and `artifacts: .../target/qa-artifacts/<scenario>/<run>`
(the path is the container's `/workspace`, which is the checkout).
`could not run:` means setup, launch, or a harness error, which no
`known_bug` excuses. A malformed step fails before launch with
`line N: <parser error>`.

The artifact directory holds `failures.txt`, `transcript.jsonl` (every
command with its reply and `app` snapshot), `tree.txt`, `state.json`,
`screenshot.png`, `strata.log`, `input.log`, `fixture.txt`, `xvfb.log`, and
`result.json`. Pass `STRATA_QA_KEEP_ARTIFACTS=1` to keep them for a passing
run.

- **A wait timed out.** The line reads `timed out after 15000 ms: <message>
  expected <predicate>, actual <value>`. Compare `actual` with what you
  expected; then read the `app` snapshot in `transcript.jsonl` for the steps
  before it to find where the state diverged. Check `screenshot.png` for an
  unexpected dialog or menu, and `strata.log` for errors. If the state is
  right but named differently, fix the predicate from `tree.txt`.
- **A target was not found.** Search `tree.txt` for the role and name. Names
  come from labels and `accessibility::set_label`, never tooltips; a widget
  scrolled away is not `present` (use `reveal`, or `"rendered":false` where
  presence is enough).
- **Focus is not where expected.** `tree.txt` starts with `focus: ...`,
  marked when focus sits on an unmapped widget; such focus shows as `null`
  in the snapshot. Wait for `{"focused":...}` or `{"focused_widget":...}`
  before sending keys, since keys go to the focus.
- **A click was lost.** A listing that replaces a row mid-press cancels the
  click. Wait for the listing to finish first: after a reload,
  `remember` the row before the reload and `wait` for
  `{"replaced":{"target":...,"since":...}}`; otherwise wait for the row's own
  final state, or `settle` it. If no observable state marks the end of the
  refresh, that is an app race: file an issue.
- **See what the snapshot holds at any point.** Add a temporary
  `{"query":"state"}` or `{"query":"tree"}` step, run with
  `STRATA_QA_KEEP_ARTIFACTS=1`, and read the `result` in
  `transcript.jsonl`; remove the step afterwards. To explore interactively,
  use the `qa-drive` skill.
- **A GLib warning failed the run.** The failure names it; it is an app bug,
  not a scenario bug.

## Change the runner

Runner unit tests live in `tests/qa/scenario_tests.rs` and
`tests/qa/step_tests.rs` and must be listed in the `unit_tests!` macro in
`tests/qa/unit_tests.rs`: the custom harness ignores `#[test]` and
`#[cfg(test)]`, and `no_test_attribute_hides_a_runner_test` rejects them.
Unit tests run in every `./scripts/qa.sh` run that names no scenario. Script
tests: `python3 -m unittest discover -s scripts -p 'test_*.py'`.

The main-thread I/O audit also runs through the runner: with
`STRATA_QA_MAIN_IO` set, `tests/qa/main_io.rs` starts every Strata launch
under `strace` and writes the `meta.json` that `scripts/trace-main-io.py`
classifies paths by. When you change how the runner launches or restarts the
app, starts peers, or places its own files, run
`./scripts/trace-main-io.py <scenario>` on an affected scenario and check its
traces still appear. See "Main-thread I/O audit" in `docs/testing.md`.

## Before committing a scenario change

- [ ] The file is in the right folder, its stem equals `scenario`, and the
      name says the behaviour.
- [ ] Every input is followed by a wait on observable state; every wait and
      polling check has a `message`; no `sleep` was added.
- [ ] Assertions test behaviour, not layout or geometry.
- [ ] Targets use accessible names; any missing name was added in the app.
- [ ] Each run passes by name, repeatedly, and in a full `./scripts/qa.sh`;
      the scenario was shown to fail when the behaviour breaks.
- [ ] `cost` covers exactly the run labels; `quality`, if present, was
      re-rated with the `qa-quality` skill and checked by its independent
      cheap-model grader.
- [ ] `skip`, `known_bug`, and any wait for the app's own timers carry an
      issue and are listed in the guide.
- [ ] A scenario with a `mutation` still catches it:
      `./scripts/qa-mutation.sh <scenario>`.
- [ ] Renamed or removed names are gone from `docs/`, `tests/`, and the
      skills.
- [ ] `git diff --check` is clean.
