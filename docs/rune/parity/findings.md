# Parity findings

What parity work has found that is not a package's own work: Strata's own bugs and flaky
GTK runs (`oracle`), runs or fuzz sequences where the two targets differ (`divergence`), gaps
in Rune bound for the filing packet (`rune-gap`), requests for a twin or a normalisation rule
(`exception`), and follow-ups to the shared seams. The codes are the labels of the plan's
"Coordination" section, and they keep their meanings.

## Recording a finding

Until 2026-11-12 each finding was a GitHub issue that also held its state. By the owner's
ruling of 2026-11-12 ([`owner.md`](owner.md#ruled)), a new finding is two things:

- **A reference issue on `lgse/strata`**, which is what the ledger, twins and the fuzz
  driver's files cite. It carries the code's label and `rune-parity`, and a short statement
  in the form of the archived issues, and it may be closed at once with a line pointing at
  this file. It is a reference, not a tracker: nothing more is recorded on it, and a ruling
  on it is recorded in `owner.md`. Closing it as soon as its entry here exists is the
  coordinating session's practice, not the owner's rule.
- **An entry in this file**, added in the pull request that meets the finding, in the form
  of the entries below: its code, two lines on what was seen, the run, seed or test it
  concerns, and its state.

Search before filing: this file first, by code and by scenario or seed name, then the issues
by label and by scenario or seed name. A finding met mid-run may
land as its own small docs-only pull request that merges quickly, so other packages see it
before the package's own pull request does. When a finding is fixed or set aside for good,
its entry says where, and stays.

An `oracle` finding is Strata's own bug, and its fix, if anyone takes it up, lands on `main`.
`main` keeps its own rules, so a fix there still starts from a bug issue on `main`, as
`AGENTS.md` asks; the entry here then links it.

**References.** The ledger's `parallel` entries, a twin's `parity.ref`, and the fuzz driver's
normalisation map, calibration mask and `parked` lists each cite the record behind them, and
their readers accept only two forms: an issue, `lgse/strata#<number>`, or, for a `rune`
twin, its packet entry, `docs/rune/upstream/rune/<number>-<name>.md`
(`tests/qa/twins.rs` and `scripts/qa_parity_check.py` today, and `tests/qa/fuzz/rules.rs`
once F1's pull request [#2181](https://github.com/lgse/strata/pull/2181) merges). A new finding's reference issue is that issue.
References to the archived issues below stay valid and need no change.

## Findings

Every archived issue below was closed on GitHub on 2026-11-12 with the package issues. A
closed issue is not a fixed finding: the state given here is the finding's.

### #2161: `cut.patch` no longer applies

- **Code:** `oracle`. [Archived issue](https://github.com/lgse/strata/issues/2161).
- `tests/qa/mutations/cut.patch` no longer applies since `src/ui/browser/clipboard.rs`
  changed under it on `main` ([#2092](https://github.com/lgse/strata/pull/2092)), so nothing proves `cut-moves-only-after-paste` can fail.
- **Concerns:** the GTK mutation patch `cut.patch` and the scenario
  `clipboard/cut-moves-only-after-paste`.
- **State:** open, set aside. `./scripts/qa-parity.sh slice` holds it as the
  `[[stale_patch]]` allowance in `tests/qa/parity/slice.toml` (slice decision 27). The fix,
  if taken up, is a rebase of the patch on `main`.

### #2162: `new-folder-with-selection-groups/columns` is flaky

- **Code:** `oracle`. [Archived issue](https://github.com/lgse/strata/issues/2162).
- After a click on the new folder `grouped`, its column does not take focus within 15 s.
  The run fails more often than it passes alone (1 of 9) and sometimes in the whole suite.
- **Concerns:** the GTK run `entry_management/new-folder-with-selection-groups/columns`.
- **State:** open, set aside. The `[[flaky]]` allowance in `tests/qa/parity/slice.toml` lets
  it fail in `slice`'s whole-suite part. While the allowance names it, its ledger entry can
  be neither `required` nor `parallel`, so [A7](packages/A7.md) cannot close it until the
  flake is fixed on `main` or the owner rules how a flaky oracle is carried.

### #2163: `distant-archive-scrolled-into-view/icons` failed once under load

- **Code:** `oracle`. [Archived issue](https://github.com/lgse/strata/issues/2163).
- In one full run of the suite the archive created off screen was never selected; alone it
  passed 8 of 8, and it has not been seen to fail since.
- **Concerns:** the GTK run `archive_reveal/distant-archive-scrolled-into-view/icons`.
- **State:** open, set aside. It has no allowance: if it fails in `slice`'s whole-suite part,
  `slice` fails.

### #2173: Shift+click on the entry being renamed does two different things

- **Code:** `oracle`. [Archived issue](https://github.com/lgse/strata/issues/2173).
- With the inline rename open on `todo.txt`, a Shift+click on that row clears the field's
  selection in 28 of 34 replays and selects the whole name in 6, from identical input.
- **Concerns:** fuzz seed `0x67` of version 1 of the `calibration` alphabet, found while F1
  calibrated the driver; step 34 of its sequence.
- **State:** open, set aside. Parked in `tests/qa/fuzz/corpus/calibration/parked` in [#2181](https://github.com/lgse/strata/pull/2181),
  and the `calibration` alphabet no longer clicks an entry while a name is edited.

### #2174: a salvaged `settings.toml` gets every default written into it

- **Code:** `oracle`. [Archived issue](https://github.com/lgse/strata/issues/2174).
- A `settings.toml` without `mode` or `theme` is salvaged entry by entry, and the first
  change then saves every default into it, pinning them and overwriting keys another process
  wrote.
- **Concerns:** `salvage_preferences`; pinned by
  `settings::saving::tests::a_salvaged_file_is_saved_with_every_value_loading_filled_in` in
  `strata-desktop`. Found by [B2](packages/B2.md) while building [#2172](https://github.com/lgse/strata/pull/2172).
- **State:** open, set aside. Both interfaces share it by construction since [#2172](https://github.com/lgse/strata/pull/2172) and
  [#2178](https://github.com/lgse/strata/pull/2178); a fix lands on `main` and both follow by merge.

### #2175: timing-sensitive GTK unit tests

- **Code:** `oracle`. [Archived issue](https://github.com/lgse/strata/issues/2175).
- Three GTK unit tests each failed once in CI's shard 1 on a pull request to `strata-next`
  and passed on a rerun of the whole workflow; a fourth failed once on a developer's host.
  Since then: the fourth failed again on a loaded host, once in a run of 417 tests and 1 time
  in 5 alone, on [#2195](https://github.com/lgse/strata/pull/2195); the third failed again in CI's shard 1 on [#2197](https://github.com/lgse/strata/pull/2197), with its recorded
  message; a fifth, a `strata-fs` test with no GTK in it, failed in shard 2 of that same
  run, its search coming back empty. The rerun, where those two passed, failed on a third
  test, which is not a flaky test but a fault of the updater: see "The updater shows
  'Interrupted system call'" below.
- **Concerns:**
  `ui::browser::progress::tests::minimization::deletion::overlapping_fast_deletions_restore_browser_input_after_playback`,
  `ui::window::tests::keyboard_dispatch::direct_commands::shipped_keys_never_reach_the_direct_stage`,
  `ui::window::tests::keyboard_dispatch::pane_focus::default_map_tab_leaves_the_listing_in_one_press`,
  and, added in a comment,
  `ui::window::tests::keyboard_dispatch::folder_jump::tenxer_z_ranks_history_and_opens_the_chosen_folder_once`;
  and, in `strata-fs`,
  `services::search::tests::folder_path_search_leads_with_a_folder_the_query_names_outright`
  (`crates/strata-fs/src/services/search/tests.rs:819`).
- **On [#2181](https://github.com/lgse/strata/pull/2181):** the second failed in CI's shard 1 at `7963d110f` and again in the one
  rerun of the whole workflow, and the third failed in the first of those two runs. That
  change is to the QA runner, its scripts and documentation, and touches no `src/` and no
  crate; `./scripts/quality.sh test` passed at that commit in the pinned image. With
  `strata-next` merged at `734d67f6c`, a sixth,
  `ui::window::tests::keyboard_dispatch::icon_motion::tenxer_icons_move_spatially_open_explicitly_and_peek`,
  failed once in `./scripts/quality.sh test` on a loaded host (`Key(104) landed on ["empty",
  "tile-03.txt", "tile-03.txt"]`) and then passed 6 times of 6 alone.
  At `6f1605908` a seventh,
  `ui::preview::tests::audio::audio_steps_keep_playback_and_volume_but_never_reuse_an_ended_request`
  (`src/ui/preview/tests/audio.rs:27`, "rendered stream"), failed in CI's shard 2, where
  `./scripts/quality.sh test` had passed at the same commit in the pinned image.
- **On [#2200](https://github.com/lgse/strata/pull/2200):** the sixth failed once in CI's shard 3 at `a6c6708c2`, with the same
  message (`src/ui/window/tests/keyboard_dispatch/icon_motion.rs:195`), and passed in the
  one rerun of the whole workflow. That change is to `crates/strata-app`, which the GTK lane
  does not compile, and to Markdown.
- **On [#2204](https://github.com/lgse/strata/pull/2204):** an eighth,
  `ui::preview::tests::video::video_steps_reuse_one_view_and_keep_playback`
  (`src/ui/preview/tests/video.rs:156`, "rendered stream", the seventh's message in its
  sibling), failed once in CI's shard 3, in the pull request's first run. That change adds
  key constants and two characters to the table key backend, which the GTK application
  does not install; `./scripts/quality.sh test` had passed with it in the pinned image.
- **State:** open, set aside. Whoever meets one of these in a red shard reads the failure
  first: if the change could reach the test, it is the change's; if not, say why on the pull
  request, rerun the whole workflow once, and stop if the same test fails twice. A new flaky
  GTK test is added to this entry. When the rerun fails on another test that the
  change cannot reach, as on [#2197](https://github.com/lgse/strata/pull/2197), the orchestrating session's ruling of 2026-11-12 is: record
  the sighting and rerun the whole workflow once more; a test that fails a second time on the
  same pull request stops the agent again.
- **Sightings on [#2202](https://github.com/lgse/strata/pull/2202)**, which changes ledger
  rows, Rune patches and docs only: `shipped_keys_never_reach_the_direct_stage` failed in
  shard 1 (the assertion at `direct_commands.rs:509`, Shift+O's `open_with`); on the
  workflow's rerun, `default_map_tab_leaves_the_listing_in_one_press` failed in shard 1
  instead. Both are already listed above.
- **With CI off on `strata-next`** (the owner's instruction of 2026-11-12), a sighting now
  comes from `./scripts/quality.sh test`. The rule is unchanged: record it here, rerun the
  test once alone with `./scripts/test-headless.py <name>`, and do not chase it.

### #2176: a GTK critical after a file appears during a rename on a multiple selection

- **Code:** `oracle`. [Archived issue](https://github.com/lgse/strata/issues/2176).
- Seven steps (a multiple selection, F2, a file written into the folder, Return, Up, Return)
  make GTK log `gtk_selection_model_selection_changed: assertion … failed`, every time.
- **Concerns:** fuzz seed `0x19b` of version 2 of the `calibration` alphabet, shrunk from 40
  steps to 7 and confirmed 3 of 3.
- **State:** open, set aside. Parked in `tests/qa/fuzz/corpus/calibration/parked` in [#2181](https://github.com/lgse/strata/pull/2181).

### #2177: after Move to Trash on a filtered pane, the preview shows another file

- **Code:** `oracle`. [Archived issue](https://github.com/lgse/strata/issues/2177).
- With the drawer open and the pane filtered, moving `report.md` to the trash from its
  context menu leaves the preview on `todo.txt`, neither focused nor selected, in 23 of 24
  replays, and on the focused entry in the other.
- **Concerns:** fuzz seed `0xb1` of version 2 of the `calibration` alphabet; step 35 of its
  sequence, which did not shrink.
- **State:** open, set aside. Parked in `tests/qa/fuzz/corpus/calibration/parked` in [#2181](https://github.com/lgse/strata/pull/2181).

### #2179: the windowed Rune host does not pump GLib again after a queued update

- **Code:** none of the plan's: a follow-up to the shell's windowed host, labelled `bug`.
  [Archived issue](https://github.com/lgse/strata/issues/2179).
- A GLib source that a queued update attaches (a `spawn_local` task, a timer) is not
  dispatched until the next pump, so a save's result from the shell's own save is heard a
  pump late. Found by reading, in the review of [#2178](https://github.com/lgse/strata/pull/2178); not seen in a running window.
- **Concerns:** `crates/strata-app/src/host/windowed.rs` (`pump`, then `shell::settle`) and
  `crates/strata-platform/src/glib_loop/mod.rs`. The offscreen host does not behave this way.
- **State:** open, not taken up. `shell.rs` and `host/` are the contract's files, so the fix
  is a `contract` pull request, with a test on the windowed host.

### #2180: under load, a click that opens a folder leaves focus in two places

- **Code:** `oracle`. [Archived issue](https://github.com/lgse/strata/issues/2180).
- A click on the folder `pictures` left focus on its column in one replay and on its first
  entry in the other, with the machine loaded four times over. Seen once, in 300 seeds.
- **Concerns:** fuzz seed `0x41c` of version 3 of the `calibration` alphabet; step 2 of its
  sequence. It may share a cause with [#2162](https://github.com/lgse/strata/issues/2162).
- **State:** open, set aside. Parked in `tests/qa/fuzz/corpus/calibration/parked` in [#2181](https://github.com/lgse/strata/pull/2181).

The fuzz issues name their sequence files with the alphabet's version, and those are
the names [#2181](https://github.com/lgse/strata/pull/2181) merges with:
`tests/qa/fuzz/corpus/calibration/fuzz-calibration-v<version>-<seed>.jsonl`, the seed as
sixteen hex digits (`fuzz-calibration-v1-0000000000000067.jsonl` for [#2173](https://github.com/lgse/strata/issues/2173)). A seed names a
sequence only together with the alphabet's version, so the version is part of the name.

### #2193: a GTK test whose name string is stale passes without running

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2193), filed on 2026-11-12, before findings moved to
  this file.
- `strata_test_support::isolated_test`, which `gtk_test` wraps, runs a GTK test in a child
  process started with `--exact <name>`, a string the test writes by hand, and asserts only
  that the child exits successfully. With a stale name the child matches no test, exits 0,
  and the test is reported as passed.
- **Concerns:** every test that calls `gtk_test`, `gtk_test_with_env` or `isolated_test`,
  and `crates/strata-test-support/src/lib.rs`. Found by [B2](packages/B2.md) while building
  [#2195](https://github.com/lgse/strata/pull/2195): three tests of a file that had moved passed without running, and the file
  was moved back. Two scans of the name strings, the author's and the reviewer's, found none
  stale then.
- **State:** open, not taken up. Until the harness checks that the child ran its test, a
  move or rename of a test file, module or function takes the name strings with it; in a
  log the only sign is a GTK test printed once and not twice. A fix lands on `main`.
  `strata-app`'s own tests are guarded since [#2237](https://github.com/lgse/strata/pull/2237), where the fault was met again in a test
  inside a nested module: `test_support::in_own_process` has the child leave a marker and
  fails without one. The whole Rune lane ran with the guard and no existing test failed it.
  That guard is the small form. The fix proper is in the shared helper,
  `strata_test_support::isolated_test`, checking that the child ran one test; it is a shared
  crate's and lands on `main` first. Until then the tests that call it directly are not
  guarded: the GTK application's (`gtk_test` under `src/`), and those of `strata-fs`,
  `strata-desktop` and `strata-core`. No other crate of the Rune side runs a test in a
  child. The guard has one gap left: a test that gives the whole path of another test that
  exists passes, because the child runs that other test, which leaves the marker.

### The updater shows "Interrupted system call" when a signal arrives during a download

- **Code:** `oracle`. No issue: recorded here on 2026-11-12. A fix lands on `main` and starts
  from a bug issue there, which is the owner's to file.
- `IdleLimited::await_input` (`crates/strata-desktop/src/services/update_install/download.rs`)
  waits for the server in slices so that a cancellation is noticed, and hands any error of a
  slice but a timeout up as the download's failure. The wait is a read with a receive
  timeout, which the kernel does not restart after a signal, so a signal that arrives during
  it ends the download with "Could not download signed update metadata: Interrupted system
  call". The GTK application catches `SIGCHLD` through GLib's child watches, so a child
  process that exits during a metadata fetch can show the user that message.
- **Concerns:** the updater on both branches. Seen once in CI, on [#2197](https://github.com/lgse/strata/pull/2197), shard 1 of the
  second attempt of its first run:
  `services::update_install::tests::cancel_stops_a_silent_metadata_fetch_without_waiting_for_a_timeout`
  (`update_install/tests.rs:927`) got that failure where it expected a cancellation. What
  interrupted the read there is not known: nothing in the shared crates installs a signal
  handler, and the test passed 80 of 80 times on a host.
- **State:** open, not taken up, and not to be fixed on `strata-next`. Until it is fixed on
  `main`, the same test can fail a shard; whoever meets it reads the failure, and it is this
  entry and not a new flaky test.

### #2201: Space in a column's sort menu does two different things

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2201), filed on 2026-11-12 as the finding's reference and
  closed at once: its state is kept here.
- With a column's sort menu opened from the keyboard (Up, Right, Right, Return) and left
  open, after a right-click and Down, Space puts the folder back in its default order, by
  name, in 16 of 24 replays, and in another order in the other 8. The menu stays open
  either way. The input logs are identical and every step is at rest.
- **Concerns:** fuzz seed `0x1de` of version 3 of the `calibration` alphabet; step 32 of its
  sequence, `{"key":"space"}`, where `app.matches` and `app.menu` differ. The shrinker takes
  the seed from 40 steps to 10 and calls it noise, since the shortest sequence differed in 2
  confirmations of 3. To reproduce:
  `./scripts/qa-fuzz.sh replay tests/qa/fuzz/corpus/calibration/fuzz-calibration-v3-00000000000001de.jsonl --repeats 24 --jobs 1`.
- **State:** open, set aside, parked. Parked in `tests/qa/fuzz/corpus/calibration/parked`
  in [#2181](https://github.com/lgse/strata/pull/2181), with the seed's first 32 steps kept beside it.

### #2218: `mise run test-platform` fails under load on a developer machine

- **Code:** none of the plan's: a follow-up to the compositor tests' task, labelled
  `testing`. [Issue](https://github.com/lgse/strata/issues/2218), filed on 2026-11-12 as the finding's reference; its state is
  kept here.
- On a 24-core machine with a load average of 30 to 39 from other builds, the whole task
  failed in `strata-platform`'s own session tests and so never reached the gallery's or
  `strata-rune`'s window tests: 34 of 177 with [B3](packages/B3.md)'s registry branch, and 22
  of 177 on plain `strata-next` (20 of them `x11_*` tests). Each failure is a wait that
  timed out (`waiting for attach`, `waiting for ready`, `waiting for the window`), and the
  set differs from run to run. The sets that failed pass alone on the same machine
  (`p0::` 26 of 26, `p2::x11` 22 of 22).
- It is neither the machine's compositor nor a change on a branch. Every wait is 10 s times
  `STRATA_TEST_TIMEOUT_SCALE` (`crates/strata-platform/tests/support/mod.rs`), which was 1
  in all of these runs, while the task starts one sway and one Xwayland for each test, as
  many at once as the machine has cores. With the scale at 3 the whole task passed once, on
  the same loaded machine and commit: 177, then the gallery's 23 and 14, then
  `strata-rune`'s 3. That is one run, not a measure of how often it passes, and scale 3 is
  not enough: see the next point.
- **2026-11-12, with scale 3 in force** (the task's default since
  [#2219](https://github.com/lgse/strata/pull/2219)): [A2](packages/A2.md)'s gate for
  [#2247](https://github.com/lgse/strata/pull/2247), at `aef813211` (`strata-next`
  `6a55189c6`), on the same 24-core machine. The load average was not recorded during the
  run; five minutes after it ended it was 48.7, 45.4 and 34.7 (1, 5 and 15 minutes), from
  other packages' builds and QA runs. `strata-platform`'s session tests passed 174 of 177
  and failed 3: `p2::x11_primary_selection_round_trip`,
  `p5::a_snapshot_that_changes_nothing_runs_no_view` and
  `p5::a_snapshot_with_new_values_runs_the_view`. Two of them are `p5` tests, not X11 ones.
  Each passed when rerun alone (`mise run test-platform <name> -- --exact`). The task
  stopped there, so the gallery's `x2` and `x3` did not run; `strata-rune`'s window tests,
  run alone, passed 3 of 3.
- `docs/rune/testing.md` said that CI sets the scale to 3 and runs these tests in
  `.github/workflows/platform.yml`. Neither was so: no workflow sets the variable, none runs
  the tests, and that file has never been in the repository. Both lines are corrected with
  this entry. The comment on `timeout` in `crates/strata-platform/tests/support/mod.rs`
  says "CI sets 3" too, and is left for whoever takes the harness.
- **Concerns:** `mise run test-platform` (`mise.toml`), and every package that changes a
  host, the shell or `strata-rune`, whose window tests are the last thing that task runs.
- **A likely cause, measured on 2026-11-12:** the same machine's load came largely from the
  gallery's PNG goldens and scripts, which each start a `strata-gallery` process that opens
  a Vulkan device on the real GPU (no lavapipe is installed), where the opens serialise in the
  driver. One render alone took 0.63 s; throughput peaked at 8 at once (6.1 a second) and
  fell beyond it (5.4 at 24, 4.5 at 48); at 72 at once, 70 to 72 threads sat in
  uninterruptible sleep. A Rune-lane run started about 1,400 of them (514 PNG goldens and
  897 scripts), 24 at a time, while
  three package agents ran lanes at once, and the load average passed 40. The compositor
  tests themselves were not measured under that load, so this is the likely cause, not a
  shown one. Those gallery tests are no longer run as validation, by the owner's instruction
  of 2026-11-12 ([`owner.md`](owner.md#ruled)).
- **State:** open, unowned. The task now sets the scale to 3 when it is unset, by the
  orchestrating session's ruling of 2026-11-12, which gives a loaded machine more time and
  does not remove the cause: three tests still failed with it under load (above). The thing to fix is the number of sessions started at once: a
  limit on concurrent sway sessions in the harness, or on the test threads the task asks
  for. [`working.md`](working.md#while-building) has how to run the window tests alone.

### #2235: the Menu key with the cursor's entry scrolled out of view

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2235), filed on 2026-11-12 as the finding's reference and
  closed at once: its state is kept here. Its title and first statement were written from
  reading, before the run below, and say only that nothing opens.
- **By run**, on the GTK target in the pinned container (`./scripts/qa-drive.sh`), with 400
  files, the cursor on the first and selected, and the wheel turned until that row was out
  of view; each key from a fresh launch:
  - Columns and Icons: the Menu key and Shift+F10 open nothing, and nothing scrolls.
  - List: both keys open the menu of `file-154.txt`, the entry drawn at the top of the list
    where the cursor's row was last placed, and that entry becomes the only selection. The
    menu's rows (Move to Trash, Rename, Cut) then act on a file the user did not choose.
- **By reading:** `open_focused_context_menu` finds the place from the cursor's row. The
  Columns view asks for a mapped row (`shown_row_at`) and has none. The mode views take the
  row's widget "mapped or not" (`item_widget`) and its last bounds, and hand that point to
  the trigger a right-click uses, which acts on whatever is drawn there. Why Icons then
  opens nothing was not traced. A row partly in view was not run: by reading it is mapped
  in every view, so its own menu opens at it.
- **Concerns:** `src/ui/browser/view_filter.rs:78`, `src/ui/browser/columns.rs:137`,
  `src/ui/browser_modes/context_menu.rs:70` and `src/ui/browser_modes/pane.rs:75`. No
  scenario presses the Menu key with the cursor out of view.
- **State:** open, not taken up. List's behaviour is a defect to fix on `main`: a menu with
  destructive rows opens for an entry the user did not choose. The bug issue there is the
  owner's to file and schedule; this entry and its reference are not that issue. The Rune
  interface's List view deliberately does not reproduce it: it opens nothing, as Columns
  and Icons do, by the orchestrating session's ruling of 2026-11-12
  ([`owner.md`](owner.md#b3-commands-and-key-dispatch)). A difference between the targets
  here is therefore not a Rune fault.

### #2254: the QA engine finds no viewport for a pane whose scroll pane is inside its entry list

- **Code:** `divergence`, of the harness. [Issue](https://github.com/lgse/strata/issues/2254), filed on 2026-11-12 as the finding's reference and
  closed at once: its state is kept here.
- **Seen:** on Rune, `keyboard_navigation/return-restores-nested-scroll/icons` stops at line 4,
  its `middle_visible` note reading nothing, before the position restore it checks.
- **By reading:** the shared engine takes a pane's viewport to be the nearest ancestor of the
  entry list with the role `scroll pane` (`viewport` in `crates/strata-qa/src/engine/locate.rs`).
  Every C1e collection puts its container role on its outer element and its vertical scroll
  viewport inside (`crates/strata-components/src/collection/element/mod.rs`, about lines
  900-935), so no ancestor qualifies: the Icons grid (C3) shows it. The List view passes for
  another reason: C4 wraps its collection in its sideways scroll viewport, an ancestor, so on
  Rune List `middle_visible`, `visible` and `visible_top` read the horizontal pane. That is
  close enough for the middle-entry note; a `scroll_value` check would read the sideways
  position.
- **Reach:** of the scenarios that read the viewport, the only Icons one is this run. One List
  run is `required`, `return-restores-nested-scroll/list` (A2's), whose `middle_visible` note
  read the sideways strip; its position checks read the entry's own box on screen, not the
  viewport, so it did not pass for the wrong reason, and it passes on the corrected node too.
  No `required` Columns run reads the viewport.
- **State:** fixed by [H3](packages/H3.md)'s first pull request, a `contract` change to the
  engine: a pane's viewport is the scroll pane inside its entry list nearest it, not looking
  inside an entry, and only when there is none the nearest one around it. GTK's lists hold
  none and read as before; an entry is skipped as the backend lists the container's entries,
  since a GTK row widget carries no mark of its own. On Rune the Icons grid has its viewport,
  List reads its rows' vertical viewport instead of C4's sideways strip, and Columns reads
  each column's own viewport instead of C2's sideways strip, as GTK reads a column's.
  `return-restores-nested-scroll/icons` flips when the coordinator asks.

### #2269: a `strata-platform` icon loader test failed once under load

- **Code:** none of the plan's: a flaky Rune-side unit test, labelled `testing`.
  [Issue](https://github.com/lgse/strata/issues/2269), filed on 2026-11-12 as the finding's
  reference and closed at once: its state is kept here.
- **Seen:** in a full `mise run test-rune` for [A1](packages/A1.md)'s
  [#2264](https://github.com/lgse/strata/pull/2264) gate, while a `./scripts/qa-parity.sh all`
  build loaded the machine, the held lookup in the test's `hold` helper stayed `Pending` where
  `NotFound` was wanted, after its 10 s wait (`loader/tests.rs:116`). The branch did not touch
  `strata-platform`. Alone, the test passed 3 of 3.
- **Concerns:** `icon_theme::loader::tests::a_request_dropped_and_pushed_out_of_the_recent_icons_before_its_turn_is_skipped`
  in `crates/strata-platform/src/icon_theme/loader/tests.rs`.
- **What it suggests:** Rune-side tests are meant to run on Rune's virtual clock, but this one
  drives the loader's real lookup thread against a wall-clock deadline, so it depends on
  thread timing. A candidate, not verified: `LoadedIcon::wait` returns once a lookup's outcome
  is set, before the lookup thread calls the waker; a `hold` that shuts the gate in that window
  holds the thread in the waker for the previous lookup, and the held one never runs.
- **State:** open, not taken up. Rerun it alone once if it fails again; do not chase it in a
  package's gate.

### #2277: an inline rename ends on Rune when an entry is listed above its row

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2277), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in a hosted test written for [A7](packages/A7.md)'s
  [#2276](https://github.com/lgse/strata/pull/2276) (`new_item/tests.rs`): a new folder listed
  above the entry being renamed ended that edit about 120 ms later, as
  `RenameIntent::Keep { why: Cancelled, by: Scrolled }`, and what was typed was lost. GTK, by
  reading, keeps the edit: the insertion is a positional `items_changed`
  (`src/ui/browser/events/listing.rs`, `src/ui/entry_list_model.rs`), the edit is a lease on
  its bound entry (`src/ui/collection_edit.rs`), and the cursor restore skips a focused editor
  (`src/ui/browser/columns/cursor.rs`). No run confirms it yet.
- **Cause, by reading:** `RenameState::tick` (`crates/strata-components/src/rename/state.rs`)
  cancels by `Scrolled` once the edited row's index leaves `visible_range()`, the range last
  reported and so a frame old (`crates/strata-components/src/collection/element/state.rs`): an
  insertion above pushes the row's index past its end while the row stays on screen.
- **Concerns:** the C5 rename editor in every view.
- **State:** fixed by [A7](packages/A7.md)'s `contract` pull request
  [#2283](https://github.com/lgse/strata/pull/2283): `RenameState::tick` counts the row as
  scrolled out only when both `CollectionState::shows_entry` (its place by the last layout,
  against the scroll now) and `visible_range` say so, since each goes stale for a frame on its
  own (an entry listed above; the lines flowing anew at a tile size). GTK keeping the edit is confirmed by running it: the new
  scenario `inline_renaming/rename-survives-entry-listed-above` passes on both targets, and
  fails on Rune with the old check, at its line 13.

### #2290: a list's visible rows reported as none, and never again

- **Code:** `rune-gap`. [Issue](https://github.com/lgse/strata/issues/2290), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** `columns::tests::fills` (`crates/strata-app`) fail every time once the sidebar
  ([A3](packages/A3.md)) or the location bar ([A5](packages/A5.md)) is mounted: the first
  column's `on_visible_rows` reports `0..5`, then `0..0` in the same frame, and never again,
  so it asks for no fills and its sizes and dates stay unknown until it scrolls. It takes
  animations on, the folder opened before the first frame, and text laid out ahead of the
  column in those frames (three or more sidebar rows, or the crumbs revealing their end).
- **Concerns:** every collection's visible range (C1e): fills, thumbnails, the rename
  editor's scroll check, in every view.
- **State:** worked around, not cut down to a Rune-only repro. strata-components'
  collection catches its range up from its own layout when the last report is empty
  (`catch_up_visible`, U-C1f-7). Rune packet entry
  [101](../upstream/rune/101-stale-visible-rows.md) is drafted. The view's place in the tree
  is not the cause: with A3's sidebar pull request keeping the view at one path whether the
  sidebar is drawn or not, and the catch-up turned off, the two fills tests still fail with
  the sidebar mounted (checked once, 2026-11-13).

### #2296: `browser-workers-reuse-processes/list` started one sandbox too many once

- **Code:** none of the plan's: a flaky required Rune run, labelled `testing`.
  [Issue](https://github.com/lgse/strata/issues/2296), filed on 2026-11-13 as the finding's
  reference and closed at once: its state is kept here.
- **Seen:** in a full `./scripts/qa-parity.sh all --target rune` for
  [#2294](https://github.com/lgse/strata/pull/2294)'s gate, the run failed at line 49: the
  log counted 4 "browser sandbox started" where the step wants `$initial` + 1, 3. Alone, it
  passed 3 of 3. Again in [#2306](https://github.com/lgse/strata/pull/2306)'s gate, at the
  same line with 5 counted; alone, 3 of 3 passed. The run turns no wheel and clicks no
  clipped target, which is all #2306 changes. Not touched by #2294, by its review: that change routes the collection's
  caught-up visible range through `route::on_visible` and skips lines that hold no entry,
  and neither reaches when the render pool starts or discards a worker.
- **Concerns:** `browser_workers/browser-workers-reuse-processes/list`, `required` since
  [#2263](https://github.com/lgse/strata/pull/2263); the render pool in
  `crates/strata-app/src/thumbnails/`.
- **Cause, traced:** with log lines in the viewer, the pool and `Lease::execute` (a scratch
  build, since removed), and reproduced alone with the machine's cores kept busy: 2 of 6
  runs failed at line 49, one with 5 counted. Rune reports a folder's rows as none before
  its list has laid them out, and reports them again once they show. Opening `photos-c`,
  the List poll usually hears that empty report before the listing has entries, and asks
  for nothing until the rows show. Under load the listing could come first: the poll then
  resynced the store with the last viewport and asked for four rows, and the empty report,
  heard next, made the store drop them, so the viewer cancelled all four, two of them
  already rendering. A cancelled render errors, `Lease::execute` discards its worker, and
  each discard costs a sandbox start: 2 extra, 5 counted. The cancel rule is GTK's, so the
  difference is what Rune cancels on: GTK cancels a running render only when the last row
  waiting for it is unbound (`install_edit_unbind` in `src/ui/browser_modes/pane.rs`,
  `cancel_thumbnail_request` in `src/ui/thumbnail.rs`; `demote_offscreen` drops queued
  jobs only), and a GTK row is not unbound while its folder loads. Not the worker kill: it
  adds the one start line 49 expects, and the extra starts are the cancels'. Every folder
  the run opens is opened from its parent, whose viewport the poll resyncs with, so the race
  can come at any of the three: in H3's gate (2026-11-13) a full Rune run failed at line 31
  instead, 4 counted where at most 2 may be, in `photos-b`, before the kill; alone it then
  passed three rounds. Nor
  [#2294](https://github.com/lgse/strata/pull/2294): the empty report is Rune's, and #2294
  changes only how the collection's catch-up after one is heard.
- **Fix:** [#2313](https://github.com/lgse/strata/pull/2313). The poll rule List and Columns
  share (`Synced::next` in `crates/strata-app/src/columns/thumbs.rs`) passes over an empty
  range while the listing has entries; an empty folder's still syncs. Both polls tell the
  store through one `Sync::tell`. `a_report_of_no_rows_in_a_listed_folder_cancels_no_render`
  (`thumbnails/tests.rs`) fails without it. The Icons grid syncs only on Rune's report and
  never asked early, so it did not race.
- **Related:** [#2290](#2290-a-lists-visible-rows-reported-as-none-and-never-again) is the
  same Rune report, an empty range for a collection that lists entries; #2290 is the variant
  where no second report comes (Rune packet entry
  [101](../upstream/rune/101-stale-visible-rows.md)), and here the second report does come.
  `Synced::next` now passes over both for List and Columns.
- **Not covered:**
  - **Icons** syncs every report, empty ones included (`on_visible` and `sync_thumbs` in
    `crates/strata-components/src/icons_grid/mod.rs`), so an empty report there after its
    requests are out would still cancel running renders. Not seen: the grid asks only on
    Rune's reports, so no request is out before the first one.
  - **The `Sync::Again` guess** uses the last viewport, the parent folder's, for a folder just
    opened. When the first real report differs, as with Back into a folder whose scroll
    position `Pane` restores, the store still cancels the guessed requests, running ones
    included, which GTK never asks for.
  - Both are left for a rule in the store if they show up.
- **State:** fixed by [#2313](https://github.com/lgse/strata/pull/2313). A failure of the
  run in a gate after it is a new sighting, not this one.

### #2310: Return does not press a focused button on the Rune interface

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2310), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A5](packages/A5.md) while testing the pane header (#2307): Return on a focused
  Back did nothing where Space pressed it. GTK's buttons take Return and Space alike; on Rune it
  holds for every button.
- **Cause, by reading Rune at `2a44156`:** Rune follows AppKit. Return goes to the window's
  default button and presses only a focused row, link or `focus_highlight` element (`takes_return`
  in `crates/rune/src/runtime/input/focus.rs`); Space presses a focused control. C15's dialogs
  already turn Return into Space (`crates/strata-components/src/dialog/keys.rs`).
- **Concerns:** every button of the Rune interface.
- **State:** worked around by A5's pull request for it: `crates/strata-app/src/buttons.rs` gives
  the pane header's buttons, an unreadable folder's Retry and Open as Administrator, and the
  window header's actions an `on_key` that sends Space for a plain Return. Rune packet entry
  [103](../upstream/rune/103-return-presses-a-button.md) asks for an opt-in. A button another
  part adds takes `buttons::on_return` too: `buttons::tests::guard` walks each view's window,
  sidebar shown, and fails on a focusable button that did not come through it, unless it sits
  in a list, listbox, menu, tree or grid, or in one of C15's dialogs, or is on its exceptions
  list (the window's own buttons, an
  open question in [`owner.md`](owner.md), A5).

### #2322: `scrolled-away-crumbs-and-columns-stay-reachable/columns` left focus nowhere once in a full Rune run

- **Code:** none of the plan's: a flaky Rune run, labelled `testing`.
  [Issue](https://github.com/lgse/strata/issues/2322), filed on 2026-11-13 as the finding's
  reference and closed at once: its state is kept here.
- **Seen:** in a full `./scripts/qa-parity.sh all --target rune` for
  [#2321](https://github.com/lgse/strata/pull/2321)'s gate, the run failed at line 37: after the
  resize to 640 (line 33) and Home onto bravo (line 35), Right on bravo left nothing with the
  keyboard after 15 s (`focused` read null where charlie was wanted). The whole set again passed
  163 of 163, and the run alone 11 of 11 after the change and 3 of 3 before it; the solo rounds
  overwrote the failing run's artifacts.
- **Concerns:** `locations/scrolled-away-crumbs-and-columns-stay-reachable/columns`, which
  #2321 would have made `required`. The failure is in the Columns keyboard steps, before the run
  reaches the crumbs #2321 changes; before #2321 the run stopped at line 52 and never reached
  line 37 in a full run.
- **Resembles:** [#2162](https://github.com/lgse/strata/issues/2162) (on GTK, an opened folder's
  column does not take focus within 15 s) and [#2180](https://github.com/lgse/strata/issues/2180)
  (on GTK, under load, a click that opens a folder leaves focus in two places). Not traced.
- **State:** open, unowned. The run stays `pending` on `rune` with this as its blocker, by the
  coordinator's ruling: the flaky allowance may not name a `required` run
  ([`owner.md`](owner.md#h1-whole-corpus-ledger), H1 item 11).

### #2324: the Rune List view's rows are table rows where GTK's are list items

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2324), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **What differs:** what a screen reader hears of the List view. GTK's List is a
  `gtk::ListView` (`src/ui/browser_modes/list_pane.rs`), a `list` of `list item`s, under a box
  of heading `button`s (`list_headings.rs`). C4 set its collection's roles to
  `Roles::GRID_ROWS`, Rune's `grid` and `row`, which AccessKit's AT-SPI adapter reports as
  `table` and `table row` (`accesskit_atspi_common` 0.21, `Role::Row => TableRow`), with each
  row's cells `gridcell`s and its headings `columnheader`s in a `row`. The roles Rune exposes
  differ, not only the QA backend's role map.
- **Seen, by reading the scenarios:** `rename_visibility/committed-rename-stays-visible`
  aims at `{"role":"list item","name":"b"}` in List at its line 34, and
  `view_switching/list-column-drag-applies-on-release`, `list-column-resize-tracks-pointer`
  and `chooser_selection/chooser-heading-escape-keeps-selection` at the headings as
  `{"role":"button","name":"Size"}`. Each still waits on another part first.
- **State:** fixed by [H3](packages/H3.md)'s [#2329](https://github.com/lgse/strata/pull/2329), a `contract` change to C4:
  the collection takes `Roles::LIST` (Rune's `list` and `listitem`, AccessKit's `List` and
  `ListItem`, which AT-SPI reports as `list` and `list item`, as GTK's; a `listbox` would be
  heard as a `list box`), its cells take no role and its Name cell none either, so the name
  is not heard twice, and its heading strip is a box of `button`s. The List probe's roles are
  `("list", "list-item")`. Nothing a user does changes: the
  collection's container is the one focus stop for both roles and routes its own keys, so
  Rune's role-keyed focus rules (`focus.rs`) do not reach its items. Columns' columns are
  `listbox`es already, and the Icons grid stays `grid` and `gridcell`, as GTK's `GridView`
  is a grid.

### #2331: the Rune List view's column headings are no Tab stops where GTK's are

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2331), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **What differs:** GTK's List headings are plain `gtk::Button`s
  (`src/ui/browser_modes/list_headings.rs`), with nothing keeping the keyboard off them, so
  they are Tab stops that Return and Space press (to sort). C4's headings are no Tab stops: the
  table is one focus stop, by an owner ruling C4's module docs record ("Headings aren't Tab
  stops", `crates/strata-components/src/list_table/mod.rs`); assistive technology presses a
  heading through its press action, and the keyboard cannot reach one.
- **Reach:** by reading, no scenario Tabs onto a heading. The gap is older than
  [#2329](https://github.com/lgse/strata/pull/2329), which made the headings `button`s for
  assistive technology and left them unfocusable. Found in #2329's review.
  The columns menu "Customize columns…" opens gives the keyboard back to the table where
  GTK's gives it to the "Name" heading ([#2399](https://github.com/lgse/strata/pull/2399)'s
  review): `list::customize_columns` focuses the table, and would focus `head-name` once the
  headings take focus.
- **State:** open, waiting for the owner ([`owner.md`](owner.md#a8-view-switching-and-appearance),
  A8 item 1). C4's "owner ruling" was a reading: the owner's sign-off of 2026-11-10 covers
  the deviations marked OWNER in `docs/rune/work/STATUS.md`, and C4's row does not mark this
  one. Meanwhile the headings stay as they are, and C4's docs call it a deviation pending the
  owner's confirmation. [A8](packages/A8.md) builds focusable headings only if the owner
  asks for them.

### #2340: the thumbnail-size slider pages by 10 steps and steps onto its grid

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2340), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in the review of [#2336](https://github.com/lgse/strata/pull/2336). GTK's scale
  (`set_increments(16.0, 1.0)` in `src/ui/browser_modes/icons_controls.rs`) pages 1 px and steps
  16 from its value (65 to 81 after a drag); C20a's `Slider` pages its `Range`'s 10 steps and
  lands a step on the next grid point (80), as its module docs choose.
- **Concerns:** C20a's slider (`crates/strata-components/src/controls/slider/`), and through it
  the Icons pane header's "Thumbnail size" ([A5](packages/A5.md)). No QA run reaches these keys.
- **State:** open, C20a's, by the coordinator's placing; #2336 leaves C20a as it is.

### #2341: a modal popover cannot also close on a wheel outside it and let the wheel through

- **Code:** `rune-gap`. [Issue](https://github.com/lgse/strata/issues/2341), filed on 2026-11-13
  as the finding's reference and closed at once: its state is kept here. The issue's text
  overstates the gap, as first drafted; this entry is the corrected record.
- **Seen:** in the review of [#2336](https://github.com/lgse/strata/pull/2336). GTK closes the
  Icons header's "Thumbnail size" popover on a wheel outside it and scrolls the grid
  (`dismiss_on_outside_scroll`, `src/ui/scrolling/popover.rs`). A Rune popover made with
  `El::modal` makes the outside inert, so a wheel outside neither closes it nor scrolls. A
  popover that is not modal can do both, as the reviewer's probe showed: a full-window scrim
  whose `on_wheel` closes it and returns false lets Rune's fallback scroll the viewport under
  the point (`scroll_at_axis`, `runtime/input/pointer.rs`, which ignores what covers the point).
  GTK scrolls only when the wheel is over the listing; over the sidebar or other chrome it only
  closes the popover (`listing_ancestor` in `src/ui/scrolling/popover.rs`). So the scrim's
  `on_wheel` takes the wheel (returns true) away from the listing. The sort popover
  (`src/ui/browser/pane_header/sort_menu.rs:61`) is affected the same way.
- **Concerns:** the "Thumbnail size" popover ([A5](packages/A5.md)), and every popover GTK
  dismisses on an outside scroll, the sort popover among them.
- **State:** worked around for Strata's plain popovers by [A5](packages/A5.md)'s plain
  popover (`crates/strata-app/src/popover.rs`), on that scrim: it closes on an outside press or
  wheel and lets a wheel over the listing scroll it. The gap left is the modal's alone, Rune
  packet entry [104](../upstream/rune/104-outside-wheel-closes-a-popover.md), not to be filed.

### #2346: a view rebuilt during a staged publish can list rows twice

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2346), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **What was seen, by reading** (in the review of [#2343](https://github.com/lgse/strata/pull/2343)):
  - A view rebuilt from the browser while a large folder is still published in stages copies
    every entry the column holds, including those not published yet. That is Rune's
    `events::replay` (`with_column_entries`), and GTK's `rebuild_columns_from` and
    `apply_snapshot` read the same snapshot.
  - The `EntriesPublished` events still to come then insert their tails again.
  - No scenario or seed reaches it.
- **State:** open, set aside. The fix, for whoever meets it: a published-count accessor on
  `Browser`, so that a rebuild copies only what has been published. GTK's fix, if taken up,
  lands on `main`.

### #2350: GTK's QA snapshot reports a rename field's caret at the selection's fixed end

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2350), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in the Columns fuzz pilot ([F2](packages/F2.md)). After Shift+Home or Shift+Left
  in a rename editor, GTK's snapshot gives the caret at the selection's fixed end
  (`Editable::position()`, `read_field` in `src/qa/state.rs`); Rune's gives the end that
  moved. By behaviour the two editors agree: Shift+Right then shrinks the selection from its
  start on both, and a typed character replaces the same text. So only GTK's report is wrong.
- **Concerns:** seeds `0x08` and `0x0e` of `columns v1`, shrunk to 4 steps and parked in
  `tests/qa/fuzz/corpus/columns/`; any check of `field.caret` while a selection is held.
- **On `main`:** bug [#2361](https://github.com/lgse/strata/issues/2361), GTK's QA snapshot faults
  of #2350 and #2351 together; the coverage `columns` leaves out waits on it.
- **State:** open, set aside. The fix is `main`'s, in `src/qa/state.rs`, placed by the
  coordinator. Meanwhile the `columns` alphabet keeps Shift+Home, Shift+End and Shift+Left out of its
  rename editor, the reason it is narrower; they come back with the fix.

### #2351: after Ctrl+A in Columns, GTK reports the old row while its cursor is on the last

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2351), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in the Columns fuzz pilot ([F2](packages/F2.md)). After Ctrl+A, GTK draws its
  cursor on the column's last entry and the next arrow moves from there, as Rune does, but
  GTK's focused widget and its snapshot's `focused` stay on the row that had focus
  (`select_all` and `focus_surface`, `src/ui/browser/view_state.rs`). Checked by hand: the
  screen and the next key agree on both targets.
- **Concerns:** seeds `0x02`, `0x04`, `0x05`, `0x09` and `0x0c` of `columns v1`; `0x02`
  shrunk to 2 steps (Shift+click, Ctrl+A) and parked in `tests/qa/fuzz/corpus/columns/`.
- **On `main`:** bug [#2361](https://github.com/lgse/strata/issues/2361), GTK's QA snapshot faults
  of #2350 and #2351 together; the coverage `columns` leaves out waits on it.
- **State:** open, set aside, by the coordinator's ruling (2026-11-13): no Rune change. It is
  the reason the `columns` alphabet has no Ctrl+A; Ctrl+A comes back once GTK's report is fixed on
  `main`, or the owner rules otherwise.

### #2353: after a Shift+click in Columns, GTK's next Down starts from the old cursor

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2353), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in the Columns fuzz pilot ([F2](packages/F2.md)). After a Shift+click, GTK
  focuses the clicked row with no outline drawn, but its next Down, or Shift+Down, starts
  from the cursor the click left in place (`pointer_selection` with Shift,
  `src/ui/collection_interaction.rs`), so the user lands two rows lower than expected. Rune
  starts from the clicked row.
- **Concerns:** seed `0x0d` of `columns v1` (shrunk to 5 steps) and seed `0x09` of
  `columns v3`, parked in `tests/qa/fuzz/corpus/columns/`.
- **State:** open, waiting for the owner ([`owner.md`](owner.md), F2 item 1). Meanwhile Rune
  keeps the clicked row, and the seeds stay parked.

### #2354: Rune loses the Shift anchor across a refresh

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2354), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in the Columns fuzz pilot ([F2](packages/F2.md)). Down, Shift+Up, F5, Shift+Down:
  GTK selects `[documents]`, extending from the anchor the shared `Browser` keeps by
  location; Rune selects `[archive, documents]`, because `handover::adopt`
  (`crates/strata-app/src/handover.rs`) restores the browser's marks, which carry no
  anchor, with the cursor as the anchor. The same path serves List and Icons.
- **Concerns:** seed `0x14` of `columns v1`, shrunk to 4 steps and parked in
  `tests/qa/fuzz/corpus/columns/`.
- **State:** fixed by [F2](packages/F2.md)'s [#2363](https://github.com/lgse/strata/pull/2363), on `handover.rs` and C1. The marks a
  view adopts carry the browser's anchor, and `Selection::restore` keeps the anchor it is given
  while there is a cursor, where it replaced it with the cursor (`Option::and`), which also
  broke position restore's range anchor. A hosted test holds it in Columns, List and Icons
  (`commands::tests::refresh_anchor`), and the shrunk sequence is no longer parked: the corpus
  replay holds it to agreeing on both targets.

### #2355: Rune's right-click moves the Shift anchor, GTK's does not

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2355), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in the Columns fuzz pilot ([F2](packages/F2.md)). Right-click `documents`,
  Escape, Shift+click `documents`: GTK selects `[archive, documents]`, since its right-click
  (`focus_context_entry`, `src/ui/browser/context_menu/focus.rs`) never moves the anchor;
  Rune selects `[documents]`, since the collection component's secondary press selects with
  `select_only` (`crates/strata-components/src/collection/pointer/machine.rs`).
- **Concerns:** seeds `0x12` and `0x07` of `columns v1` (shrunk to 3 and 10 steps) and seed
  `0x0f` of `columns v3`, parked in `tests/qa/fuzz/corpus/columns/`.
- **State:** fixed by [F2](packages/F2.md)'s [#2371](https://github.com/lgse/strata/pull/2371) in C1e: a secondary press
  selects an unselected entry with `Selection::select_only_keeping_anchor`, so an anchor there
  is stays where it was in Columns, List and Icons, as GTK's shared `focus_context_entry`
  leaves it in all three (checked on GTK's List and Icons with the same three steps). With
  no anchor, as in a column just opened, the anchor goes to the right-clicked entry, as
  `strata-fs`'s `set_selection` takes the cursor for an anchor it lacks: click `pictures`,
  right-click `photo.txt`, Escape, Shift+click `diagram.txt` selects both on both targets.
  Held by C1e's secondary-press test (both cases) and by `qa::tests::right_click` (each view,
  and the no-anchor case in Columns); the sequences `v1-0x12`, `v1-0x07` and `v3-0x0f` are no
  longer parked: the corpus replay holds them to agreeing on both targets. The filter results
  (C18) take no anchor from GTK's right-click even when they have none: finding
  [#2375](https://github.com/lgse/strata/issues/2375), A10's.

### #2356: after a click on an open folder, GTK reports a focused row it neither draws nor uses

- **Code:** `oracle`, reclassified on 2026-11-13 from `divergence`. [Issue](https://github.com/lgse/strata/issues/2356), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in the Columns fuzz pilot ([F2](packages/F2.md)). With `pictures` selected and
  its column open, a click on it reveals the column (`activate_column_entry`,
  `src/ui/browser/columns/state.rs`), and GTK's keyboard focus goes to its first row's widget,
  which GTK's QA snapshot reports as `focused` (`diagram.txt`), in 24 GTK replays of 24. But a
  screenshot shows no cursor drawn in the column and nothing selected, and the next Down lands
  on that first entry and selects it, since GtkListView's own cursor is unset. Rune gives the
  column the keyboard with no cursor and reports no focused entry; its next Down lands on the
  first entry too. So the screen and the next key agree; only GTK's report differs, as in
  [#2351](https://github.com/lgse/strata/issues/2351).
- **Concerns:** seed `0x10` of `columns v1`, shrunk to 4 steps and parked in
  `tests/qa/fuzz/corpus/columns/`.
- **On `main`:** the third case of bug [#2361](https://github.com/lgse/strata/issues/2361), GTK's QA snapshot faults.
- **Also by the location bar** ([A5](packages/A5.md)'s `location` alphabet, 2026-11-13): with
  `pictures` selected and its column open, Ctrl+L and Return on the path as it stands, or a
  typed path to the folder shown, end the same way. GTK reports `diagram.txt` focused while its
  screenshot draws no cursor and its next Down lands on `diagram.txt`; Rune focuses the column
  with no entry. Shrunk to 3 steps, confirmed 3 of 3, the same in 24 of 24 GTK replays, and
  parked in `tests/qa/fuzz/corpus/location/` as `v1-000000000000000e`.
- **State:** open, set aside: no Rune change. A fix that gave Rune's column a cursor
  ([#2368](https://github.com/lgse/strata/pull/2368)) was closed unmerged: it made Rune's next
  Down move on where GTK's lands on the first entry, as a list with no cursor does ([#2367](https://github.com/lgse/strata/issues/2367)).

### #2357: Up, Down or a page key in the rename editor commits on GTK, not on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2357), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in the Columns fuzz pilot ([F2](packages/F2.md)). On GTK nothing in the rename
  field handles Up, Down, Page_Up or Page_Down: the entry's own key navigation moves focus out
  of the field, and the field commits the rename on focus-out (`connect_leave`,
  `src/ui/collection_edit.rs:235`). So a vertical key renames the file by a toolkit side effect,
  and the cursor lands where the key took focus. On Rune the editor keeps the keyboard and
  moves its caret, and nothing is renamed. The field that parts first is `app.field`: null on
  GTK, the open editor on Rune. Page_Up does the same as the others on GTK (a probe: `x` typed
  over `readme.md`, then Page_Up, renames it `x.md` and focuses `archive`).
- **Concerns:** seeds `0x02`, `0x0c`, `0x12` and `0x14` of `columns v2`; `0x12` shrunk to
  3 steps (F2, `x` typed, Down), confirmed 3 of 3 and the same in 24 of 24 GTK replays, and
  parked in `tests/qa/fuzz/corpus/columns/`. The calibration survey met it too.
- **State:** open, waiting for the owner ([`owner.md`](owner.md), F2 item 2), by the
  coordinator's ruling. Meanwhile Rune keeps editing, and `columns v3` keeps Up, Down and the
  page keys out of its rename editor, the reason it is narrower.

### #2358: Rune's entry menu shows Open on a selection of more than one entry

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2358), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in the Columns fuzz pilot ([F2](packages/F2.md)). With more than one entry
  selected, GTK's entry menu has no "Open" row and Rune's has one, between "New Folder with
  Selection" and "Open With…": seen on folders and files mixed, and on two files. The
  calibration survey met it too.
- **Concerns:** seed `0x10` of `columns v2`, shrunk to 2 steps (Shift+End over the root, then a
  right-click on `documents`), confirmed 3 of 3 and the same in 24 of 24 GTK replays; seeds
  `0x10`, `0x11` and `0x14` of `columns v3`. All parked in `tests/qa/fuzz/corpus/columns/`.
- **State:** open, [A6](packages/A6.md)'s (menus), noted in its file.

### #2365: a location suggestion is a named list item on Rune, where GTK's row holds a label

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2365), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A5](packages/A5.md)'s path suggestions pull request. GTK's suggestion row
  holds a `label` named after the folder (`documents/`); C6's row is a `list item` named so,
  with its parent as the description, and the text inside is not in the tree. A screen reader
  hears the same name, in another shape.
- **Concerns:** `locations/path-completion-dismisses-on-window-switch/columns`, whose line 6
  waits for the `label` (it is pending on [A21](packages/A21.md)'s second window as well); C6's
  suggestion list (`crates/strata-components/src/location_bar/popup.rs`).
- **State:** ruled by the coordinator (2026-11-13): C6 stays as it is, since a screen reader
  hears the same name; the run takes an `a11y` twin once it can otherwise flip, after
  [A21](packages/A21.md)'s second window. [A5](packages/A5.md)'s, or whoever flips the run.

### #2367: withdrawn, an artifact of a fix that did not merge

- **Code:** `divergence`, withdrawn. [Issue](https://github.com/lgse/strata/issues/2367), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [F2](packages/F2.md), on a probe of [#2368](https://github.com/lgse/strata/pull/2368): after a click on an open folder,
  GTK's first Down selects the column's first entry in place, where Rune with #2368 moved on.
  #2368 had given Rune's column a real cursor; GTK's has none (see #2356), so its first Down
  lands on the first entry, as Rune without #2368 does. With a cursor and nothing selected by
  Escape, Down moves on in Columns, List and Icons on both targets.
- **State:** withdrawn: #2368 was closed unmerged, and there is nothing left to fix.

### #2375: a right-click in filter results with no anchor anchors there on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2375), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen, by reading** (in the review of [#2371](https://github.com/lgse/strata/pull/2371)): GTK's
  search and filter results keep an anchor of their own (`interactions.anchor`,
  `src/ui/inline_search/collection.rs`), which a right-click (`focus_search_result`, then
  `focus_result`) never sets; with none, the next Shift+click selects the clicked result alone.
  C1e's secondary press, since #2371, anchors the right-clicked entry when there is no anchor,
  as the browser's views do; C18's results are a C1e collection, so a Shift+click after it
  would range from the right-clicked result.
- **Driven on 2026-11-13** (A10, both targets; filter `a` over `a1.txt` to `a4.txt` and
  `b.md`, right-click `a1.txt`, Escape, Shift+click `a3.txt`):
  - In Icons and List, GTK selects `a3.txt` alone; Rune, whose pane results are a C1e
    collection since A10's Icons and List pull request, selects `a1.txt` to `a3.txt`. The
    divergence is confirmed there.
  - In a filtered Columns column both select `a1.txt` to `a3.txt`: GTK's Columns branch does
    anchor the right-clicked result. No divergence there.
- **Concerns:** the results of the filter in Icons and List; no scenario steps through it.
- **State:** fixed, by [A10](packages/A10.md)'s `contract` pull request [#2428](https://github.com/lgse/strata/pull/2428): C1e's
  `PointerSettings::secondary_keeps_no_anchor`, off by default, leaves a missing anchor
  missing on a secondary press, and the pane's results turn it on
  (`single::tests::filter::a_right_click_leaves_the_*_without_an_anchor`).

### #2379: Shift+Tab from a Columns column stops on its header's actions on GTK, on the sidebar on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2379), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A5](packages/A5.md)'s `location` alphabet, where 8 of `location v1`'s 20 smoke
  seeds first parted at it. GTK's column header actions are Tab stops, so Shift+Tab from the
  entries focuses the targeted column's last one, the "Filter this pane (Ctrl+F)" toggle
  (A10's); its QA snapshot then names the column `focused_pane` with no entry. C2's slot keeps
  the header's actions off the focus chain (`focusable(false)`, as its Close), so Rune's
  Shift+Tab goes on to the sidebar's Home.
- **Concerns:** `location v1`'s seed `0x2`, shrunk to 1 step, confirmed 3 of 3, the same in
  24 of 24 GTK replays, and parked in `tests/qa/fuzz/corpus/location/`; `location v2` keeps
  Shift+Tab off the files. C2's header slot (`crates/strata-components/src/columns/column.rs`)
  and `pane_header::actions` in `strata-app`.
- **State:** open, waiting for the owner ([`owner.md`](owner.md#a5-location-bar), A5 item 2),
  as [#2331](https://github.com/lgse/strata/issues/2331) is for the List headings: whether a
  column header's actions become Tab stops on Rune. Meanwhile they stay off the focus chain.

### #2380: after a typed path fails to open, GTK's next Ctrl+L shows that path, Rune's the location

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2380), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A5](packages/A5.md)'s `location` alphabet. GTK's `submit_location`
  (`src/ui/browser/location.rs`) restores the entry's text only when a path fails validation;
  a path that validates and then cannot be opened ("Unable to open location") stays in the
  entry, and `begin_location_edit` reopens on it, all selected, as GTK's screenshot shows.
  Rune's `begin` (`crates/strata-app/src/location/mod.rs`) opens on the trail's text, the shown
  location's path.
- **Concerns:** `location v1`'s seed `0x7`, shrunk to 5 steps, confirmed 3 of 3, the same in
  24 of 24 GTK replays; with `location v2`'s seeds `0x7` and `0xd`, in
  `tests/qa/fuzz/corpus/location/`.
- **State:** fixed by [A5](packages/A5.md)'s [#2383](https://github.com/lgse/strata/pull/2383): the bar keeps the text Enter opened
  (`Bar::typed`, `crates/strata-app/src/location/`) for the next Ctrl+L, and forgets it as
  GTK's entry does: when the columns are truncated, added to or relocated, when the view
  switches, and when an edit is cancelled; a location refused at once is not kept. A hosted
  test, `location::tests::a_location_that_could_not_be_opened_is_the_next_edits_text`, holds
  each. The three sequences are unparked and replay as kept sequences.

### #2381: after an outside click closes "Unable to open location", GTK's keyboard is on the first crumb

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2381), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A5](packages/A5.md)'s `location` alphabet. A click outside GTK's error dialog
  only closes it, and keyboard focus falls to the header's first crumb, "/", since the
  location entry that held it is hidden; the next Down moves along the header. Rune's dialog
  closes and the clicked column takes the keyboard; the next Down lands on its first entry.
  Closed by Escape, both return the keyboard to the files.
- **Concerns:** `location v1`'s seed `0x13`, shrunk to 5 steps, confirmed 3 of 3, the same in
  24 of 24 GTK replays, and parked in `tests/qa/fuzz/corpus/location/`; `location v2` makes
  no click while a dialog shows.
- **State:** open, waiting for the owner ([`owner.md`](owner.md#a5-location-bar), A5 item 3):
  GTK's keyboard lands where nothing put it, and whether Rune copies that is not the
  alphabet's to decide.

### #2376: a background click in a scrolled inactive column selects its first visible entry on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2376), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by the reviewer of [#2374](https://github.com/lgse/strata/pull/2374), by reading. A
  plain click on the empty space of a column that isn't active makes it active and selects an
  entry in it: GTK's `clear_marquee_selection` (`src/ui/browser/columns/append/surface.rs`)
  selects the column's first entry, C2's `background_click`
  (`crates/strata-components/src/columns/view.rs`) its first entry in view. They differ only in
  a scrolled column.
- **Concerns:** no scenario yet; the scenarios that cover the press start at the top.
- **State:** open, [A8](packages/A8.md)'s.

### #2377: a drag from a column's background clears only that column's selection on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2377), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by the reviewer of [#2374](https://github.com/lgse/strata/pull/2374), by reading. On
  GTK the marquee's press calls `clear_marquee_selection`, which clears every column's selection
  and closes the deeper columns before the band moves. On Rune a press that becomes a band goes
  to the marquee (`crates/strata-components/src/columns/pointer.rs`), whose `begin`
  (`crates/strata-components/src/collection/marquee/machine.rs`) starts the band in its own
  column's selection only, so the other columns keep theirs.
- **Concerns:** no scenario yet drags from a column's background with entries selected in
  another column.
- **State:** open, [A8](packages/A8.md)'s.

### #2392: rows and buttons under an open plain popover still show hover

- **Code:** `rune-gap`. [Issue](https://github.com/lgse/strata/issues/2392), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here. The
  issue's text, written before GTK was driven, says nothing outside GTK's popover hovers; this
  entry is the corrected record.
- **Seen:** by the owner, in List with the Appearance popover open: a List row beside and under
  the popover lit up as the pointer moved over the popover's rows. A hosted probe in
  `strata-app` reproduced it: with the pointer over the popover's "Hidden files" row, both that
  row and the List row under it show hover 1.0, and 60 pt beside the popover, over the scrim,
  the List row does. GTK, driven
  (`./scripts/qa-drive.sh`, XTest motion on Xvfb, the List view, 2026-11-13): with the pointer
  over the popover's rows, nothing under the popover hovers; outside it, the sidebar's Home and
  the header's Settings button hover; a List row beside it hovered in one of three sessions
  (one where the pointer had been over the listing before the popover opened) and not in the
  other two. So GTK's gap with Rune is under the surface for certain, and beside it at most
  in the listing.
- **Why:** Rune paints hover on every element whose rect holds the pointer (`hover_flip` in
  `runtime/input/hover.rs` at `2a44156`); only what is not `allowed` (inert, or outside a modal)
  and the ancestors of a `hover_alone` element are spared, and nothing above an element can stop
  it. The plain popover is not modal on purpose (#2341), so everything under its scrim hovers:
  the listing's rows, and by the same code the sidebar's and the header's.
- **Concerns:** `crates/strata-app/src/popover.rs` and its three users: the Appearance popover,
  the Icons header's "Thumbnail size", and "Customize columns…". Rune packet entry
  [105](../upstream/rune/105-hover-paints-through-a-cover.md).
- **Workarounds weighed** (2026-11-13), none built:
  1. The window's content `.inert()` while a plain popover is open: one change in `Plain`, and
     hover, cursor and tooltips stop beneath, as GTK's grab does. It costs the driven "a wheel
     over the listing closes it and scrolls the listing": `scroll_at_axis` skips what is not
     `allowed`, and `Tx` has no relative scroll request, so the first wheel would only close
     it (`popover::tests` covers that wheel).
  2. The same, with the scrim scrolling the listing itself by `Tx::scroll_to_y(listing, offset
     + dy)`: exact, but each view would have to give its listing's live offset through a new
     `slots::View` accessor, Columns one per column: a `contract` change to the slots and three
     views.
  3. The window's content built with its hover tokens transparent while a popover is open, the
     popover on the real look: hover is baked into each element when it is built, so it works,
     but it rests on every kit hover token being swapped, and rune-kit's controls pick their
     own.
- **State:** left as it is, by the owner's ruling of 2026-11-13 (to the coordinator), until
  Rune can block hover under a cover (packet 105) or let a modal background route a wheel
  (packet 104). [A8](packages/A8.md)'s, noted in `popover.rs`'s module docs.

### #2386: a column that is not active loses its load-cursor (provisional) mark in the Rune view

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2386), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [F3](packages/F3.md)'s `browser-view` invariant, on Rune. The browser keeps a
  load cursor for each column, but `Browser::selection_is_load_cursor` reports the active
  column's only, and `events::marks` (`crates/strata-app/src/events.rs`) reads it for every
  column. So when the browser reports a column's marks again while another is active (a file
  written into its folder, hidden files toggled), `handover::adopt` clears the column's
  provisional flag, and the view takes the browser's load cursor for a fill. This is the
  load cursor, the selection a load made on its own, not a column's loading state. Seed `0x07` of
  `calibration v3` shows it in three steps: launch, Return on `archive`, a file written into
  the root.
- **What a user sees:** nothing found. Delete, rename and open act on the browser's
  selection, which is right. The view's flag decides a 10xer verb's targets
  (`Selection::targets`), and Rune has no 10xer yet. `Selection::move_cursor` reads it too,
  for the Ctrl+Left and Ctrl+Right nudge and the 10xer moves, but a move happens in the
  column that has the keyboard, which is the active one, and so never in a column the flag
  was lost in. The conclusion stands.
- **Concerns:** seed `0x03` of `columns v3` and seed `0x07` of `calibration v3`, parked
  until the fix.
- **State:** fixed by [F3](packages/F3.md)'s [#2398](https://github.com/lgse/strata/pull/2398). `events::marks` reads each
  column's own load cursor: a selection the browser does not count as a fill
  (`Browser::filled_entries`). A hosted test holds it,
  `qa::tests::invariants::a_column_keeps_its_load_cursor_when_its_folder_changes_while_another_is_active`,
  and fails without the fix. `columns v3` `0x03` is unparked, and the corpus replay holds it to
  agreeing on both targets. `calibration v3` `0x07` is taken out of the corpus: it agrees on
  Rune, but the `calibration` alphabet does not agree across targets (type to search is on),
  so a differential replay could not hold it.

### #2387: the browser's Shift anchor outlives its entry and comes back with it

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2387), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [F3](packages/F3.md)'s `browser-view` invariant, on Rune. The browser keeps the
  Shift anchor by location, and nothing clears it when its entry leaves the listing; when the
  entry comes back (Undo of a Move to Trash, Redo of a new folder) the anchor is on it again.
  The view moved its anchor to the cursor when the entry left, as GTK's does, so the two
  disagree until a reload makes the view take the browser's anchor.
- **Run on both targets:** click `todo.txt`, Delete, Ctrl+Z, F5, Shift+Up. GTK and Rune both
  select `pictures`, `readme.md` and `todo.txt`, a range from the entry anchored before it
  was deleted; without F5, both select `pictures` and `readme.md`. So the fault is the shared
  browser's, and Rune agrees with GTK on what the user sees.
- **Concerns:** seeds `0x04` and `0x05` of `entries v1`, and `0x01`, `0x0a`, `0x0b`, `0x0d`,
  `0x0f` and `0x10` of `entries v3`, whose Undo with its Redo straight after meets it often,
  parked in `tests/qa/fuzz/corpus/entries/`.
- **State:** open, set aside, its sequences parked. The fault is `strata-fs`'s, in the
  browser both interfaces share, so GTK has it too: its fix belongs on `main`, from a bug
  issue there, and reaches `strata-next` by merge. The coordinator places it.

### #2397: the "Column width" caption can't be read on the Rune interface

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2397), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by the owner, in Columns in a dark theme: resting on or dragging a column's edge
  showed the caption as a light box whose text could not be seen. C9's divider caption, over
  the preview drawer's edge and inside its minimum outline, had the same fault. GTK draws both
  as `.resize-caption` (`src/style.css`): the accent, written in the background
  (`@strata_accent`, `@strata_bg`), with no border.
- **Why:** both captions filled with Kit's `surface_float` and wrote in `text_base`. rune-kit's
  `surface_float` is the palette's foreground, an inverted tooltip surface that its own
  tooltip pairs with inverse text, and strata-kit leaves it at `from_palette`'s value, so
  Strata's text sat on a fill of nearly its own colour, in light themes as in dark.
- **Concerns:** `crates/strata-components/src/columns/column.rs` and
  `crates/strata-components/src/preview_drawer/divider.rs`; no scenario reads colours.
- **State:** fixed by [#2400](https://github.com/lgse/strata/pull/2400): both captions take
  `Look::resize_caption`, Kit's `bg_accent` and `text_inverse` (Strata's accent and
  background), and draw no border, as GTK does; `columns.caption.border` now defaults to 0.

### #2403: a long file name widens the context menu on the Rune interface

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2403), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by the owner: a right-click on a file with a long name opened a menu as wide as the
  name. GTK's item menu shows the name and the compact path as header labels shortened in the
  middle past 30 characters (`ITEM_CONTEXT_SUMMARY_MAX_CHARS`, `EllipsizeMode::Middle`, in
  `src/ui/browser/context_menu/item_options.rs`), so its menu keeps its usual width; the
  labels keep their whole text for assistive technology.
- **Why:** C21 draws its header as rune-kit group labels, whose text never shrinks or
  ellipsizes, and the menu surface takes only a least width, so a header line as wide as the
  name widened it. The rows were not at fault: their labels already end in an ellipsis.
- **Concerns:** `crates/strata-components/src/context_menu/view.rs`, so every context menu with
  a header: the item and multi-selection menus. No scenario names a file that long.
- **State:** fixed by [#2404](https://github.com/lgse/strata/pull/2404): C21 shortens each header
  line in the middle at GTK's 30 characters, and when it shortened one, the menu's accessible
  description gives the header whole. rune-kit's group labels take no truncation, so the cut
  is made on the string before it reaches one.

### #2401: a `strata-app` undo test failed once under load

- **Code:** none of the plan's: a flaky Rune-side unit test, labelled `testing`.
  [Issue](https://github.com/lgse/strata/issues/2401), filed on 2026-11-13 as the finding's
  reference and closed at once: its state is kept here.
- **Seen:** in a full `mise run test-rune` for [A8](packages/A8.md)'s "Show sizes in Columns
  view" gate, while a `./scripts/qa-parity.sh all` build loaded the machine:
  `undo::tests::a_rename_is_undone_by_its_key_and_redone_by_either_of_redos` said
  "undo-target still absent" after its 30 s wait (`crates/strata-app/src/undo/tests.rs:59`).
  The branch did not touch undo. Alone, the test passed 3 of 3.
- **Concerns:** that test, [A7](packages/A7.md)'s undo.
- **What it suggests:** a hypothesis, by reading only, not verified: the test presses the redo
  chord once the undone name is back on disk, and the operation's completion may not have
  reached the undo stack by then, so the redo finds nothing to redo.
- **State:** placed by the coordinator, 2026-11-13: unowned flake beside #2322; if it recurs
  outside load, it goes to whoever next works in undo.

### #2405: an open plain popover is a named dialog on Rune and an unnamed panel on GTK

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2405), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A8](packages/A8.md)'s `views` alphabet's first smoke tier (v1, never merged):
  14 of 20 sequences parted at the click that opens the Appearance popover, on `app.dialog`,
  `null` on GTK and "Appearance" on Rune.
- **Why:** GTK's `GtkPopover` has the accessible role `generic` and no name: the GTK QA tree,
  driven (`./scripts/qa-drive.sh`, 2026-11-13), reports the open Appearance popover as
  `GtkPopover role="panel"` with `GtkPopoverContent role="panel"` inside, both unnamed, from
  `widget.accessible_role()` (`role_name` in `src/qa/targets/naming.rs` maps `Generic` to
  `panel`). That is what GTK exposes to AT-SPI too. Rune's `popover::Plain` gives the surface
  the role `dialog` and names it (rune-kit's popover does the same), and strata-qa's
  `open_dialog` names the first rendered `dialog` in `app.dialog`. So a screen reader hears
  "Appearance, dialog" on Rune and an unnamed panel on GTK: not a QA artifact only.
- **Concerns:** every plain popover: the Appearance popover, the Icons header's "Thumbnail
  size" ([A5](packages/A5.md)), and "Customize columns…"'s menu is C4's and not one. The
  `views` alphabet keeps its popover actions; its smoke tier parts there until this is ruled.
- **Options for the owner:**
  1. Rune keeps the named `dialog`, the more useful role for assistive technology, and the
     normalisation map gains a rule for `app.dialog` while a plain popover is open, which
     needs the owner's ruling as every map rule does.
  2. `Plain` drops to an unnamed generic group, matching GTK's AT-SPI exactly, and assistive
     technology loses the popover's name and role.
  3. The QA backend leaves plain popovers out of `app.dialog`. Wrong if option 1's role
     stands, since it would hide a real difference from the comparison rather than rule it.
- **State:** ruled by the owner, 2026-11-13 ([`owner.md`](owner.md#ruled)): option 1. Rune
  keeps the named `dialog`, a deliberate difference, and `tests/qa/fuzz/normalisation.map`
  excuses `app.dialog` only for GTK's `null` against a plain popover's name ("Appearance",
  "Thumbnail size"): a rule that names its values, which the map takes since this finding.
  Noted in `popover.rs`'s module docs. [A8](packages/A8.md)'s.

### #2408: Down in an empty Icons view takes GTK's keyboard to the footer

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2408), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A8](packages/A8.md)'s `views` alphabet (v2, never merged), shrunk 3 of 3
  GTK against Rune from `fuzz-views-v2-000000000000000c`. On the canonical fixture, type to
  search off: Right on `archive` in Columns, the fixture's one empty folder, Ctrl+2, Down.
  `app.focused_pane` is then `null` on GTK and `"archive"` on Rune.
- **Not the snapshot:** GTK driven (`./scripts/qa-drive.sh`, 2026-11-13), by F2's lesson
  (#2350, #2351, #2356, `main` bug #2361): after the Down, GTK's widget tree's focus line and
  the focus ring on screen are on the footer's `GtkToggleButton "F1 Shortcuts"`. Ctrl+A leaves
  it there; Alt+Up brings it back to the parent's `archive`. Rune's keyboard stays in the
  pane. Whether GTK's move is what it should do (an empty grid handing Down to the next widget
  below) is the area's question.
- **Concerns:** the Icons view's keys in an empty folder ([A1](packages/A1.md)'s area). The
  sequence is parked in `tests/qa/fuzz/corpus/views` (`v2-000000000000000c`), and the `views`
  alphabet from v3 keeps Right off `archive`.
- **State:** open, with the owner ([`owner.md`](owner.md#a1-icons-view-and-thumbnails), A1
  item 11), placed by the coordinator on 2026-11-13: GTK's move looks like its directional
  focus falling through, so Rune does not copy it meanwhile, and the sequence stays parked.

### #2406: hiding the sidebar with the keyboard in it leaves no element with the keyboard on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2406), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [F3](packages/F3.md)'s `keyboard` invariant, in the sidebar alphabet on Rune.
  Ctrl+B or the header's toggle hid the sidebar while one of its rows had the keyboard. The
  key of an element no longer drawn kept it, and every key after that (Menu, the arrows)
  went nowhere until a click. On GTK, in the same replay (seed `0x01` of `sidebar v1`,
  `gtk,rune`), the next Menu key opens the selected entry's context menu.
- **Also seen:** two more cases leave no element with the keyboard, both checked against
  GTK in the same `gtk,rune` replays.
  - Seed `0x10`: a drag of a sidebar row while that row's menu is open. GTK's menu swallows
    the drag and stays open; Rune's closes, and the keyboard goes nowhere. This is a
    divergence of its own (a drag across an open menu), and not as local as the hidden
    sidebar.
  - Seed `0x0e`: Unpin of the focused row. Driven on GTK (`qa-drive.sh`, 2026-11-14): Left
    into the sidebar, the Menu key, Unpin on `Home`. GTK draws the keyboard's ring on the next
    row, `Trash`; Down then moves it to `Network`, and the Menu key opens `Network`'s menu.
    On Rune no element has the keyboard. So the divergence stands: GTK gives the keyboard to
    the row after the one that went.
- **State:** the hidden sidebar fixed by [F3](packages/F3.md)'s
  [#2409](https://github.com/lgse/strata/pull/2409): the toggle gives the shown view the keyboard when it hides the sidebar with the
  keyboard in it (`sidebar/width.rs`), held by
  `sidebar::tests::hiding_the_sidebar_with_the_keyboard_in_it_gives_the_keyboard_to_the_view`.
  The drag and Unpin cases are open, their seeds parked in `tests/qa/fuzz/corpus/sidebar/`
  by F3's [#2432](https://github.com/lgse/strata/pull/2432).

### #2407: the location bar's hierarchy menu holds no keyboard on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2407), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [F3](packages/F3.md)'s `keyboard` invariant, in the sidebar alphabet on Rune,
  after the Menu key with the keyboard on the location bar's crumbs. The hierarchy menu
  (C6's `location-menu-menu`) was open, drawn and had the keyboard, but the location part's
  `keyboard` answered nothing for it, so the window's dispatcher did not treat it as the
  owner of every key, as GTK's `input_owner` treats a menu. No key was seen to leak: Rune's
  menu takes the keys it is sent. The QA snapshot's `menu` reads `[]` for it, since its rows
  are check menu items.
- **State:** fixed by [F3](packages/F3.md)'s [#2409](https://github.com/lgse/strata/pull/2409): the part holds
  `Holds::Menu` while the menu is open, held by
  `location::tests::the_hierarchy_menu_holds_the_keyboard_while_it_is_open`.

### #2410: after "Group by file type", GTK's keyboard is on the Appearance button, Rune's in the List

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2410), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A8](packages/A8.md)'s `views` alphabet (v3, never merged), shrunk 3 of 3 GTK
  against Rune from `fuzz-views-v3-0000000000000006`, and first in 7 of its 20 smoke seeds:
  Ctrl+3, a click on the header's Appearance button, a click on "Group by file type". GTK's
  popover closes and leaves the keyboard on the Appearance button, as Escape does; Rune's
  regroup rebuilt the List view, which took the keyboard. `app.focused` is `null` on GTK and
  the cursor's entry on Rune.
- **Not the snapshot:** GTK driven (`./scripts/qa-drive.sh`, 2026-11-13): its widget tree's
  focus line and the focus ring on screen are on the Appearance button after the click.
- **State:** fixed by [A8](packages/A8.md): the Group row gives the button the keyboard after
  the regroup (`appearance/popover.rs`), held by
  `appearance::tests::popover::group_by_file_type_regroups_the_list_and_hidden_files_shows_them`.
  The sequence joins the `views` corpus unparked with the alphabet's pull request.

### #2411: Escape on a header button clears the selection on GTK, not on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2411), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A8](packages/A8.md)'s `views` alphabet (v3, never merged), shrunk 3 of 3 GTK
  against Rune from `fuzz-views-v3-000000000000000b`, and met by seed `0x14` too: End, Up, a
  click on the header's Appearance button, Escape (the popover closes, and the keyboard goes
  back to the button on both), Escape. GTK's selection is then `[]`, Rune's `[readme.md]`.
- **Why:** GTK's window-level `dismissal` stage (`src/ui/window/keyboard/items.rs`) closes the
  preview or clears the active column's selection on an Escape from anywhere outside text.
  Rune clears a selection on Escape only through the view's own keys, while the view has the
  keyboard; nothing answers Escape at the `Dismissal` stage.
- **Not the snapshot:** GTK driven (`./scripts/qa-drive.sh`, 2026-11-13): after the second
  Escape nothing is drawn selected, the focus ring stays on the Appearance button, and Return
  there opens the popover again.
- **Concerns:** the window's keys, [A6](packages/A6.md)'s (keymap), noted in its file. The
  sequence is parked in `tests/qa/fuzz/corpus/views/`, and the `views` alphabet from v4 sends
  Escape only while a pane has the keyboard.
- **State:** open, [A6](packages/A6.md)'s.

### #2412: after hidden files are toggled, GTK's List extends a Shift range from its first row

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2412), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A8](packages/A8.md)'s `views` alphabet (v3, never merged), shrunk 3 of 3 GTK
  against Rune from `fuzz-views-v3-0000000000000005`, and met by seed `0x02` too. In Columns:
  Down, Ctrl+period, Down, Down (onto `.hidden.txt`), Ctrl+period, Ctrl+period, Down, Up (onto
  `readme.md`); then the Appearance popover's List row, and Shift+Up. GTK selects `archive`,
  `documents`, `pictures` and `.hidden.txt`, a range from the folder's first row, which the
  user never anchored; Rune selects `.hidden.txt` and `readme.md`. Without the switch to List,
  GTK's Shift+Up in Columns selects `.hidden.txt` and `readme.md` too.
- **Not the snapshot:** GTK driven (`./scripts/qa-drive.sh`, 2026-11-13): the four rows are
  drawn selected, and the next Shift+Down extends the same range from `archive` to
  `readme.md`.
- **Also without hidden files** (`views v4`, seed `0x04`, shrunk to 7 steps): in Columns,
  Shift+Down from `archive`, Down, Down, Shift+Up, Up (onto `documents`), Ctrl+3, Shift+Down.
  GTK selects `archive` to `pictures`, extending from where the first Shift range started;
  Rune selects `documents` and `pictures`. So GTK's List, switched to, can extend from an
  anchor plain arrows in Columns had already moved.
- **Or from no anchor** (`views v4`, seed `0x02`, shrunk to 7 steps): in Columns, Down, Down,
  Up, Down, Shift+Up (`documents` and `pictures`, anchored on `pictures`), Ctrl+3, Shift+Up.
  GTK selects `archive` alone, as if the range had no anchor; Rune selects `archive` to
  `pictures`.
- **Concerns:** GTK's List view's Shift anchor after a view switch. Rune's views take the
  browser's anchor ([#2354](https://github.com/lgse/strata/issues/2354)), and agree with GTK's
  Columns. The sequences are parked in `tests/qa/fuzz/corpus/views/`.
- **State:** open, set aside: no Rune change. A fix is `main`'s, from a bug issue there.

### #2414: two Downs at the Icons grid's edge make GTK open the selected folder in Columns

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2414), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A8](packages/A8.md)'s `views` alphabet (v3, never merged), shrunk 3 of 3 GTK
  against Rune from `fuzz-views-v3-000000000000000c`, and met by seed `0x13` too. On the
  canonical fixture every tile is on the grid's one row: Ctrl+2, Down, Down, then the
  Appearance popover's Columns row. GTK opens `archive` as the child column (`app.matches`
  `[]`); Rune shows the root alone.
- **Why:** each arrow GTK's Icons view takes commits the selection (`commit_selection` in
  `src/ui/window/keyboard/items.rs`), even at the grid's edge where nothing moves. After two,
  the cursor the folder's load placed counts as the user's choice, and the switch back to
  Columns opens the lone selected folder (`follow_selection_on_return`, in the browser both
  interfaces share). Rune's Icons grid ignores a press at its edge (`owner.md`, A1 item 1), so
  the load cursor stays one and nothing opens. Driven on GTK (`./scripts/qa-drive.sh`,
  2026-11-13): nothing moves on screen at either Down; with one Down, or none, Columns opens
  nothing.
- **In List too** (`views v4`, seed `0x0e`, shrunk to 9 steps): in a folder grouped by file
  type with its cursor on the last row and nothing selected, GTK's Down selects that row, as
  its arrows first select the cursor's entry when nothing is (`resume_native_selection`) even
  where they then cannot move; Rune's does nothing at the edge.
- **Concerns:** a single pane's keys at its edge ([A1](packages/A1.md)'s area, noted in its
  file; List's is [A2](packages/A2.md)'s, closed). The sequences are parked in
  `tests/qa/fuzz/corpus/views/`, and the `views` alphabet from v4 sends no Up or Down in
  Icons.
- **State:** open, with the owner ([`owner.md`](owner.md#a1-icons-view-and-thumbnails), A1
  item 12), placed by the coordinator on 2026-11-13. Meanwhile Rune's grid ignores the press.

### #2415: GTK draws no cursor after a view switch or an open in Icons, and its next Down lands on the first entry

- **Code:** `oracle`, by the coordinator's ruling of 2026-11-13, which the owner may reopen
  ([`owner.md`](owner.md#a8-view-switching-and-appearance), A8 item 2).
  [Issue](https://github.com/lgse/strata/issues/2415), filed on 2026-11-13 as the finding's
  reference and closed at once: its state is kept here.
- **Seen:** by [A8](packages/A8.md)'s `views` alphabet (v3, never merged), shrunk 3 of 3 GTK
  against Rune:
  - `fuzz-views-v3-0000000000000009`: Ctrl+3, a double click on `documents`, Ctrl+1, Down.
    `app.focused` is `notes.txt` on GTK and `report.md` on Rune.
  - `fuzz-views-v3-0000000000000011`: the Appearance popover's Icons row, a double click on
    `documents`, Down. The selection is `[notes.txt]` on GTK and `[]` on Rune.
- **Driven on GTK** (`./scripts/qa-drive.sh`, 2026-11-13): in List, GTK draws its cursor on
  `notes.txt`, and Down moves to `report.md`. Shown in Columns by Ctrl+1, or opened in Icons,
  it draws none, though its snapshot still names `notes.txt` (`main` bug
  [#2361](https://github.com/lgse/strata/issues/2361), as in
  [#2356](https://github.com/lgse/strata/issues/2356)), and its next Down lands on `notes.txt`
  and selects it. Rune keeps the cursor on `notes.txt` in both, so its Down moves on from it:
  to `report.md` in Columns, and nowhere in Icons, where the grid's one row is its edge.
- **Concerns:** GTK's keyboard cursor after a view switch and an open in Icons. The sequences
  are parked in `tests/qa/fuzz/corpus/views/`, with seed `0x09` of `views v4`, the same four
  steps.
- **State:** open, set aside: no Rune change, by the coordinator's ruling, as with
  [#2367](https://github.com/lgse/strata/issues/2367). GTK drops the cursor its List view
  showed, a fault for `main`, from a bug issue there.

### #2425: a view shown again restores a folder position it remembered stale

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2425), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A8](packages/A8.md)'s `views` alphabet, v4, shrunk 3 of 3 GTK against Rune:
  - `fuzz-views-v4-0000000000000001`: the Appearance popover's List row, Ctrl+2, a double
    click on `documents`, Ctrl+3, Alt+Up. GTK selects `documents`; Rune's List shows `archive`
    selected as a load cursor while the browser holds `documents`, which F3's `browser-view`
    invariant reports.
  - `fuzz-views-v4-0000000000000011`: End, Ctrl+3, the popover's Icons row, a double click on
    `documents`, the popover's List row, Alt+Left. GTK focuses `documents`, Rune `todo.txt`.
  - Seeds `0x13` and `0x14` part the same way at Alt+Left in Icons.
- **Why:** GTK keeps one history of where each folder was left, whatever the view
  (`PaneNavigation`). Rune's Icons and List views each keep a memory of their own in their
  `Pane` (`pane/restore.rs`, [A1](packages/A1.md)'s note in this package's file). A view
  switched away hears nothing, and when it is shown again the rebuild's `Reset` has it
  remember the folder it showed last as it last saw it, so Back or Up into that folder brings
  the stale position back over the one the user left.
- **Concerns:** the view switch ([A8](packages/A8.md)'s). The sequences are parked in
  `tests/qa/fuzz/corpus/views/`.
- **State:** open, A8's: a fix is one memory for the window's single-pane views, kept by the
  shell, and no remembering on a rebuild, in a pull request of its own.

### #2426: Escape with nothing selected closes the deepest column on GTK

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2426), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A8](packages/A8.md)'s `views` alphabet, v4, shrunk 3 of 3 GTK against Rune from
  `fuzz-views-v4-000000000000000f`: in Columns, Home (`archive` chosen, its column open),
  Escape, Ctrl+2, Escape, a double click on `documents`, Alt+Left. GTK goes back to the root,
  Rune to `archive`.
- **Why, by reading, not driven:** with nothing selected, GTK's Escape in the files goes on to the
  browser's `escape()` (`directory_enter_or_escape` in `src/ui/window/keyboard/items.rs`),
  which closes the peek, clears the selection, or else closes the deepest column, here the
  `archive` column the Icons view does not show. Rune's view keys clear a selection and stop,
  so the column stays, and the folder opened in place next records it in the history.
- **Concerns:** the Escape stages, [A6](packages/A6.md)'s with
  [#2411](https://github.com/lgse/strata/issues/2411), noted in its file. In Columns the same
  Escape closes a column a user sees; not driven. The sequence is parked in
  `tests/qa/fuzz/corpus/views/`.
- **State:** open, [A6](packages/A6.md)'s, which drives the cause on GTK when it starts.

### #2427: grouping by file type moves a load's cursor to the first row shown on GTK

- **Code:** `divergence`, reclassified on 2026-11-13 from `oracle` by the coordinator's ruling.
  [Issue](https://github.com/lgse/strata/issues/2427), filed on 2026-11-13 as the finding's
  reference and closed at once: its state is kept here.
- **Seen:** by [A8](packages/A8.md)'s `views` alphabet, v4, shrunk 3 of 3 GTK against Rune from
  `fuzz-views-v4-0000000000000007`: the Appearance popover's List row, a double click on
  `documents`, the popover's "Group by file type", its Icons row, Ctrl+3. GTK's cursor is on
  `spreadsheet.csv`, the first row in the groups' order; Rune's on `notes.txt`, the first entry
  in the folder's order.
- **Driven on GTK** (`./scripts/qa-drive.sh`, 2026-11-13, by the review of
  [#2433](https://github.com/lgse/strata/pull/2433)): no view switch is needed. With
  `documents` open in List, its load's cursor on `notes.txt` and nothing selected, Group by
  file type then the popover's List row put GTK's cursor on `spreadsheet.csv`, the first
  grouped row: the regroup moves it. It goes by entry, not by index: with `spreadsheet.csv`
  chosen (End), regrouping keeps it there. Only a cursor the load placed moves, to the first
  row the pane shows.
- **Why:** GTK's stated rule, `sync_type_grouping` in `src/ui/browser_modes/rebuild.rs`: "The
  browser decides which listings show the groups, so its automatic cursor lands on the first
  row this pane shows." Rune's `shell::regroup` keeps the load's cursor on the first entry in
  the folder's order.
- **Concerns:** the regroup ([A8](packages/A8.md)'s, `shell::regroup`). The sequence is parked
  in `tests/qa/fuzz/corpus/views/`.
- **State:** open, A8's: a fix moves a load's cursor to the first row shown when the grouping
  changes, in a later pull request, after or alongside
  [#2425](https://github.com/lgse/strata/issues/2425)'s.

### #2422: the folder menu's Select All over filter results selects the hidden listing on GTK

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2422), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A10](packages/A10.md), asked by #2416's review, GTK driven in List and Icons
  (`./scripts/qa-drive.sh`): with a filter's results in the place of the pane's listing, the
  results' empty-space menu's Select All (`state.select_all`, `folder_menu.rs`, which outside
  Columns is `browser.select_all`) selects every entry of the hidden listing and no result;
  Escape then gives the listing back all selected. Ctrl+A in the results selects the results
  (`select_all_search_results`) and leaves the listing alone.
- **Concerns:** the filter in Icons and List; no scenario steps through it.
- **State:** open, set aside. Rune's pane menu does as GTK does (`single::rows`' `select_all`)
  until the owner rules on it; Ctrl+A in the results is C1e's own key and selects them on
  both.

### #2423: Tab in a rename on a filter result moves the hidden listing's cursor on GTK

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2423), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A10](packages/A10.md), asked by #2416's review, GTK driven in List and Icons
  (`./scripts/qa-drive.sh`). With the pane's filter results showing, Tab or Shift+Tab in a
  rename on a result commits it, and then `advance_rename` (`inline_edit.rs`) moves the
  browser's cursor (`move_selection`) in the listing under the results. No editor shows
  there, and the renamed result keeps the keyboard, as `renamed-filter-result-keeps-focus`'s
  `tab` runs expect. When the filter closes, the listing's cursor has moved one entry from
  where it was before the filter, and its selection has gone.
- **Concerns:** the filter in every view; the scenario checks the result only.
- **State:** open, set aside. Rune's Tab on a result only commits (`rename::advance` asks
  `filter::targets` first), and the browser's selection from before the filter stays: the
  scenario's visible behaviour, without the hidden move.

### #2420: closing a filter restores a load-cursor mark the browser no longer has on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2420), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [F3](packages/F3.md)'s `browser-view` invariant, on Rune, at the Escape that
  closes a type-to-search filter (`calibration v3`, seeds `0x02`, `0x06` and `0x0a`). By
  reading: the filter keeps the column's position before its results show, the load cursor's
  provisional mark with it (`Columns::listing_position`), and Escape restores it
  (`Columns::drop_hits`). The view then hands the selection over, which commits it in the
  browser (`Browser::commit_selection`), and the handover carries no provisional mark. So the
  view keeps a mark the browser no longer has, until a key moves the cursor.
- **What a user sees:** nothing found, as with #2386: the mark decides only a 10xer verb's
  targets and the cursor's own moves.
- **State:** open, [A10](packages/A10.md)'s; the three seeds parked in
  `tests/qa/fuzz/corpus/calibration/`.

### #2421: Escape from a pane's filter results leaves no element with the keyboard on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2421), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [F3](packages/F3.md)'s `keyboard` invariant in a hosted run of the QA backend,
  in List and Icons since [#2416](https://github.com/lgse/strata/pull/2416). Click
  `readme.md`, press Ctrl+F, type `no`, press Down so the keyboard is on `notes.txt` in the
  results, then Escape. The listing comes back with `readme.md` selected, but no element has
  the keyboard, and Down and Up change nothing until a click. Without the Down, Escape gives
  the keyboard to `readme.md`. No merged alphabet opens a pane's filter yet.
- **State:** fixed by [A10](packages/A10.md)'s
  [#2430](https://github.com/lgse/strata/pull/2430): Escape from a pane's results gives the
  listing the keyboard, as GTK does. F3's `qa::tests::invariants` filter tests in List and
  Icons go on past the Escape and hold the invariants there.

### #2435: Redo after undoing a rename into a new path does not redo it

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2435), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [F3](packages/F3.md)'s `undo-redo` invariant, the runner's own, on GTK and on
  Rune alike, in `entries` (seeds `0x0b` and `0x0a` of the draft v2, `0x04` and `0x0e` of
  v3). F2 on `archive` with `kept/one` typed, so the rename moves it into a new folder; Ctrl+Z
  puts `archive` back; Ctrl+Shift+Z or Ctrl+Y then does not redo the rename. Either nothing
  changes, or, with an older operation on the redo history (`v3-0x04`: a Move to Trash
  undone before), the Redo replays that one instead.
- **State:** open, set aside, two `entries v3` seeds parked. The fault is in the undo history
  both interfaces share, so its fix belongs on `main`; the coordinator places it with
  [#2387](https://github.com/lgse/strata/issues/2387).
