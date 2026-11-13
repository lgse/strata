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
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
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
  [Issue](https://github.com/lgse/strata/issues/2322), filed on 2026-11-14 as the finding's
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
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
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
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **What differs:** GTK's List headings are plain `gtk::Button`s
  (`src/ui/browser_modes/list_headings.rs`), with nothing keeping the keyboard off them, so
  they are Tab stops that Return and Space press (to sort). C4's headings are no Tab stops: the
  table is one focus stop, by an owner ruling C4's module docs record ("Headings aren't Tab
  stops", `crates/strata-components/src/list_table/mod.rs`); assistive technology presses a
  heading through its press action, and the keyboard cannot reach one.
- **Reach:** by reading, no scenario Tabs onto a heading. The gap is older than
  [#2329](https://github.com/lgse/strata/pull/2329), which made the headings `button`s for
  assistive technology and left them unfocusable. Found in #2329's review.
- **State:** open, waiting for the owner ([`owner.md`](owner.md#a8-view-switching-and-appearance),
  A8 item 1). C4's "owner ruling" was a reading: the owner's sign-off of 2026-11-10 covers
  the deviations marked OWNER in `docs/rune/work/STATUS.md`, and C4's row does not mark this
  one. Meanwhile the headings stay as they are, and C4's docs call it a deviation pending the
  owner's confirmation. [A8](packages/A8.md) builds focusable headings only if the owner
  asks for them.

### #2340: the thumbnail-size slider pages by 10 steps and steps onto its grid

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2340), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in the review of [#2336](https://github.com/lgse/strata/pull/2336). GTK's scale
  (`set_increments(16.0, 1.0)` in `src/ui/browser_modes/icons_controls.rs`) pages 1 px and steps
  16 from its value (65 to 81 after a drag); C20a's `Slider` pages its `Range`'s 10 steps and
  lands a step on the next grid point (80), as its module docs choose.
- **Concerns:** C20a's slider (`crates/strata-components/src/controls/slider/`), and through it
  the Icons pane header's "Thumbnail size" ([A5](packages/A5.md)). No QA run reaches these keys.
- **State:** open, C20a's, by the coordinator's placing; #2336 leaves C20a as it is.

### #2341: a modal popover cannot also close on a wheel outside it and let the wheel through

- **Code:** `rune-gap`. [Issue](https://github.com/lgse/strata/issues/2341), filed on 2026-11-14
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
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
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
- **Also by the location bar** ([A5](packages/A5.md)'s `location` alphabet, 2026-11-14): with
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
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A5](packages/A5.md)'s path suggestions pull request. GTK's suggestion row
  holds a `label` named after the folder (`documents/`); C6's row is a `list item` named so,
  with its parent as the description, and the text inside is not in the tree. A screen reader
  hears the same name, in another shape.
- **Concerns:** `locations/path-completion-dismisses-on-window-switch/columns`, whose line 6
  waits for the `label` (it is pending on [A21](packages/A21.md)'s second window as well); C6's
  suggestion list (`crates/strata-components/src/location_bar/popup.rs`).
- **State:** ruled by the coordinator (2026-11-14): C6 stays as it is, since a screen reader
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
- **Concerns:** C18's results, once hosted on Rune. Not yet run: no package hosts them.
- **State:** open, [A10](packages/A10.md)'s (search, filter and sort), noted in its file. When
  A10 takes it, it also checks a filtered Columns column: GTK's `focus_search_result` has a
  Columns branch (`column.selection.select_item`), and whether that reaches the browser's
  no-anchor rule (`set_selection` in `strata-fs`) has not been traced.

### #2379: Shift+Tab from a Columns column stops on its header's actions on GTK, on the sidebar on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2379), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
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
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A5](packages/A5.md)'s `location` alphabet. GTK's `submit_location`
  (`src/ui/browser/location.rs`) restores the entry's text only when a path fails validation;
  a path that validates and then cannot be opened ("Unable to open location") stays in the
  entry, and `begin_location_edit` reopens on it, all selected, as GTK's screenshot shows.
  Rune's `begin` (`crates/strata-app/src/location/mod.rs`) opens on the trail's text, the shown
  location's path.
- **Concerns:** `location v1`'s seed `0x7`, shrunk to 5 steps, confirmed 3 of 3, the same in
  24 of 24 GTK replays, and parked in `tests/qa/fuzz/corpus/location/`.
- **State:** open, [A5](packages/A5.md)'s to fix in `crates/strata-app/src/location/`.

### #2381: after an outside click closes "Unable to open location", GTK's keyboard is on the first crumb

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2381), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
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
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
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
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by the reviewer of [#2374](https://github.com/lgse/strata/pull/2374), by reading. On
  GTK the marquee's press calls `clear_marquee_selection`, which clears every column's selection
  and closes the deeper columns before the band moves. On Rune a press that becomes a band goes
  to the marquee (`crates/strata-components/src/columns/pointer.rs`), whose `begin`
  (`crates/strata-components/src/collection/marquee/machine.rs`) starts the band in its own
  column's selection only, so the other columns keep theirs.
- **Concerns:** no scenario yet drags from a column's background with entries selected in
  another column.
- **State:** open, [A8](packages/A8.md)'s.
