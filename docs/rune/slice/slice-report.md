# Slice report: Columns on Rune with QA-harness parity

| | |
| --- | --- |
| Work package | [V10](V10-slice-report.md), the last of the [slice](README.md) |
| Written | 2026-11-11, at `d4f086f25` |
| Exit command | `./scripts/qa-parity.sh slice` exits 0 at `d4f086f25` ([the run](#the-exit-check)) |
| Evidence | `~/dev/strata-next-verify/target/v10-d4f086f25/`, and the folders [each package names](#where-the-evidence-is) |

The slice set out to have one command run the same QA scenario files against the GTK binary
and a new Rune binary, and have both pass. That command exists and exits 0.

- **What runs.** `strata-rune --qa <directory>` speaks Strata's QA protocol. It shows the
  Columns view of a real directory, fed by Strata's own `Browser`, with keyboard navigation,
  selection, the context menus and inline rename.
- **What passes.** The parity set is 75 Columns runs in seven scenario folders. 36 are
  `required` and pass on both targets, in each of the last three runs of the exit command.
- **What does not.** The other 39 are `pending`: 35 need a part of the interface the slice
  left out (`scope`), three need a real clipboard or input method (`platform`), and one waits
  on Rune (`rune`). None is pending for a component that lacks a behaviour, for a step the
  harness lacks, or for a ruled deviation.

This report gives the owner what the decision on the slice needs. It makes no recommendation.

## What the slice does not prove

Read these before any number below.

- **Input never crossed winit or a compositor in the parity runs.** The Rune target is an
  offscreen host. Keys and pointer events are injected at Rune's input layer, the state is
  read from Rune's accessibility tree, and nothing is drawn on a display. Focus as a
  compositor hands it out, real drag and drop, the clipboard, input methods, window
  decorations and the picture itself are untested by parity.
- **The harness reads the view, not `Browser`.** A QA run on the Rune target sees what the
  Columns view shows. [V7](V7-rune-qa-backend.md#what-v7-found) broke what the adapter tells
  `Browser` and gate G1 still passed. The in-process tests of
  [V6](V6-columns-adapter.md#tests) and [V9](V9-rename-parity.md#the-wiring) are the guard:
  they compare the view with `Browser` after every key and every rename. Of the gates, only
  G3's runs see `Browser` from outside, because a rename acts on the entry `Browser` holds
  and the runner then reads the folder on disk.
- **Two required runs pass on Rune for a weaker reason than on GTK.**
  `window-accelerators-leave-rename/columns-ctrl-backslash` and `columns-ctrl-k` check that a
  window shortcut leaves an open rename editor alone. They pass on Rune because `strata-app`
  binds neither Ctrl+\ nor Ctrl+K yet. The guard itself is held in process with four keys the
  window does bind ([V9](V9-rename-parity.md#the-sort)).
- **One hand check is the only evidence from a real compositor.** The owner checked the
  windowed binary [by hand on Hyprland](#the-hand-check). No other compositor was checked.
  Three compositor tests drive the window in a private headless sway, where it lists its
  folder, takes a key and stays on time under a cover; they run locally, and nothing in them
  looks at the picture.
- **More than half of the set is outside the slice.** 18 of the read path's 41 runs and 17 of
  the write path's 33 are `scope` pending.
- **The backend presents two things Rune's tree leaves out,** from the drawn elements: a row's
  name as its `label`, and the panes behind an open menu. A screen reader hears neither.
- **No CI job compiles, lints or tests the Rune side** (decision 13, open). The parity
  command runs locally, in the pinned image.
- **The measurements compare a window with no window.** The GTK figures include a window on
  Xvfb; the Rune figures are the offscreen host's, which opens none
  ([Measurements](#5-measurements)).

## The exit check

`./scripts/qa-parity.sh slice`, alone, in the pinned image, from the verification worktree
`~/dev/strata-next-verify` detached at `d4f086f25`, on 2026-11-11 (23:39 to 23:53 UTC). The
commits after `d4f086f25`, this report's among them, change documents only.

| Part | Result |
| --- | --- |
| The gates on GTK | 36 of 36 required runs pass |
| The gates on Rune | 36 of 36 required runs pass |
| The runner's tests and the whole GTK suite | 795 tests: 793 passed, none failed, 2 ignored, in 66 s. That is the runner's 35 unit tests, the ledger's among them, and 758 scenario runs |
| The Rune mutation patches | 3 of 3 caught, in 1.1 minutes |
| The GTK mutation patches | 21 of 21 caught, in 11.0 minutes; `cut.patch` is stale still |
| Exit status | 0, after 13 min 43 s |

The command's closing lines:

```text
== allowances, from tests/qa/parity/slice.toml ==
ALLOWANCE USED: cut.patch does not apply, as before the slice; the other 21 GTK mutation patches run.
== slice exits 0 ==
```

- **The flaky run did not fail in this run.** `new-folder-with-selection-groups/columns`
  passed in the whole-suite part, so its allowance was not used and no bundle was kept under
  `target/qa-parity/allowance/`. No real run of the exit command has met the flake
  since the owner ruled the run allowed outright (decision 27); a rehearsal against a
  stand-in for the suite shows that path
  ([V9b](V9b-test-image-and-exit-check.md#decision-27-as-ruled)).
- **The image.** The scripts resolved the base built on this host in V9b for the key
  `4fa6c8cf…`: `strata-test:base-4fa6c8cf7d7c…-1000-1000`, image
  `sha256:1a3fac7a69070751a7b1f93aacc343930535c55fc6c13cbdfbb7ac94e22ccb87`. The base
  published for the same key (publisher run 43117209452) was not pulled, because a local one
  exists. The cache was warm: the first line is `Dependencies: reusing the Cargo cache in
  /workspace/target/qa-container; nothing is copied.`
- **The engine** is Docker; this host has no Podman.

Kept in `target/v10-d4f086f25/`: `qa-parity-slice.log`, `slice-run-context.txt` (the commit,
the times and the image), `qa-parity/allowances.txt`, `qa-parity-mutation-summary.md` and
`qa-mutation-summary.md`.

**After the index was closed.** The runner's ledger test reads `docs/rune/slice/README.md`,
which the commit that closes the slice's index edits. That README was checked before it was
committed: at a tree holding it as committed, `./scripts/qa.sh` exits 0 with 793 passed,
none failed and 2 ignored, the runner's 35 tests and the ledger's two among them
(`target/v10-index/qa-full.log`). The exit command was not run again.

## 1. The parity matrix

Generated by `parity-matrix.py` in `target/v10-d4f086f25/`, from
`tests/qa/parity/slice.toml` and four logs:

| Log | Command | At | Exit |
| --- | --- | --- | --- |
| `target/v9b-7c3d69d3f/qa-parity-slice.log` | `./scripts/qa-parity.sh slice` | `7c3d69d3f` | 0, in 13 min 53 s |
| `target/v9b-033f8fb48/qa-parity-slice.log` | `./scripts/qa-parity.sh slice` | `033f8fb48` | 0, in 13 min 33 s |
| `target/v10-d4f086f25/qa-parity-slice.log` | `./scripts/qa-parity.sh slice` | `d4f086f25` | 0, in 13 min 43 s |
| `target/v10-d4f086f25/qa-parity-pending-survey.log` | `./scripts/qa-parity.sh slice --pending` | `d4f086f25` | 0, in 55 s: 37 of the 39 pending runs fail on Rune and 2 are not started |

These are the last three runs of the exit command. The application, the Rune interface, the
runner and the ledger's runs are the same at all three commits: between `7c3d69d3f` and
`d4f086f25` only `scripts/qa-parity.sh`, its Python helper and that helper's test, the
allowances' wording in the ledger, one doc comment and documents changed. The first two ran
before this package.

### Totals

| Gate | Runs | `required` | `pending` | `platform` | `rune` | `scope` |
| --- | --- | --- | --- | --- | --- | --- |
| G1 | 1 | 1 | 0 | 0 | 0 | 0 |
| G2 | 41 | 22 | 19 | 1 | 0 | 18 |
| G3 | 33 | 13 | 20 | 2 | 1 | 17 |
| All | 75 | 36 | 39 | 3 | 1 | 35 |

Codes with no run: `harness`, `component`, `toolkit`, `deviation`.

### Required runs

Results are those of the 3 runs of the exit command, oldest first; one word means every run
agrees.

| Gate | Run | GTK | Rune |
| --- | --- | --- | --- |
| G1 | `arrow-keys-move-focus/columns-arrows` | pass | pass |
| G2 | `alt-up-and-history/columns` | pass | pass |
| G2 | `arrow-keys-move-focus-remaining/columns-hjkl` | pass | pass |
| G2 | `columns-home-end-mirror/columns` | pass | pass |
| G2 | `select-all-selects-every-entry/columns` | pass | pass |
| G2 | `shift-arrow-extends-selection/columns` | pass | pass |
| G2 | `control-click-sets-shift-anchor/columns` | pass | pass |
| G2 | `home-end-page-move-shift-anchor/columns` | pass | pass |
| G2 | `leftover-click-honors-modifiers/columns` | pass | pass |
| G2 | `right-click-selects-entry/columns-file` | pass | pass |
| G2 | `right-click-selects-entry/columns-folder` | pass | pass |
| G2 | `shift-click-range/columns` | pass | pass |
| G2 | `enter-after-escape-opens-folder/columns` | pass | pass |
| G2 | `escape-only-clears-active-column/columns` | pass | pass |
| G2 | `shift-after-escape-starts-on-focused/columns-no-range` | pass | pass |
| G2 | `shift-after-escape-starts-on-focused/columns-had-range` | pass | pass |
| G2 | `background-click-clears-selection/columns` | pass | pass |
| G2 | `background-press-clears-before-drag/columns` | pass | pass |
| G2 | `beside-last-column-clears/columns` | pass | pass |
| G2 | `hidden-files-persist-across-restart/columns` | pass | pass |
| G2 | `startup-with-closed-pipes/columns-stdout` | pass | pass |
| G2 | `startup-with-closed-pipes/columns-stderr` | pass | pass |
| G2 | `startup-with-closed-pipes/columns-both` | pass | pass |
| G3 | `escape-preserves-name-remaining/columns-existing-folder` | pass | pass |
| G3 | `escape-preserves-name/columns` | pass | pass |
| G3 | `invalid-names-retain-original/columns-enter` | pass | pass |
| G3 | `invalid-names-retain-original/columns-click` | pass | pass |
| G3 | `long-rename-keeps-caret/columns` | pass | pass |
| G3 | `rename-commits-on-click-away/columns` | pass | pass |
| G3 | `rename-commits-on-enter/columns` | pass | pass |
| G3 | `rename-moves-into-path/columns-file-as-folder` | pass | pass |
| G3 | `tab-renames-past-open-folder/columns-mirrored` | pass | pass |
| G3 | `tab-renames-past-open-folder/columns-unmirrored` | pass | pass |
| G3 | `window-accelerators-leave-rename/columns-f5` | pass | pass |
| G3 | `window-accelerators-leave-rename/columns-ctrl-backslash` | pass | pass |
| G3 | `window-accelerators-leave-rename/columns-ctrl-k` | pass | pass |

- GTK: 36 of 36, 36 of 36, 36 of 36 passed.
- Rune: 36 of 36, 36 of 36, 36 of 36 passed.

### Pending runs

Runs that share a gate and a blocker share a row. After each run is the scenario line it
stops at on Rune in the survey; "not started" is a run the runner starts on neither target.
GTK is the runs' result in the whole-suite part of the same runs of the exit command. The
blocker is the ledger's first sentence, less its line number; the ledger has the rest.

**`platform`: 3**

| Gate | Runs, and where each stops on Rune | GTK | Blocker |
| --- | --- | --- | --- |
| G2 | `keyboard-only-copy-paste/columns` (line 13) | pass | the clipboard, at Ctrl+C and Ctrl+V: the offscreen host has none (the session host; P2 and X3 hold the pieces) |
| G3 | `input-method-composes-over-selection/columns-selected-stem` (not started); `input-method-composes-over-selection/columns-caret-at-end` (not started) | not started | An input method: fcitx5-chewing on a session bus, which the pinned image lacks |

**`rune`: 1**

| Gate | Runs, and where each stops on Rune | GTK | Blocker |
| --- | --- | --- | --- |
| G3 | `tone-mark-escape-before-rename/columns` (line 18) | pass | Rune's window host delivers the Escape that breaks off a dead-key sequence as a key press, so it ends the rename; GTK's input method takes it and ends only the tone mark (docs/rune/upstream/rune/98-escape-after-a-dead-key.md, drafted) |

**`scope`: 35**

| Gate | Runs, and where each stops on Rune | GTK | Blocker |
| --- | --- | --- | --- |
| G2 | `arrow-keys-move-focus/columns-hjkl-tenxer-ru` (line 7) | pass | C1c: 10xer mode (`tenxer_mode`), from the first `j` |
| G2 | `back-then-file-keys-target-focused-folder/columns` (line 10); `sidebar-navigation-range-anchor/columns` (line 5) | pass | C8: the sidebar, at `click` on `sidebar` Home |
| G2 | `columns-return-selects-source-folder/columns` (line 15) | pass | C6: the location bar's crumbs, at `click` on the `documents` breadcrumb button |
| G2 | `focus-usable-after-view-change/columns` (line 5) | pass | C4: the List view, at Ctrl+3 (`view_mode`) |
| G2 | `tenxer-keeps-location-edit/columns` (line 9) | pass | C6: the location field, at Ctrl+L (`field`) |
| G2 | `control-click-toggles/columns` (line 2); `parent-pane-anchors-first-entry/columns` (line 2); `shift-click-revisits-after-folder/columns-name` (line 2); `shift-click-revisits-after-folder/columns-row-space` (line 2); `empty-column-clears-other-columns/columns` (line 2) | pass | the Appearance popover, at `open_menu` on its header button: no work package composes it (X1's button is inert), and its controls are the view switch (C3, C4), the preview toggle (C9), C20a's density sliders and the text size |
| G2 | `escape-dismisses-transient/columns` (line 10) | pass | C15: the properties dialog, at Alt+Return (`dialog`) |
| G2 | `tenxer-escape-closes-peek/columns` (line 6) | pass | C1c: 10xer mode (`tenxer_mode`), at Space, which there selects and moves on |
| G2 | `tenxer-escape-filter-preview-fill/columns` (line 4) | pass | C1c and C18: 10xer mode's `f` and the filter field (`field`) |
| G2 | `disabled-transport-click/columns-audio` (line 4); `disabled-transport-click/columns-video` (line 4) | pass | C9 and C13: the preview drawer and its media transport (`preview`) |
| G2 | `preview-blank-click-keeps-selection/columns` (line 5) | pass | C9 and C14: the preview drawer and the image viewer (`preview`) |
| G2 | `multiple-arguments-non-utf8/columns` (line 10) | pass | P8 and the shell: `launch` with five arguments needs a window for each, a revealed file argument and the session bus; strata-rune takes one directory |
| G3 | `escape-preserves-name-remaining/columns-new-file` (line 16); `new-item-first-free-number/columns` (line 15); `new-item-named-with-path/columns-nested-file` (line 10); `new-item-named-with-path/columns-file-to-folder` (line 10); `new-item-named-with-path/columns-own-name` (line 10); `new-item-named-with-path/columns-refused` (line 10) | pass | strata-app creates no item: the context menu's New File row (the slice leaves New Folder and New File out, with `Browser::name_new_item`; the menu rows are shown and inert by decision 19) |
| G3 | `escape-preserves-name-remaining/columns-new-folder` (line 22); `new-item-named-with-path/columns-nested-folder` (line 10) | pass | strata-app creates no item: Ctrl+Shift+N (the slice leaves New Folder and New File out, with `Browser::name_new_item`; the menu rows are shown and inert by decision 19) |
| G3 | `new-item-clears-hiding-filter/columns-file` (line 4); `new-item-clears-hiding-filter/columns-folder` (line 4) | pass | C18: the filter bar, at Ctrl+F |
| G3 | `rename-and-undo-restores/columns-file` (line 28); `rename-and-undo-restores/columns-folder` (line 29); `rename-moves-into-path/columns-file` (line 26); `rename-moves-into-path/columns-open-folder` (line 26); `rename-moves-into-path/columns-existing-folder` (line 26); `rename-moves-into-path/columns-parent` (line 26); `rename-moves-into-path/columns-back-into-folder` (line 19) | pass | strata-app wires no undo (the slice leaves it out; `Browser`'s undo engine has no work package of its own): Ctrl+Z |

The survey: 37 FAIL, 2 not started.

What the tables leave out:

- **The weak two.** `window-accelerators-leave-rename/columns-ctrl-backslash` and
  `columns-ctrl-k` are in the required table as passes. They pass on Rune only because the
  key is unbound, as [the first screen](#what-the-slice-does-not-prove) says.
- **A `rune` blocker behind a `scope` one.** `arrow-keys-move-focus/columns-hjkl-tenxer-ru`
  needs 10xer mode first, and then a key press that carries its physical key
  ([request 97](../upstream/rune/97-key-press-with-physical-key.md)).
- **Gate G2 as the plan counts it** is 42 runs: the ledger's 41 and G1's one. 23 of the 42
  are required.
- **No pending run passes on Rune.** A survey prints a line for a pending run that passes;
  this one printed none.

## 2. Harness reach

Counted by `harness-reach.py` in `target/v10-d4f086f25/` (output: `harness-reach.txt`),
against the protocol's own lists in `crates/strata-qa`: the 37 steps of `COMMANDS`
(`src/command.rs`), the 27 target kinds of `TARGET_KINDS` (`src/target.rs`) and the 70
variants of `Check` (`src/predicate.rs`).

**How an item counts.** The engine is shared, so most of the protocol is not the backend's
to implement: the engine runs a step or decides a predicate from what `Backend` answers. An
item counts as implemented on Rune when the engine can run or answer it from methods that
`crates/strata-app/src/qa/backend.rs` answers. It counts as lacking when the reply is "…
is not supported on this target", or when it rests on a method the Rune backend answers with
nothing: `scroll`, `scroll_position`, and the scroll pane above a pane's entry list, which on
Rune is the strip's sideways viewport and not the list's own
([V7](V7-rune-qa-backend.md#what-v7-found)).

| Kind | Implemented | Lacking |
| --- | --- | --- |
| Steps | 29 of 37 (78%) | The eight behind `Backend::command`: `reveal`, `reveal_control`, `set_scrollbar`, `capture`, `measure_text`, `drop_samples`, `clipboard_set`, `clipboard` |
| Targets | 26 of 27 (96%) | `viewport` |
| Predicates | 59 of 70 (84%) | The seven behind `Backend::check`: `pixel`, `accent_pixels`, `repainted`, `contrast_below`, `footer_fill`, `elapsed`, `diagnostics_mention`. And four that read the scroll pane: `scroll_value`, `visible`, `visible_top`, `band_selected` |
| All three | 114 of 134 (85%) | |

Implemented in a kind and lacking in one form, which the table above does not show:

- an entry's `part`: `label` is implemented; `icon` and `metadata` are not, because Rune's
  row is one accessible node and the backend presents only its name;
- the `note` step's `visible`, `visible_top`, `middle_visible` and `scroll_target`, which
  read the same scroll pane;
- a role target's `horizontal` and `vertical` states: `orientation` is unanswered;
- the `state` predicate's `pressed`, `busy` and `marked`, which are always false;
- `toplevel`, `other_window`, `adopt`, `some_window` and `windows`: the backend presents one
  window.

**Scenario files.** Of the 351 files under `tests/qa/scenarios/`, 325 (93%) use no step,
target or predicate that is lacking, counting the forms in the list above but the last two.
The 26 that are out name `clipboard` (6 files), an entry's `icon` (4), `pixel` (4), `visible`
(4), `viewport` (4), `reveal` (2) and nine other items once each; a file can name several.

That figure is an upper bound on what the backend could run, not a count of what would pass:

- **13 of the 325 have a runner step that reads or acts on the X screen** (`baseline`,
  `screenshot`, `foreign`, `foreign_label`, `foreign_stop`), where the offscreen host draws
  nothing. Without them it is 312 of 351 (89%).
- **It counts the interface as able to do anything.** 304 of the 351 files have a Columns
  run at all; the rest are Icons or List only. Sidebars, dialogs, the preview and search are
  not mounted.
- **It cannot see what a step needs at run time.** The engine never wheels a clipped target
  into view on Rune (`scroll` is unanswered), a row a virtual list has not mounted is not in
  the tree, 16 of the 325 name a second window, 45 drag, 8 start the portal host and 16 need
  a session bus.
- **Use is uneven.** Of the 20 lacking items, the corpus uses ten: five steps, four
  predicates and the one target.

## 3. Cost

Lines are from `cost.py` in `target/v10-d4f086f25/` (output: `cost.txt`), which sums `git
diff --numstat -M <base> <tip>` over each package's stretch of the branch: the numbers `git
diff --stat` draws between the commit before a package's first and its last. Each file is
put in one part by its path. A package's last commit is its record, so its documents are in
its row.

| Package | Commits | First and last commit dated | Strata | Rune interface | Harness | Build and CI | Documents |
| --- | --- | --- | --- | --- | --- | --- | --- |
| V1 | 3 | 2026-11-10 18:08 | +1 −1 |  |  | +19 −11 | +3 −3 |
| V4 | 19 | 2026-11-10 18:15 to 20:09 | +4,120 −3,558 |  | +9 −9 | +324 −114 | +150 −49 |
| V2 | 1 | 2026-11-10 20:06 |  |  | +286 −270 | +16 −2 | +2 −2 |
| V0 | 1 | 2026-11-10 21:08 |  |  |  |  | +225 −0 |
| The plan's files | 1 | 2026-11-10 21:25 |  |  |  |  | +1,773 −0 |
| V3 | 9 | 2026-11-10 21:41 to 23:00 | +1 −10 |  | +6,861 −4,870 | +4 −0 | +219 −207 |
| V4b | 9 | 2026-11-10 23:51 to 2026-11-11 00:12 |  | +1,211,744 −0 |  | +2,975 −97 | +64,069 −183 |
| V5 | 9 | 2026-11-11 00:17 to 01:28 | +1,003 −859 | +1,910 −1 |  | +50 −12 | +477 −169 |
| of which V5b | 2 | 2026-11-11 01:03 to 01:04 | +962 −850 | +96 −23 |  | +4 −2 |  |
| V6 | 4 | 2026-11-11 01:50 to 02:54 |  | +5,076 −127 |  | +1 −0 | +365 −112 |
| V7 | 5 | 2026-11-11 03:34 to 04:11 | +12 −4 | +74 −21 | +3,702 −30 | +5 −1 | +502 −177 |
| V8 | 21 | 2026-11-11 09:57 to 11:35 | +3 −12 | +2,000 −131 | +837 −135 | +1 −0 | +522 −26 |
| V9 | 10 | 2026-11-11 11:50 to 12:36 | +110 −39 | +1,456 −72 | +665 −115 | +1 −0 | +513 −31 |
| V9b | 8 | 2026-11-11 12:56 to 17:37 |  |  | +633 −20 | +722 −17 | +943 −51 |

- **Strata** is `src/` and the shared crates. **Rune interface** is the six Rune-side crates
  and their goldens, less the QA backend. **Harness** is `crates/strata-qa`, `src/qa`,
  `crates/strata-app/src/qa`, `tests/qa` and the QA scripts. **Build and CI** is the
  manifests, the lockfile, the other scripts, the test image and the workflows.
- **V4b is a copy.** 1,276,535 of its lines are one commit, `ad88c18a8`, which copies
  strata-components' four crates, their goldens and their documents from another repository.
  Nothing in it was written in the slice. The rest of V4b is 2,494 added and 453 removed
  lines in 774 files.
- **V4 and V2 are moves.** Git reads most of V4's 755 files as renames, so the 4,120 added
  lines are what changed in them, not what moved.
- **V0's spike is not on the branch.** Its row is the findings document.
- **V4's stretch has V2's commit inside it** and is counted as two stretches around it.
  Seven commits in it are not named in [V4's record](V4-shared-crates.md) and are counted
  with it.
- **The whole slice** is 100 commits, from the first of V1's three, which end at
  `2a6054141`, to `d4f086f25`.

By group. The plan named three; the gate packages are a fourth, because V8 and V9 changed
the interface, the components and the harness together:

| Group | Packages | Code | Documents |
| --- | --- | --- | --- |
| The refactor of Strata | V1, V2, V4 | +4,775 −3,965 | +155 −54 |
| The Rune interface | V4b, V5, V6 | +1,222,759 −1,096 | +64,911 −464 |
| The harness | V3, V7 | +10,659 −4,936 | +721 −384 |
| The gates and the exit check | V8, V9, V9b | +6,428 −541 | +1,978 −108 |

V3 is in the harness group, as the plan has it, though it is also a refactor of Strata's own
QA code: counted with the refactor instead, that group is +11,641 −8,845 lines of code and
the harness is V7's +3,793 −56.

**Elapsed time is missing.** No package record states how long the package took. The records
give a date for each package's runs, 2026-11-10 or 2026-11-11, and the durations of single
commands. The table's dates are the commits' own, in UTC−6. They are not working time:
commits were often made in batches, six of V4b's and four of V7's in one minute each.

What the checks cost, where a record states it:

| Command | Time | Source |
| --- | --- | --- |
| `./scripts/qa-parity.sh slice` | 13 min 33 s to 13 min 57 s, five runs | [V9b](V9b-test-image-and-exit-check.md#done-when-and-the-evidence), and this report's |
| Its GTK mutation part | 10.9 to 11.3 minutes | The same logs |
| `./scripts/qa.sh`, the whole GTK suite | 66 to 80 s | [V3](V3-qa-backend-trait.md), [V4b](V4b-import-components.md), [V7](V7-rune-qa-backend.md), [V8](V8-read-path-parity.md) |
| `./scripts/qa-parity.sh g1`, once built | About 5 s | [V7](V7-rune-qa-backend.md#what-it-left-for-v8) |
| The test image's build, from nothing on four cores | 4 min 46 s | [V9b](V9b-test-image-and-exit-check.md#what-the-build-costs) |

## 4. Gaps

### strata-components

| Gap | Found by | Work package | State |
| --- | --- | --- | --- |
| A host could not describe a Columns row, and a column's group had no name | V0's name probe | C2c, `90fdcb2f7` (decision 5) | Closed in V6: `ColumnSources::description`, and the pane group named by its folder |
| Six keys where the Columns view differed from the GTK view: Return on a folder whose column is open, an arrow at the list's end, a key that leaves the cursor in place, Ctrl+A's cursor, a key that only changes the active column, Right when the open column is another folder's | V6's probe | C2d, `ad73edd15` and `c072c317f` (decision 17) | Closed in V8. Two details of the GTK view are left out and named in C2's module docs: Up reaching a pane header's actions, and the cursor after Ctrl+A when only the last entry was selected |
| Left in a column with no cursor landed on its last entry | V9, by `tab-renames-past-open-folder/columns-unmirrored` | C2e, `236f6c9f8` (decision 17) | Closed in V9 |
| The rename editor returned the keyboard to its row's collection, which in Columns takes none | V9's first engine test | C5a, `29410171c`: `RenameEditor::focus_return` | Closed in V9 |
| Shift+Left in a column: the GTK view goes to the parent column, the component extends the selection in place | V9, read in the code and not run | C2; no package of its own yet | Open. Not changed |
| The Menu key and Shift+F10 open no menu: C2's columns install their own keyboard controller, and C21's `MenuKeys` has to wrap it | V8 | C2 and C21 (decision 22) | Open, after the slice |
| An empty folder shows nothing, where the GTK view shows its empty page | V6 | None named in the records | Open |
| No gallery scene of the rename editor in Columns | V9 | C5 | Open |

Not gaps, and recorded so nobody files them as such: Rune's row label is as wide as its text
and has no leftover space ([V8](V8-read-path-parity.md#the-rows-name-label-decision-20)); a
remembered row is gone after a rename, because a row's key is its entry's id
([V9](V9-rename-parity.md#ids-the-cursor-and-the-selection-after-a-rename)).

### Rune

| Request | What | Found by | State |
| --- | --- | --- | --- |
| [97](../upstream/rune/97-key-press-with-physical-key.md) | A host that owns a `Runtime` cannot send a key press with its physical key, so a binding on a plain letter does not match under a non-Latin layout through the QA backend | V7 and V8 | Filed on 2026-11-11 as [anomalyco/rune#657](https://github.com/anomalyco/rune/issues/657), with a headless repro that was run |
| [98](../upstream/rune/98-escape-after-a-dead-key.md) | The Escape that breaks off a dead-key sequence reaches a text field as a key press, and ends the rename | V9 | Drafted from the sources and not run in a window. It has no headless repro and cannot have one. Decision 26 is open: check it in a window, then file it or drop it |
| Rune [#139](https://github.com/anomalyco/rune/issues/139), an issue that was open before the slice | Under a covered Wayland window a `Waker` call never runs `poll`, and a `wake_after` timer runs four times a second | V0 named it; V5 measured it at `2a44156` | Worked around: `strata-platform`'s `glib_loop` reaches Rune's loop through `AppHandle::update_if`, which stays on time under a cover and which Rune does not promise. The compositor tests `wake::` and `window::` are the guard |
| No draft | rune-kit's `Menu` rows are `menuitem` or `menuitemcheckbox` only: it has no radio row, no toggle button and no slider | V8, at the Appearance popover | Not drafted. The popover needs a radio row |
| No draft | `Runtime::frame` reports "animating", not "has work"; the backend settles with `Runtime::settle_all` | V0 | Worked around in the backend |
| No draft | With a menu open, Rune's accessibility tree holds the menu alone | V8 | Worked around: the backend presents the panes from the drawn elements, without the rows' descriptions |
| No draft | The tree places a caret by shaped cluster and gives no line's start, so a field is read on its first line only | V9 | Worked around by converting clusters to characters; a later line answers nothing |

## 5. Measurements

Both targets were measured the same way, in the pinned image, at `d4f086f25`. No file of the
checkout was changed for it.

**What the figures are, in one table.** Release builds, 20 samples each, median with the
least and the greatest:

| | GTK, a window on Xvfb | Rune, the offscreen host |
| --- | --- | --- |
| Launch to first listing: the start of the process to the listing's last event | 191 ms (181 to 207) | 16.0 ms (15.0 to 16.4) |
| The same, to the `ready` reply | 261 ms (253 to 285) | 16.6 ms (15.6 to 17.0) |
| A 10,000-entry listing, from the navigation to the last event, in a running process | 65.7 ms (64.0 to 69.6) | 10.6 ms (9.7 to 11.6) |
| The same at launch, on that folder | 192 ms (76 to 221) | 12.5 ms (11.0 to 13.2) |

**These do not compare two finished interfaces.** Read the table with what differs:

- **The GTK application paints, and the Rune host does not.** The GTK target opens a window
  on Xvfb and draws it with the software renderer the QA runs use, on the thread that
  delivers the listing's events. The Rune target is the offscreen host: it lays out frames
  and draws on no surface. A Rune window was not measured: the image has no compositor and
  no Vulkan driver.
- **The GTK application is whole, and the Rune interface is the slice.** GTK starts its
  sidebar, its tabs, its location bar and its theme; `strata-rune` starts a header and one
  view.
- **GTK does more after the last listing event.** Its views ask for the metadata of their
  visible rows, which arrives 41 to 117 ms later in every 10,000-entry sample. The Rune
  interface asks for none yet, so nothing follows there. Neither is in the figures.
- **The GTK launch figure for 10,000 entries is not one number.** Its samples fall in
  groups: five at 76 to 82 ms, one at 113, ten at 188 to 198 and four at 213 to 221. What
  separates them was not looked into; the window's first frames are painted in the same
  stretch.

What can be said: the path the slice built, from `Browser`'s events through the adapter into
the Columns component, costs about 10 ms for 10,000 entries on a host that does not paint.
V0's probe measured 8.9 ms for the same listing at launch; with the adapter the slice built
it is 12.5 ms.

### The method

- **Where.** `measure.sh` in `target/v10-d4f086f25/` starts a `./scripts/qa-drive.sh`
  session, which gives the pinned container, and runs everything else through
  `./scripts/qa-drive.sh exec`. The base is the one the exit command ran in. The machine is
  an Intel Core Ultra 9 285K with 24 cores; the fixtures are on the container's tmpfs.
- **How a sample starts.** `measure.py` starts the target's `--qa` binary on a directory as
  the QA runner starts it: the harness manifest's environment, a fresh HOME seeded by
  `scripts/qa_drive.py`, a fresh fixture, no session bus, and for the GTK target a private
  Xvfb of its own with the runner's arguments. One process is one sample.
- **What is read.** Two things, the same on both targets:
  - the first reply on stdout, `ready`. The GTK backend sends it once its window shows an
    entry list and has painted; the Rune backend once the launch's folder is listed in the
    host;
  - three lines of the application's own log, which both binaries write from the same shared
    code, with `RUST_LOG=info,strata_fs::app::browser=debug`: "directory load started" (the
    listing adapter, when a load is asked for), "directory load finished" (its worker) and
    "directory publication finished" (`Browser`, on the interface thread, right after it has
    emitted `LoadFinished` or `SortingFinished`). The last is "the last event".
- **The start of the process** is the wall clock read right before the fork. The log's
  timestamps are the same clock.
- **Three kinds of sample.** A launch on the canonical fixture, five entries shown. A launch
  on a folder of 10,000 empty files. And a launch on the fixture with that folder in it as
  `big`, then `{"key":"Down"}`, which moves the cursor onto `big` so that its column opens.
- **`Browser` asks for a listing twice in a row,** at launch and for a child column, on both
  targets. The timed stretch starts at the first request and holds both.
- **Run count.** 20 samples for each target, kind and profile, after two that are thrown
  away. The targets alternate.
- **Build profile.** The release binaries were built for this, with the `qa` feature alone
  and the workspace's release profile (LTO, one codegen unit): `cargo build --release
  --locked --features qa --bin strata` in 5 min 11 s and `cargo build --release --locked -p
  strata-rune --features qa` in 3 min 28 s, each a cargo run of its own, into
  `target/v10-d4f086f25/release-build/`. The debug binaries are the QA builds the scripts
  make.

Kept in `target/v10-d4f086f25/`: `measure-samples.jsonl` (every sample),
`measure-summary.md` (the tables below, by `measure-summary.py`), `measure-run.log`,
`measure-build-first-run.log` (the release builds) and `sample-logs/` (the application's log
of the last sample of each kind). A first run of the same 264 samples is kept beside them as
`measure-samples-first-run.jsonl`. It timed the key's listing from `Browser`'s second request
instead of its first; its other figures agree with these, and its GTK launch samples fall in
groups in the same places.

### The spread

Milliseconds: median (least to greatest; 10th, 90th percentile; samples).

#### Release builds

**Launch on the canonical fixture**

| Measure | GTK | Rune |
| --- | --- | --- |
| Start of the process to the last listing event | 191 (181 to 207; 183, 202; n=20) | 16.0 (15.0 to 16.4; 15.4, 16.1; n=20) |
| Start of the process to the `ready` reply | 261 (253 to 285; 255, 280; n=20) | 16.6 (15.6 to 17.0; 15.9, 16.7; n=20) |
| Start of the process to the load being asked for | 171 (165 to 183; 165, 176; n=20) | 11.5 (11.3 to 11.8; 11.4, 11.6; n=20) |
| Load asked for to the last listing event | 17.9 (16.5 to 30.7; 16.9, 29.4; n=20) | 4.4 (3.5 to 4.8; 3.8, 4.6; n=20) |
| GTK's own line: application window presented | 123 (117 to 129; 119, 126; n=20) | none |
| GTK's own line: first themed frame painted | 154 (125 to 164; 128, 159; n=20) | none |

**Launch on a folder of 10,000 files**

| Measure | GTK | Rune |
| --- | --- | --- |
| Load asked for to the last listing event | 192 (76 to 221; 77, 217; n=20) | 12.5 (11.0 to 13.2; 11.6, 13.0; n=20) |
| The worker's own time for the read that is published | 31.0 (11.0 to 35.0; 11.0, 33.0; n=20) | 4.0 (3.0 to 4.0; 3.0, 4.0; n=20) |
| Start of the process to the last listing event | 360 (253 to 390; 256, 384; n=20) | 24.6 (23.2 to 26.4; 23.6, 25.4; n=20) |
| Start of the process to the `ready` reply | 427 (360 to 493; 392, 464; n=20) | 33.3 (30.1 to 35.1; 32.0, 34.3; n=20) |

**A key that opens a folder of 10,000 files, in a running process**

| Measure | GTK | Rune |
| --- | --- | --- |
| Load asked for to the last listing event | 65.7 (64.0 to 69.6; 65.0, 66.9; n=20) | 10.6 (9.7 to 11.6; 9.8, 11.0; n=20) |
| The worker's own time for the read that is published | 4.5 (4.0 to 9.0; 4.0, 8.0; n=20) | 4.0 (3.0 to 4.0; 3.9, 4.0; n=20) |
| Key sent to the last listing event | 153 (151 to 158; 152, 156; n=20) | 86.2 (85.3 to 87.2; 85.5, 86.6; n=20) |

- **GTK.** Every process exited 0. Requests for the listing in the timed stretch: launch 2,
  launch on 10,000 2, key 2. "directory publication finished" lines for the column: launch
  2, launch on 10,000 1 or 2, key 1. In 40 of the 40 samples of 10,000 entries, metadata
  fills follow the last listing event, the last of them 41 to 117 ms after it.
- **Rune.** Every process exited 0. Requests for the listing in the timed stretch: launch 2,
  launch on 10,000 2, key 2. "directory publication finished" lines for the column: launch
  2, launch on 10,000 1, key 1. In none of the 40 samples of 10,000 entries does a metadata
  fill follow the last listing event.

#### Debug builds

**Launch on the canonical fixture**

| Measure | GTK | Rune |
| --- | --- | --- |
| Start of the process to the last listing event | 272 (265 to 311; 266, 283; n=20) | 55.4 (50.5 to 57.8; 53.9, 55.9; n=20) |
| Start of the process to the `ready` reply | 364 (353 to 384; 356, 378; n=20) | 60.1 (55.3 to 62.7; 58.6, 60.8; n=20) |
| Start of the process to the load being asked for | 238 (232 to 250; 233, 246; n=20) | 21.8 (21.4 to 22.3; 21.5, 21.9; n=20) |
| Load asked for to the last listing event | 32.6 (25.6 to 64.5; 31.3, 37.6; n=20) | 33.7 (28.8 to 35.9; 32.4, 34.2; n=20) |
| GTK's own line: application window presented | 164 (159 to 173; 161, 171; n=20) | none |
| GTK's own line: first themed frame painted | 223 (174 to 235; 219, 233; n=20) | none |

**Launch on a folder of 10,000 files**

| Measure | GTK | Rune |
| --- | --- | --- |
| Load asked for to the last listing event | 373 (367 to 378; 369, 377; n=20) | 46.8 (38.4 to 48.9; 42.3, 47.7; n=20) |
| The worker's own time for the read that is published | 33.0 (31.0 to 37.0; 31.9, 34.1; n=20) | 8.0 (7.0 to 11.0; 7.0, 9.1; n=20) |
| Start of the process to the last listing event | 613 (602 to 625; 606, 620; n=20) | 68.9 (60.8 to 71.2; 64.2, 70.2; n=20) |
| Start of the process to the `ready` reply | 772 (747 to 798; 753, 786; n=20) | 135 (127 to 138; 129, 138; n=20) |

**A key that opens a folder of 10,000 files, in a running process**

| Measure | GTK | Rune |
| --- | --- | --- |
| Load asked for to the last listing event | 145 (138 to 150; 141, 148; n=20) | 31.9 (30.4 to 34.0; 30.7, 33.0; n=20) |
| The worker's own time for the read that is published | 18.0 (12.0 to 19.0; 12.9, 18.0; n=20) | 8.5 (8.0 to 10.0; 8.0, 10.0; n=20) |
| Key sent to the last listing event | 238 (232 to 243; 234, 242; n=20) | 109 (108 to 112; 108, 111; n=20) |

- **GTK.** Every process exited 0. Requests for the listing in the timed stretch: launch 2,
  launch on 10,000 2, key 2. "directory publication finished" lines for the column: launch
  2, launch on 10,000 1, key 1. In 40 of the 40 samples of 10,000 entries, metadata fills
  follow the last listing event, the last of them 44 to 149 ms after it.
- **Rune.** Every process exited 0. Requests for the listing in the timed stretch: launch 2,
  launch on 10,000 2, key 2. "directory publication finished" lines for the column: launch
  2, launch on 10,000 1, key 1. In none of the 40 samples of 10,000 entries does a metadata
  fill follow the last listing event.

**The debug figures are not like for like.** The GTK QA build carries `invariant-checks`,
`Browser`'s exact and slow consistency checks; the Rune QA build does not. They are given
because they are the builds every parity run uses.

"Key sent to the last listing event" starts when the step is written to the application. It
holds the 75 ms both interfaces wait before a child column follows the cursor.

### What was not measured

- **A Rune window:** its launch, its first frame and a listing with the frames painted. That
  needs a compositor and a GPU or lavapipe, which the pinned image does not have.
- **The listing shown.** No line of the log that both interfaces write says when the rows
  are on the screen, and adding one is a code change. The `ready` reply is the nearest
  thing, and it means a painted window on one target and a listed folder on the other.
- **Memory, and a folder larger than 10,000 entries.** V6 measured 100,000 entries in
  process on the host ([V6](V6-columns-adapter.md#listings)).

## 6. A projection for the next slice

The next slice is Icons and List on the same harness. This section states what the
projection rests on and stops there: no record of this slice states how long a package took,
so nothing here is a duration.

The figures are from `next-slice-basis.py` in `target/v10-d4f086f25/` (output:
`next-slice-basis.txt`), read from the scenario headers, the ledger and
`crates/strata-app/src`.

**What carries over unchanged.**

| From | What | Size in `strata-app` |
| --- | --- | --- |
| V1 to V4 | The workspace, the shared crates, and `strata-qa`'s engine behind `Backend` | |
| V5, V5b | The shell, the two hosts, the loop and `glib_loop`, start-up, reading and saving `settings.toml` | 1,448 lines of code, 210 of tests |
| V7, V9 | The QA backend: the tree, the role and key maps, layouts and dead keys, `field`, the input log. Nine of its lines name the Columns view | 1,983 lines of code, 1,206 of tests |
| V8 | The context menus' rows, as data | 436 lines of code, 292 of tests |
| V9 | The rename policy and Strata's shared name rules | 338 lines of code, 42 of tests |
| V7, V9b | The target switch in the runner, `qa-parity.sh`, the allowances, the mutation check, the image and its seeding | |

**What repeats for each view.**

| Work | What Columns took | What is known for Icons and List |
| --- | --- | --- |
| An adapter between `Browser` and the view's component | `columns/`: 1,572 lines of code and 1,932 of tests (V6, extended in V8 and V9) | C3 (Icons) and C4 (List) are `merged` in [`STATUS.md`](../work/STATUS.md). Mirroring is the Columns view's own. Whether the listing, the hidden-row map and the id table can be shared as they are was not tried |
| Who owns the cursor and the selection | Decision 11, by a probe | In Icons and List the GTK application already has the view interpret keys and write the result to `Browser` ([V6](V6-columns-adapter.md#what-the-gtk-application-does)), which is the design the adapter has |
| The component's keys against the GTK view's | Seven differences, closed in C2d and C2e | Not compared for C3 or C4 |
| Accessible names, roles and descriptions against GTK's tree | Decision 5 and C2c | Not probed. The List view's table roles are mapped by name and were not seen live |
| The backend's marks for the view's pane, list and rows | Read from the Columns adapter's keys | To add for each view |
| Where the editor and the menus sit in the view | `columns/editing.rs`, and the pointer's intents | To add for each view |
| A sort of the view's runs, with fixes, and three clean runs | V8 and V9: 75 runs, 36 required | 55 runs in the same folders: 22 Icons and 24 List in G2's, 4 Icons and 5 List in G3's. 34 of them are in a scenario file whose Columns run is required today |
| A Rune mutation patch for each gate area | Three | To add for each view |

**What is new and belongs to neither list.**

- **The view has to be chosen.** `strata-app` mounts one view and reads `browser_mode` as
  text. A run in another mode needs the mode read at launch; switching it live is the
  Appearance popover (decision 21) and the view keys.
- **The ledger lists Columns runs only,** and the runner's test holds it to the Columns runs
  of its folders. Both have to learn the other modes.
- **The rulings already made for the next slice:** undo (decision 23, seven runs), naming a
  new item (24, ten runs), GTK's dialog for a failed rename (25, with C15) and the
  Appearance popover (21, five runs).

**What the projection cannot rest on.** The parity runs say nothing about a window
([the first screen](#what-the-slice-does-not-prove)), so a view that passes its runs is as
unproven on a desktop as Columns was before the hand check.

## Owner decisions

All 27, from the [index](README.md#owner-decisions), which has each in full.

| # | Decision | Ruling | Ruled |
| --- | --- | --- | --- |
| 1 | Where `strata-rune` lives | On `strata-next`, in the workspace | Not dated in the records; with the workspace rulings of 2026-11-10 |
| 2 | Raise the toolchain to the one Rune needs | Done: Rust 1.99.1 | Not dated; before V0 |
| 3 | A software Vulkan driver in the pinned image | None was needed for the offscreen host, and none was added | No ruling was needed. Found by V0 on 2026-11-10 |
| 4 | Which strata-components revision | `integration` at `e4f2df1a`. The old repository is frozen there | 2026-11-10 |
| 5 | GTK's accessible names and AT-SPI role names as the contract | The component changed: a host describes each row, and the view names each column's group (C2c) | 2026-11-11 |
| 6 | Non-US layout runs: required or pending | Required. The runner tells a Rune target the layout and the backend types under it | 2026-11-11 |
| 7 | Hand check of the windowed binary on Hyprland | The owner does the check. [Done](#the-hand-check) | 2026-11-11, and the check on the same day |
| 8 | The windowed wake: a tick, or a GLib embedding | The embedding, notified through `AppHandle::update_if` | 2026-11-11 |
| 9 | `gstreamer` in `strata-platform` against the `AGENTS.md` rule | The rule stands. The `media-audio` shim was not imported | 2026-11-10 |
| 10 | Import history: squash or keep | Squashed | 2026-11-10 |
| 11 | Who owns cursor and selection | The view interprets input and `Browser` holds the result, as the probe found | 2026-11-11 |
| 12 | The test image's dependency stage | Ruled twice: add the Rune side to the stage; then, on V7's finding, fix the stage and have the scripts seed their caches from it. Built in V9b, pushed and published | 2026-11-11 |
| 13 | A `rune` job in CI | Open. Deferred until after the slice | Deferred on 2026-11-11 |
| 14 | `[profile.dev.package."*"] debug = false` for the workspace | Open | Not ruled |
| 15 | A home for the `media-audio` shim | Open | Not ruled |
| 16 | Move the settings store, against the `AGENTS.md` rule on `src/ui` | Moved (V5b). Any other move under `src/ui` still needs asking | 2026-11-11 |
| 17 | Six key behaviours where the Columns component differs from the GTK view | The component changes so all six match GTK (C2d). The seventh, found by V9, is kept (C2e), and so is the F5 binding | 2026-11-11 |
| 18 | Point the usage checks of `release.yml` and `ci.yml` at `strata-rune` | Open. Deferred until after the slice | Deferred on 2026-11-11 |
| 19 | G2's stop condition was hit by scope | Go on; widen the slice to the context menus and the Appearance menu; code such a run `scope` | 2026-11-11 |
| 20 | A row's name label, which Rune's tree does not have | Backend-only first, and that was enough | 2026-11-11 |
| 21 | The Appearance popover | Left for the next slice. Its five runs stay `scope` pending | 2026-11-11 |
| 22 | The Menu key and Shift+F10 | Both wait until after the slice | 2026-11-11 |
| 23 | Undo | Left for the next slice. Seven runs stay `scope` pending | 2026-11-11 |
| 24 | Naming a new folder or file | Left for the next slice. Ten runs stay `scope` pending | 2026-11-11 |
| 25 | A rename the browser fails: the inline editor, or GTK's dialog | GTK's dialog is the target, in the next slice. The editor stands in until dialogs (C15) are mounted. A known difference, not a `deviation` | 2026-11-11 |
| 26 | Request 98, the Escape after a dead key | Open: check it in a window, then file it or drop it | Not ruled |
| 27 | The exit command failed for one GTK run that was flaky before the slice | Ruled twice: a named allowance held to a passing rerun alone; then, on V9b's figures, the named run allowed outright. With it, `cut.patch` staying stale is what "as before" means | 2026-11-11 |

Open after the slice: 13, 14, 15, 18 and 26.

## Stop conditions

| Condition | Hit |
| --- | --- |
| V0's host probe fails: an offscreen `Runtime` cannot host the application or give the tree | No |
| V0's core probe finds widget types under `Browser` or rename | No |
| V0's loop probe shows a stall | No, offscreen or in a window. V5 measured a folder change reaching the interface in 102 ms with the window shown and 102 ms with it covered |
| V3 changes a reply | No. Every verdict matches the baseline. `app` differs between two runs of the baseline itself, so the ledger compares verdicts |
| G2 is mostly blocked: more than a third of its 42 runs land as `component` or `rune` pending | Yes, as first counted on 2026-11-11: 20 of 42, each a run that needs a part the slice marks Out. V8 stopped. The owner then ruled such a run `scope` (decision 19), and under that coding the count is 0, with one `rune` blocker named behind a `scope` one. V8 went on |
| G3 has none of its own. Counted as G2's | No: 1 of 33, the `rune` run |

## Known differences from GTK

No required run checks any of these, and none is ruled a `deviation`.

- **A rename the browser fails.** The Rune interface opens the inline editor again, with the
  browser's message under the field. The GTK view shows an "Unable to rename item" dialog.
  The dialog is the target, in the next slice; the editor stands in until dialogs (C15) are
  mounted (decision 25).
- **Inert context-menu rows.** The menus show every row the GTK menus show for a local
  folder. Open, Rename, Select All, Refresh and Show or Hide Hidden Files act. These are
  shown and do nothing: New Folder, New File, New Folder with Selection, Open With…, Quick
  preview, Run, Open in Terminal, Open in… with New Tab and New Window, Extract here,
  Extract to…, Cut, Copy, Duplicate, Move to…, Copy to…, Compress…, Pin to sidebar,
  Customize…, Copy path, Copy name, Copy paths, Copy names, Properties, Move to Trash,
  Permanently delete, and Paste in a pane's menu. Open on several entries opens the one
  under the pointer, where GTK opens them all. A row has no icon
  ([V8](V8-read-path-parity.md#the-context-menus-decision-19)).
- **The Menu key and Shift+F10** open no menu (decision 22). The menus open by the pointer.
- **An empty folder** shows no empty page.
- **Undo and the naming of a new item** are unbound (decisions 23 and 24): Ctrl+Z and
  Ctrl+Shift+N do nothing.
- **Shift+Left in a column** extends the selection in place, where the GTK view goes to the
  parent column. Read in the code, not run.
- **Not ported in the rename path:** the label showing the new name before the rename lands,
  and the field's own menu ([V9](V9-rename-parity.md#the-wiring)).
- **Start-up:** `strata-rune` has no GVfs probe, so a hung `gvfsd` blocks it at start, and it
  applies the saved language but not the date format or reduced motion
  ([V5](V5-shell-and-loop.md#start-up-shared), [V5b](V5b-settings-store.md#left-for-later)).
- **The window** has the header with five inert actions, and no sidebar, tabs or location
  bar. That is the slice's scope, not a difference to rule on.

## The hand check

Decision 7. On 2026-11-11 the owner checked the windowed `strata-rune` by hand on Hyprland: a
real directory in the Columns view, keyboard navigation and selection, and inline rename.
All behaved correctly and nothing looked wrong. In the owner's words: "I have manually
verified that rename works already and everything looks good."

- This is the slice's only evidence from a real compositor.
- No other compositor was checked.
- The record does not say whether the context menus, a covered window or a folder change on
  disk were tried.

The command that builds and opens the window, from the checkout:

```bash
cargo run --locked -p strata-rune -- <directory>
```

It is one package of the Rune lane, never `--workspace`, and without the `qa` feature. The
binary takes one directory, or none for the home directory (`usage: strata-rune
[DIRECTORY]`). It was checked on the host at `d4f086f25` with `--help` in the directory's
place and no display variable set: it builds and prints that line
(`hand-check-command-help.log` in `target/v10-d4f086f25/`). This package opened no window,
and the record does not say which command the owner used.

## Where the evidence is

Under `~/dev/strata-next-verify/target/`, unless another place is named. Each package's
record names its files.

| Package | Evidence |
| --- | --- |
| V0 | The spike worktree `~/dev/strata-spikes/v0-probes`, on the local branch `spike/v0-rune-probes`. [Findings](V0-findings.md) |
| V1, V2, V4 | `qa-baseline-65eb2da03/` and `qa-baseline.log`: the GTK suite at the commit V4 ends on. Neither V1 nor V2 has a record of its own |
| V3 | `qa-baseline-65eb2da03/`, `qa-baseline-{2,3}-artifacts/`, `qa-step{2..8}-artifacts/`, `qa-final-{1,2,3}.log`, `qa-final-{1,2,3}-artifacts/`, `qa-mutation-final-summary.md`, `quality-final-{fmt,clippy,test}.log`, `v3-tools/` |
| V4b | `v4b-quality.log`, `v4b-qa.log`, `v4b-qa-artifacts/` |
| V5, V5b | `v5-startup-quality-test.log`, `v5-startup-qa.log`, `v5-final-quality.log`, `v5-final-qa.log` |
| V6 | None kept: everything ran on the host. [The record](V6-columns-adapter.md#done-when-and-the-evidence) has the commands |
| V7 | `v7-e97b84e0b/` |
| V8 | `v8-2acdb32e8/` and `v8-work/` (the surveys), `v8-24fb0b9c6/`, `v8-5118ca9b7/` |
| V9 | `v9-work/` (the surveys), `v9-61a7d6224/` |
| V9b | `v9b-37b63862f/`, `v9b-481392c68/`, `v9b-481392c68-superseded-a2ee7c3bc/`, `v9b-7c3d69d3f/`, `v9b-record/`, `v9b-033f8fb48/` |
| V10 | `v10-d4f086f25/`: the exit command's log, the survey, the measurements, and the scripts that made this report's tables |

The scripts in `v10-d4f086f25/` are one-off generators and are kept out of the tree:

| Script | Makes |
| --- | --- |
| `parity-matrix.py` | [The parity matrix](#1-the-parity-matrix): `parity-matrix.md` |
| `harness-reach.py` | [Harness reach](#2-harness-reach): `harness-reach.txt` |
| `cost.py` | [Cost](#3-cost): `cost.txt`, `cost.md` |
| `measure.sh`, `measure.py`, `measure-summary.py` | [The measurements](#5-measurements): `measure-samples.jsonl`, `measure-summary.md` |
| `next-slice-basis.py` | [The projection](#6-a-projection-for-the-next-slice)'s figures: `next-slice-basis.txt` |
| `build-report.sh`, `assemble.py` | This file, from its text in `slice-report.template.md`, `measurements.template.md` and `after-index.md` with the tables above put in |
| `check-links.py` | The check of this file's links and anchors |

## What the records do not settle

- **Elapsed time.** No record states it for any package ([Cost](#3-cost)).
- **V1 and V2 have no record.** The index names their commits; nothing names their runs.
- **V0's GTK tree** (`.v0/gtk/tree-after.txt`) is on the spike branch and not in the tree.
- **Test counts are counted two ways.** The whole GTK suite is "793 tests" in V7 and V8 and
  "795 tests" from V9b on. V7 and V8 had 33 runner tests, and 758 scenario runs and 2 ignored
  with them is 793; V9b's two more runner tests make 795. And V9 counts the Rune side's tests
  as distinct names, one more than V8's method gives.
- **The flaky run alone.** V7 saw it pass 4 of 4 times alone, V3 saw it fail 5 of 6 and V9b
  saw it fail 8 of 9. The records agree that it is unreliable and not on how.
- **`distant-archive-scrolled-into-view/icons`** failed once, at V3, and has no allowance. It
  passed in all three runs the matrix reads.

## Open after the slice

For the owner:

1. **Decisions 13, 14, 15, 18 and 26** ([above](#owner-decisions)).
2. **What the rulings left for the next slice:** the Appearance popover (21), undo (23), the
   naming of a new item (24), GTK's dialog for a failed rename (25); and after the slice, the
   Menu key (22).
3. **Two workflow changes V9b wrote up and did not make:** `test-image.yml` watching
   `crates/**/Cargo.toml`, and whether the base's tag moves with every dependency change
   ([V9b](V9b-test-image-and-exit-check.md#workflow-changes-this-package-did-not-make)).
4. **From the import:** archiving `wmfeht/strata-components`, and whole-workspace builds,
   which do not compile ([V4b](V4b-import-components.md#open-for-the-owner)).
5. **On `main`:** rebasing `cut.patch`, and fixing
   `new-folder-with-selection-groups/columns`. Each allowance goes when its cause is fixed.
6. **The component items:** Shift+Left in a column, the empty-folder page, and a gallery
   scene of the editor in Columns ([Gaps](#4-gaps)).
7. **A second host.** The plan names a session host, a real window in headless sway driven by
   a virtual keyboard and pointer, for what offscreen parity cannot prove. It is not built.
