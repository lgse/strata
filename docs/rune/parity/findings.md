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
- **Reach:** of the scenarios that read the viewport, the only Icons one is this run, and none
  of the List ones is `required`: no required run passes for the wrong reason.
- **State:** open, [H3](packages/H3.md)'s. The fix finds the collection's own viewport, a
  descendant of the entry list, rather than the nearest ancestor.
