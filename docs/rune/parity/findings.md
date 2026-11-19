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
- **On [#2549](https://github.com/lgse/strata/pull/2549):** a ninth,
  `ui::window::tests::keyboard_dispatch::preview_ownership::default_archive_keys_follow_pane_focus`
  ("keyboard fixture did not settle", `src/ui/window/tests/keyboard_dispatch.rs:324`), and,
  with no GTK in it, `strata-desktop`'s
  `services::update_install::tests::unowned_release_binary_keeps_in_place_updates`
  (`crates/strata-desktop/src/services/update_install/tests.rs:258`, the install read as
  `Pacman`), each failed once in `./scripts/quality.sh all` in the pinned image at
  `1d4e3b8a9` on a loaded host. That change touches neither `src/` nor
  `crates/strata-desktop`; alone, the first passed 3 of 3 and the second 1 of 1
  (`./scripts/test-headless.py`).
- **On [#2561](https://github.com/lgse/strata/pull/2561), [A13](packages/A13.md)'s closing pull request:** a tenth,
  `ui::media::tests::reopening_resumes_where_the_preview_closed`
  (`src/ui/media/tests.rs:99`, `assertion failed: media.imp().first_frame.get()`), failed once
  in `./scripts/quality.sh all` in the pinned image at `17d3fa87d` on a loaded host, and 1
  time in 3 alone (`./scripts/test-headless.py`). At `60c062c9a`, with `strata-next` merged
  again, the ninth failed once more there, with its message. That change touches no `src/`.
- **On [#2594](https://github.com/lgse/strata/pull/2594), [A21](packages/A21.md)'s first slice:** the ninth failed once more in
  `./scripts/quality.sh all` in the pinned image at `5da7cb20e`, with its message ("keyboard
  fixture did not settle", `src/ui/window/tests/keyboard_dispatch.rs:324`). It passed alone
  (`./scripts/test-headless.py`), and the rerun of `./scripts/quality.sh test` passed whole
  (4074 tests). That change touches no `src/` and no crate the GTK lane builds.
- **On [#2604](https://github.com/lgse/strata/pull/2604), [A21](packages/A21.md)'s Slice 2a, at `4ed45f2d5`:** the ninth failed once more in
  `./scripts/quality.sh all`, with its message. It then passed alone. The rerun of
  `./scripts/quality.sh test` failed on an eleventh instead, `strata-desktop`'s
  `services::update_install::tests::repository_probe_selects_strata_instead_of_a_dependency`
  (`crates/strata-desktop/src/services/update_install/tests.rs:185`: "could not query the
  package repository: The file is running and cannot be changed", an `ETXTBSY` on the fake tool
  it writes and runs), which then passed alone 3 of 3. A second rerun passed whole (4074 tests).
  That change touches `strata-fs`'s FileManager1 adapter (two items made public) and the Rune
  side, neither the keyboard fixture nor `strata-desktop`.
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
- **Sighting on [#2483](https://github.com/lgse/strata/pull/2483):** a ninth,
  `ui::browser::progress::tests::minimization::deletion::fast_delete_animates_only_its_own_success_without_showing_progress`
  (`src/ui/browser/progress/tests/minimization.rs:182`, `progress transition timed out`,
  sibling of the first above), failed once in `./scripts/quality.sh test` (shard 0, lane 3,
  109 of 110 passed; all other 11 lanes passed) at `3eca49ef1`, and passed alone under
  `./scripts/test-headless.py` (7.49 s). That pull request's GTK-side change only delegates
  the five item-customization getters/setters in `src/ui/preferences/file_settings.rs` and
  the two emoji/choice helpers in `src/assets.rs` to `strata_desktop::settings`, which
  `minimization::deletion` never calls.
- **Sighting on [#2550](https://github.com/lgse/strata/pull/2550):** a tenth, in
  `strata-desktop`, `services::update_install::tests::repository_probe_selects_strata_instead_of_a_dependency`
  (`crates/strata-desktop/src/services/update_install/tests.rs:185`), failed once in
  `./scripts/quality.sh all` (shard 0, lane 3) at `cca75e9ac` with `Err("could not query the
  package repository: The file is running and cannot be changed")`: `ETXTBSY` on the probe
  script it had just written and made executable, as when another thread's fork holds the
  file open for writing. It passed 3 times of 3 alone under `./scripts/test-headless.py`,
  and the whole `./scripts/quality.sh test` passed on rerun. That pull request touches no
  shared crate and no `src/`.
- **Sighting on [#2548](https://github.com/lgse/strata/pull/2548):** an eleventh, the tenth's
  sibling in `strata-desktop`,
  `services::update_install::tests::unowned_release_binary_keeps_in_place_updates`
  (`crates/strata-desktop/src/services/update_install/tests.rs:258`, `left: Pacman, right:
  InPlace`), failed in `./scripts/quality.sh all --jobs 4` and again in the one rerun of
  `./scripts/quality.sh test --jobs 4` (shard 0, lane 3, 79 of 80 passed each time) at
  `4bc5cd808`, and passed alone twice under `./scripts/test-headless.py`; it had passed in
  the same image at `b7bf69ec2`, and `crates/strata-desktop` did not change between the two.
  The likely cause is the tenth's, in the test itself: `ownership_probe` writes a `pacman` script and
  `update_method_for` runs it at once, so a fork on another test thread that still holds the
  script's write descriptor makes `exec` fail with `ETXTBSY`, which `update_method_for` reads
  as owned (its `Err(error)` arm), hence `Pacman`. That pull request changes no `src/` and no
  shared crate: only `crates/strata-app`, `crates/strata-components`, the QA files and
  Markdown, none of which the GTK lane compiles. It failed twice, so by the rule above the
  agent stopped chasing it and reported it; after `strata-next` was merged again it passed in
  `./scripts/quality.sh all --jobs 4` at `c3dcb8917`. A fix, if taken up, lands on `main`.
- **Cross-reference from the Rune lane, on [#2567](https://github.com/lgse/strata/pull/2567):**
  the same `ETXTBSY` pattern in [A14](packages/A14.md)'s
  `terminal::tests::ghostty_opens_directories_through_new_window_only_from_1_3`
  (`crates/strata-app/src/terminal/tests.rs:206`, `"Ghostty 1.3.1" via "ghostty"`: `left:
  ["--working-directory=/tmp/work"]`, without `+new-window`). It failed once in a full
  `mise run test-rune` at `731c24b50` and passed alone. The test writes a `#!/bin/sh` probe and
  runs it at once to read the version (`terminal/tests.rs:194-203`); a fork on another test
  thread holding the write descriptor fails the probe, and the legacy arguments are chosen.
  #2567 does not touch `terminal/`. The fix, if taken up, is A14's: write the script under a
  temporary name and rename it, or retry on `ETXTBSY`. Seen again in a full `mise run
  test-rune` for [#2579](https://github.com/lgse/strata/pull/2579) on its way to `ed4a3bdcd`,
  with the same assertion; it passed alone three times and in the full rerun, and #2579 does not touch
  `terminal/` either.
- **Sighting on [#2577](https://github.com/lgse/strata/pull/2577):** the ninth again,
  `fast_delete_animates_only_its_own_success_without_showing_progress`
  (`minimization.rs:182`, `progress transition timed out`), failed once in
  `./scripts/quality.sh all --jobs 4` (shard 0, lane 3; the other three lanes passed) at
  `1ba2778c1909325066429fdf368128030d2ddea1`, and passed 3 times of 3 alone under `./scripts/test-headless.py` (7.7 s each).
  That pull request changes no `src/`.
- **Sighting on [#2589](https://github.com/lgse/strata/pull/2589)**, A11's close (at `97d5e0e8e`): the
  eleventh, `unowned_release_binary_keeps_in_place_updates` (`tests.rs:258`, `left: Pacman`),
  failed in `./scripts/quality.sh all --jobs 4` and again in the one rerun of
  `./scripts/quality.sh test --jobs 4` (shard 0, lane 3, 79 of 80 each time), as on #2548, and
  passed 3 times of 3 alone under `./scripts/test-headless.py`. The branch changes no `src/`
  and no shared crate. Twice on one pull request: not chased further, by the rule above.
- **Sighting on [#2589](https://github.com/lgse/strata/pull/2589) after review** (at
  `ccf43b177`, `strata-next` at `7426fd041` merged in): the ninth,
  `default_archive_keys_follow_pane_focus` (`keyboard_dispatch.rs:324`, "keyboard fixture did
  not settle"), failed in `./scripts/quality.sh all --jobs 4` and again in the one rerun of
  `./scripts/quality.sh test --jobs 4` (shard 0, lane 2, 331 of 332 each time; the other lanes
  passed), and passed 3 times of 3 alone under `./scripts/test-headless.py`. The branch's GTK
  lane is `strata-next`'s: no `src/`, shared crate or manifest differs. Not chased further.
- **Sighting on [#2589](https://github.com/lgse/strata/pull/2589) after the second review** (at
  `b86787184`, `strata-next` at `8c2ffda4f` merged in): the tenth,
  `repository_probe_selects_strata_instead_of_a_dependency` (`tests.rs:185`, `ETXTBSY`: "The
  file is running and cannot be changed"), failed once in `./scripts/quality.sh all --jobs 4`
  (shard 0, lane 3, 79 of 80; the other lanes passed) and passed 3 times of 3 alone under
  `./scripts/test-headless.py`. The branch changes no `strata-desktop`.
- **Sighting on [#2599](https://github.com/lgse/strata/pull/2599)** ([A16](packages/A16.md)): the
  tenth, `repository_probe_selects_strata_instead_of_a_dependency`, failed once more in
  `./scripts/quality.sh all --jobs 4` (shard 0, lane 3, 79 of 80) at `42a5a67f1`, with the same
  `ETXTBSY` message, and passed 3 times of 3 alone under `./scripts/test-headless.py`. That pull
  request touches no shared crate and no `src/`. In its Rune lane on the way there, `mise run
  test-rune` once failed `footer::tests::clipboard_badge_follows_copy_cut_paste_and_text_copy`
  (`crates/strata-app/src/footer/tests.rs:213`, "consuming a cut withdraws the clipboard
  badge": the badge still showed) with the load average near 10. It passed 3 times of 3 alone
  and in the full rerun (3058 passed); #2599 does not touch the footer or the clipboard.
- **On [#2620](https://github.com/lgse/strata/pull/2620), [A21](packages/A21.md)'s Slice 3, at `ca098e391`:** the tenth failed once more in
  `./scripts/quality.sh all --jobs 4` (shard 0, lane 3, 79 of 80) with its `ETXTBSY` message, and
  passed 3 times of 3 alone; a rerun of `./scripts/quality.sh test` passed whole (4076 tests).
  That change touches no shared crate and no `src/`. In the same gate `./scripts/qa.sh` failed
  only #2162's run, at its line 34.

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
  Seen again on 2026-11-14, in a full `mise run test-rune` for [A4](packages/A4.md)'s
  modifiers hook, run alone at a load average of 6 to 7: the same assertion at the same line.
  That branch did not touch `strata-platform` either. Alone, the test passed 3 of 3.
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
- **Also met by a deselecting Ctrl+click in List** ([A9](packages/A9.md)'s `pointer v2`, never
  merged, seed `0x10`). After a Ctrl+click that takes `pictures` out of the selection, GTK's
  next `j` moves from the entry before it, the cursor `pointer_selection` left, while its focus
  is on the clicked row. Rune moves from the clicked row. `pointer v3` Ctrl+clicks only entries
  not selected.
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
- **Also by the preview** ([A15](packages/A15.md)'s `preview` alphabet, 2026-11-16): with a
  folder's column opened by a click and the preview's placeholder beside it, Escape closes the
  preview and gives the column the keyboard (`return_from_preview`, `focus_active`); GTK then
  reports its first entry (`broken.cbz`) focused with nothing selected, and Rune focuses the
  column with no entry. Parked in `tests/qa/fuzz/corpus/preview/` as `v4-000000000000000b`.
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
- **Why:** GTK's selection menu has an Open row (`MultipleOpenOptions`), but hides it until its
  open-with lookup finds a default application the entries share, and shows it only then
  (`prepare_open_with` in `src/ui/browser/context_menu/open_with.rs`). In the fixture none is
  found.
- **State:** fixed by [A6](packages/A6.md)'s pull request [#2446](https://github.com/lgse/strata/pull/2446), which leaves the row out as GTK does
  when the lookup finds none. The lookup is [A14](packages/A14.md)'s, which shows the row when
  it finds a shared default. Replayed GTK against Rune: `columns v2` `0x10` and `columns v3`
  `0x10` agree and are unparked. `0x11` and `0x14` of `columns v3` now part later, at a click
  an open menu covers on one target only, and are parked against
  [#2445](https://github.com/lgse/strata/issues/2445).

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
- **Twin written** by [A21](packages/A21.md)'s Slice 2a ([#2604](https://github.com/lgse/strata/pull/2604), 2026-11-18):
  `locations/path-completion-dismisses-on-window-switch-rune`, whose lines 6 and 10 wait for the
  `list item`; the run is `parallel`.

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
  `PointerSettings::missing_anchor_stays` (named `secondary_keeps_no_anchor` until
  #2431's fix), off by default, leaves a missing anchor
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
  Seen again on 2026-11-14, in a full `mise run test-rune` for [A6](packages/A6.md)'s
  [#2475](https://github.com/lgse/strata/pull/2475), run while a `./scripts/qa-parity.sh` gate loaded the machine:
  `undo::tests::an_undo_with_an_item_in_the_way_moves_nothing_and_keeps_its_entry` saw its
  undo answer `Declined` where it expects `Ran` (`crates/strata-app/src/undo/tests.rs:156`).
  The branch did not touch undo. Alone, the test passed.
  A third, outside undo, seen in the review of the same pull request: in the full lane at a
  load average of 13 to 20,
  `sidebar::tests::the_arrows_move_along_the_header_and_down_returns_to_the_sidebar` failed
  twice; alone, it passed 3 of 3. It has no owner.
  A fourth, in this pull request's own lane at a load average of 33:
  `sort::tests::a_columns_menu_sorts_its_own_column` said "the inner folder's column" after its
  30 s wait (`crates/strata-app/src/sort/tests.rs:167`); alone, it passed 3 of 3. It has no
  owner.
  The sidebar's again on 2026-11-17, in the full lane of [A16](packages/A16.md)'s Slice 2 at
  `bb330eb69`: after Left the keyboard was on `location-copy` where `header-sidebar-toggle` was
  wanted (`crates/strata-app/src/sidebar/tests.rs:550`). That slice changes only the preview's
  viewers; alone, the test passed 3 of 3.
- **Concerns:** those four tests: the two of [A7](packages/A7.md)'s undo, the sidebar's and
  the sort's.
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
  size" ([A5](packages/A5.md)), the footer's "Jobs" dashboard (C16's, [A14](packages/A14.md)),
  and "Customize columns…"'s menu is C4's and not one. The
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
  "Jobs", "Thumbnail size"; "Jobs" added by [A14](packages/A14.md)'s
  [#2550](https://github.com/lgse/strata/pull/2550)): a rule that names its values, which the map takes since this finding.
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
- **State:** fixed by [A6](packages/A6.md)'s pull request [#2452](https://github.com/lgse/strata/pull/2452): an Escape outside text closes the
  peek or clears the active column's selection at the `dismissal` stage, as on GTK
  (`crates/strata-app/src/window_keys.rs`). Driven on GTK again on 2026-11-14 with a click
  on the Appearance button: the second Escape clears the selection and the focus ring stays
  on the button. `fuzz-views-v3-000000000000000b` agrees in 4 replays and is unparked.

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
  steps, and in `tests/qa/fuzz/corpus/tabs/` (`fuzz-tabs-v3-000000000000000c`).
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
- **State:** fixed by [A8](packages/A8.md): one memory for the window's single-pane views, held
  by the shell (`pane::Memory`), and nothing remembered while a view shown again is rebuilt.
  `appearance::tests::a_folder_comes_back_by_back_where_another_view_left_it` and
  `a_folder_comes_back_by_the_parent_where_another_view_left_it` hold it. Seeds `0x11`,
  `0x13` and `0x14` are no longer parked: the corpus replay holds them to agreeing on both
  targets. Seed `0x01`, run whole again, met [#2437](https://github.com/lgse/strata/issues/2437)
  further on, and its file holds that sequence now.

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
- **State:** fixed by [A6](packages/A6.md)'s pull request [#2452](https://github.com/lgse/strata/pull/2452). Driven on GTK on 2026-11-14: in
  Columns, with nothing selected in the open column, Escape closes it, and the keyboard goes
  to its folder in the column before; in List nothing visible changes, as the column closed
  is one List does not show. Rune reads an Escape that reaches the listing with nothing to
  clear at the `item_navigation` stage and calls the browser's `escape()`
  (`crates/strata-app/src/window_keys.rs`). `fuzz-views-v4-000000000000000f` agrees in 4
  replays and is unparked.

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
- **Driven again on GTK** (2026-11-14), which corrects the reading above: the cursor in
  `documents` is no load's. A double click opens the folder with a first visit, after which
  GTK's browser holds no cursor at all (`grouped_focus_drifted` in
  `src/ui/browser_modes/events.rs` asks for none); the ring on `notes.txt` is the list widget's
  own focus. So each GTK view rebuilt over it rings its own first row shown: the grouped List
  `spreadsheet.csv`, the List ungrouped again and Icons and Columns `notes.txt`. F2 renames and
  Return opens the ringed row. A cursor a load placed, Alt+Up's folder come from, stays on its
  entry in every view rebuilt, grouped or not.
- **Fixed in part** by [#2458](https://github.com/lgse/strata/pull/2458) (2026-11-14). The
  Rune side must hand the first visit's cursor to the browser, for F3's `browser-view`
  invariant, so the single panes' shared memory keeps it as the visit's until the user's own
  input moves the cursor or the selection, and a single pane that shows it off its first row
  moves it there (`pane::Memory`, `single::first_visit`). The parked sequence is in the corpus
  unparked.
- **Remaining in Columns, driven on GTK** (`./scripts/qa-drive.sh`, 2026-11-14): in List with
  "Group by file type" on, a double click on `documents` (the first visit: the ring on
  `spreadsheet.csv`, nothing selected), then Ctrl+1, then Down.
  - Ctrl+1: GTK's Columns rings its first row, `notes.txt`, nothing selected; Rune's shows the
    browser's `spreadsheet.csv`.
  - Down: GTK's selects `notes.txt` in place, the ring staying on it, since its browser holds
    no cursor and a cursorless column's first key picks its first row; GTK's List, whose
    widget has a focus row of its own, moves on instead.
  - Matching it needs both halves: the single panes' rule for Columns' active column, and the
    first key selecting the cursor's row in place while the mark holds, key handling in
    `columns/`, possibly with a hook in C2's keyboard. The rule alone would only move the
    difference to the next key. Left by the coordinator's ruling of 2026-11-14, for its narrow
    path.
- **State:** fixed in List and Icons; the Columns residual is open.

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

### #2437: after Up in List, Rune's Columns opens the folder Up came from, GTK's does not

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2437), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A8](packages/A8.md)'s `views` alphabet, v4, once
  [#2425](https://github.com/lgse/strata/issues/2425) was fixed: seed `0x01`, run whole, shrunk
  3 of 3 GTK against Rune to Ctrl+2, Ctrl+period twice, a double click on `documents`, the
  Appearance popover's Hidden files row, Ctrl+3, Alt+Up, the popover's Columns row. GTK's
  Columns shows the root alone (`app.matches` its entries); Rune's opens `documents`, the
  folder Up came from, as the child column.
- **Why, not yet known:** on GTK the selection Up placed is still the cursor the navigation
  placed, which `follow_selection_on_return` does not open; on Rune it counts as a choice by
  then. The same steps part the same way on Rune without #2425's fix (replayed on both), so it
  is older than that fix.
- **Driven on GTK** (`./scripts/qa-drive.sh`, 2026-11-14): after the popover's Columns row,
  GTK's root column shows `documents` selected with the focus ring on it and no child column
  on screen; the next Down moves to `pictures` and opens its column. Without the hidden-files
  toggles GTK ends the same way.
- **On Rune, probed in a hosted test** (2026-11-14): in List alone, with no view switch, a
  folder opened and then Alt+Up leaves `Browser::selection_is_load_cursor` false, with
  `documents` selected; it was true in the folder before the Up. So the view hands the
  browser the cursor Up placed as a choice, where GTK keeps it the navigation's. Where, not
  yet found: the single pane's `settle` (`pane.rs`), through a remembered position coming back
  or a cursor the collection placed itself (`handover::placed`).
- **Cause, found** (2026-11-14), two of them:
  - Alt+Up's root comes with no reveal pending, so the root's remembered position comes back,
    `documents` chosen, and the single pane handed it to the browser as a choice
    (`handover::hand_over` commits), which cleared the load's cursor. GTK's
    `restore_selection` sets the browser's selection without committing it.
  - A view's rebuild (`shell::rebuild`) applied the replay outside the queue, so no part heard
    `caught_up`, and Columns took the cursor it was rebuilt with for a move of its own and
    mirrored it. GTK's views mirror from `FocusChanged`, which no rebuild raises.
- **Fixed** by [#2458](https://github.com/lgse/strata/pull/2458) (2026-11-14): a restore is handed back with no commit
  (`handover::hand_back`), and the parts catch up after a rebuild. The sequence is in the
  corpus unparked.
- **State:** fixed.

### #2431: a Shift+click in pane filter results with no anchor anchors there on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2431), filed on
  2026-11-13 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in #2428's review, by scenario probes on both targets in Icons and List: right-click
  `a1.txt` in the results, Escape, Shift+click `a3.txt`, Shift+click `a1.txt`. GTK selects
  `a1.txt` alone; Rune selected `a1.txt` to `a3.txt`. GTK's results keep an anchor of their
  own, which a press sets to what `pointer_selection` returns, the anchor it was given for a
  Shift+press (`src/ui/collection_interaction.rs`); C1e's Shift+press with no anchor started
  one at its entry. In Columns both range.
- **State:** fixed, by [A10](packages/A10.md)'s `contract` pull request [#2434](https://github.com/lgse/strata/pull/2434), to match GTK
  by the coordinator's ruling, which the owner may reopen ([`owner.md`](owner.md), A10):
  C1e's `PointerSettings::missing_anchor_stays` covers a Shift+press as well as a secondary
  press, and the pane's results turn it on; Columns leaves it off.

### #2429: the header's Appearance button is a button holding a toggle button on GTK

- **Code:** `oracle`, by the coordinator's ruling of 2026-11-14, which the owner may reopen
  ([`owner.md`](owner.md#a8-view-switching-and-appearance), A8 item 3); filed as `divergence`.
  [Issue](https://github.com/lgse/strata/issues/2429), filed on 2026-11-14 as the finding's
  reference and closed at once: its state is kept here.
- **Seen:** by [A10](packages/A10.md)'s `search_filter_sort/grouped-list-keeps-file-targets/list`,
  which stopped on Rune at line 26, a click on `{"role":"button","name":"Appearance"}`.
  GTK's tree, driven (`./scripts/qa-drive.sh`): a `GtkMenuButton role="button"` named
  "Appearance" holding a `GtkToggleButton role="toggle button"` of the same name. Rune's header
  has the toggle button alone. Thirty-two shared steps aim at the toggle button, which both
  have.
- **State:** closed by the coordinator's ruling: the pair is GTK's widget structure, not
  behaviour a user meets, so Rune adds no outer node, and line 26 of the scenario aims at the
  toggle button as the others do ([A8](packages/A8.md)'s pull request; the scenario's ledger
  row is A10's).

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
- **State:** fixed in [A10](packages/A10.md)'s closing pull request: `Columns::drop_hits`
  adopts the browser's marks (`events::marks`) and settles the mirror (`Columns::caught_up`)
  after restoring the saved position, as GTK's `resync_column_selection` does, and
  `Columns::take_hits` refreshes `held[depth]`. Seeds `0x02`, `0x06` and `0x0a` of
  `calibration v3` are unparked and pass 20 of 20 on Rune.

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

### #2441: closing a Columns column's Sort by menu leaves the keyboard on its button on GTK, on the view on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2441), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** driving GTK for [A10](packages/A10.md)'s sort controls. Escape or a click outside
  closes a Columns column's Sort by menu, and GTK's keyboard stays on the header's "Choose
  sort field" button. C2's header-actions slot asks for its buttons to be built
  `focusable(false)`, so Rune's `focus_return` has nowhere to land. A10's `sort` part gives
  the keyboard to the Columns view instead.
- **Concerns:** `crates/strata-app/src/sort/` (`refocus`), and C2's header slot
  (`ColumnsView::header_actions`). No run checks it.
- **State:** open. The blocker is C2's slot rule that a column header's actions are no focus
  stops, the rule [#2379](https://github.com/lgse/strata/issues/2379) waits on
  ([`owner.md`](owner.md#a5-location-bar), A5 item 2). Changing it is a separate `contract`
  question.

### #2445: a click aimed at an entry lands inside an open menu on one target only

- **Code:** `exception` (`toolkit`). [Issue](https://github.com/lgse/strata/issues/2445), filed
  on 2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** replaying the Columns pilot's parked sequences once #2358 was fixed. A fuzz step
  clicks an entry by name with a context menu open. On GTK, the Menu key opened the menu over
  `todo.txt`, so a Shift+click aimed at it lands in the menu: the menu stays open, and the
  next keys move in it. On Rune the same menu opens where `todo.txt` is not covered, so the
  click lands outside and the menu closes.
- **Why:** each toolkit places the popup by its own geometry. Driven on GTK
  (`./scripts/qa-drive.sh`, 2026-11-14), a Shift+click on an entry outside the open menu
  closes it, takes no selection and leaves the keyboard on the cursor's entry, as Rune does.
- **Concerns:** seeds `0x11` and `0x14` of `columns v3` (steps 43 and 42), parked in
  `tests/qa/fuzz/corpus/columns/`. The Columns alphabet's upkeep is [F2](packages/F2.md)'s
  and [F4](packages/F4.md)'s: for example, aim an entry click only while no menu is open.
- **State:** open, set aside for the alphabet's upkeep.

### #2451: a secondary press on an open menu's row runs the row on GTK and does nothing on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2451), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A6](packages/A6.md), checking a remark of its survey. Driven on GTK
  (`./scripts/qa-drive.sh`, List view): right-click `todo.txt`, then right-click the menu's
  Rename row. The menu closes and the rename starts. On Rune, a probe of C21's menu
  (`strata-components`) pressing and releasing the secondary button on a row runs nothing,
  and the menu stays open. rune-kit's menu rows take no secondary press, so Rune's
  `secondary_target` finds no handler from the row up.
- **Not the same as** the feature map's "pressing the secondary button, moving onto a menu
  row, and releasing does not run the row" ([#1621](https://github.com/lgse/strata/issues/1621)),
  where the press starts outside the menu.
- **Concerns:** no scenario or fuzz alphabet yet.
- **State:** open, for the owner: whether the Rune interface runs a row on a secondary
  press, as GTK's popover does ([`owner.md`](owner.md#a6-menus-dialogs-and-keymap)).

### #2454: a double-click right after a dialog over the view closes does nothing on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2454), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A6](packages/A6.md)'s Run confirmation, in
  `dialogs_and_menus/executable-requires-confirmation/columns`. A double-click on `run-me`
  opens "Run this program?", the dialog closes, and a second double-click on `run-me`
  follows within the double-click interval of the first. GTK opens the dialog again: its
  modal layer ends the view's click run, as a grab does. On Rune nothing opens: no modal
  ends the collection's click run (`collection/pointer/clicks.rs` in `strata-components`), so
  the second double-click's presses count as the run's third and fourth, and the collection
  acts only on the first and second. With a pause of 600 ms before the second double-click
  the run passes.
- **Concerns:** `executable-requires-confirmation/columns` (line 9), and any double-click
  soon after a dialog over the view closes.
- **State:** fixed by [A6](packages/A6.md)'s [#2461](https://github.com/lgse/strata/pull/2461): a part that starts answering
  `Holds::Modal` ends the shown view's pointer gestures (`shell/grab.rs`), as GTK's modal
  layer does, and `executable-requires-confirmation/columns` is `required`.

### #2460: after a regroup, Rune's matches keep a typed query's, GTK's show every entry

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2460), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by the review of [#2458](https://github.com/lgse/strata/pull/2458), in a sequence
  replayed GTK against Rune on the canonical fixture with type to search on: in List, open
  `documents`, type `r`, switch views and back, then the Appearance popover's "Group by file
  type". GTK's `app.matches` lists all three entries; Rune's only `report.md` and
  `spreadsheet.csv`. It parts the same way on `strata-next` without #2458, so it is older.
- **Why:** driven on GTK by [A10](packages/A10.md), with and without the view switch: "Group
  by file type" closes the List pane's filter, its field, query and results, since
  `set_group_by_type` rebuilds the List pane and the filter with it (`rebuild_list` in
  `src/ui/browser_modes/rebuild.rs`). The keyboard stays on the Appearance button, and the
  listing comes back grouped with the selection it had. Rune's regroup rebuilt the view and
  left the filter open.
- **Concerns:** the typed search ([A10](packages/A10.md)'s), through the regroup's rebuild
  (`shell::regroup`, [A8](packages/A8.md)'s).
- **State:** fixed by [#2465](https://github.com/lgse/strata/pull/2465): the Group row
  closes the filter before it regroups, in a view that groups (`filter::close_for_regroup`).

### #2468: Escape from a search result's menu leaves GTK's keyboard on an unnamed box

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2468), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** driving GTK for [A10](packages/A10.md)'s search palette. Ctrl+K, a query, a
  right-click on a result, then Escape: the menu closes and the keyboard lands on the
  palette's layer box (`.search-backdrop`), which has no role and no name. The next character
  typed is lost. Down gives the field the keyboard back (`move_selection` in
  `src/ui/search.rs`).
- **Concerns:** `show_result_menu` in `src/ui/search/results.rs`. No run checks it.
- **State:** open, set aside, and not copied: on Rune the keyboard goes back to the palette's
  field, with the result still highlighted (`palette::menu`), by the coordinator's ruling.
  The fix belongs on `main`.

### #2470: the Rune search palette's Alt+Return does not follow a remapped key

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2470), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** building [A10](packages/A10.md)'s search palette. GTK's palette reads
  `open_containing_folder` with `press.is(OPEN_CONTAINING_FOLDER)` in its `handle_key`
  (`src/ui/search.rs`), through the active keymap, so a user's binding moves it. Rune's
  palette reads C17's reveal chord, its default `alt+enter` with Super tolerated. The window's
  commands cannot read it instead: Rune's single-line field takes Return with any modifier
  once C17's key handler passes it on.
- **Concerns:** `crates/strata-app/src/palette/` and C17's `SearchPalette::reveal_keys`. No
  run checks it: the scenarios use the default key.
- **State:** open, A10's. The fix is a converter from a catalog binding to a `rune::Chord`,
  for `reveal_keys`, or a C17 hook that hands the host each key before the field's editor.

### #2466: the Rune shortcut reference is a table of cells where GTK's is labels

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2466), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **What differs:** what a screen reader hears of the shortcut reference (F1). GTK has a
  visible `label` "Keyboard shortcuts" in a dialog of that name, and each row is a box of two
  `label`s, the action and its keys (`src/ui/shortcut_footer/build.rs`, `catalog.rs`), with
  every row built. C19's `ShortcutReference` names the dialog and hides its title, so the
  name is not heard twice; its rows are a `table`, each section's title a `row header` and
  each shortcut a `table row` of two `table cell`s; and it builds only the rows near the
  viewport (`crates/strata-components/src/footer/reference/view.rs`).
- **Concerns:** `dialogs_and_menus/shortcut-reference-search-keeps-editing-chords/columns`
  (the title at line 3, the Preview media rows' keys at lines 4 to 7, rows below the
  viewport) and `keymap/saved-keymap-applies-at-launch/columns` (the title at line 19, the
  Rename row's labels at line 20).
- **State:** the coordinator ruled on 2026-11-14 that C19's table stays; the owner may reopen
  it ([`owner.md`](owner.md#a6-menus-dialogs-and-keymap)). Both runs are `parallel` under
  `a11y` twins from [A6](packages/A6.md)'s [#2471](https://github.com/lgse/strata/pull/2471), which find the
  reference by its dialog's name, read the rows' cells, and search to bring the media rows
  into the tree.

### #2469: the tab strip's close buttons are named "Close tab" in English in every language

- **Code:** `rune-gap`. [Issue](https://github.com/lgse/strata/issues/2469), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by the review of [#2464](https://github.com/lgse/strata/pull/2464): each tab's
  close button in the Rune window's strip is named "Close tab" in English, whatever the
  language. GTK names it with the catalogs' "Close tab" (`src/ui/window/composition/tabs/strip.rs`),
  which all thirteen languages translate.
- **Why:** rune-kit's `TabStrip` labels the button itself (`rune-kit/src/shell/tab_strip/tab.rs:307`
  at `2a44156`) and offers no way to set it: only the close icon and whether the button is a
  Tab stop are settable, so C7's `BrowserTabs` cannot pass Strata's label.
- **Concerns:** the strip's accessible names ([A4](packages/A4.md)'s done-when item 2) in the
  twelve languages other than English. No run reads the name in another language.
- **State:** fixed on Rune `631e8a67` (`rune_kit::shell::TabStrip::words` / `TabWords::close`,
  anomalyco/rune#922, packet entry [39](../upstream/rune/39-kit-tab-strip.md) item 1): C7's
  `BrowserTabs::words` forwards `TabWords` to `TabStrip`, and `strata-app`'s `tab_strip` passes
  GTK's translated `"Close tab"` string (`the_close_buttons_speak_the_users_language`).

### #2473: a close the window manager asks for is not refused while file operations run

- **Code:** `rune-gap`. [Issue](https://github.com/lgse/strata/issues/2473), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A4](packages/A4.md), reading GTK's close guard: GTK refuses every close of a
  window while one of its tabs has a file operation running, and shows "File operations are
  still active" (`close_guard::install` in `src/ui/window/composition/tabs.rs`). On Rune the
  header's and the strip's Close and Ctrl+W on the last tab refuse the same way
  (`shell::close_window`); a close from the window manager (its title bar, Alt+F4, the
  compositor) does not.
- **Why:** Rune's `App::on_close` runs before the window goes and cannot keep it (at
  `2a44156`), and nothing else hears a close request.
- **Concerns:** no run: no scenario closes a window with an operation running.
- **State:** drafted and worked around by [A21](packages/A21.md)'s Slice 2b ([#2605](https://github.com/lgse/strata/pull/2605), 2026-11-18). Rune
  packet entry 110 (`docs/rune/upstream/rune/110-refusable-close-request.md`) asks for a close
  request the app can refuse. Until then, `on_close` opens the window's state again in a new
  window, which says why (`crates/strata-app/src/host/windowed/refused.rs`). The same guards
  now also refuse the last window's close while a custom action's job runs, as GTK's jobs
  guard does. Open until the entry is filed and answered.

### #2477: `reopen-closed-tabs/columns` sometimes misses the click on a new tab's folder

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2477), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A10](packages/A10.md)'s `all --target rune` on #2472, then reproduced by
  [A4](packages/A4.md), each run alone under the machine lock. The run fails at line 4, "the
  browser to be working in 'archive'", after Ctrl+T and a `select` of `archive` in the new
  tab: `archive` ends selected and focused, and no child column opens.
  - Rune at `48460bc99`: 3 of 30 unloaded, 2 of 10 with every core kept busy.
  - Rune with [#2474](https://github.com/lgse/strata/pull/2474): 0 of 30 unloaded, 2 of 10
    loaded.
  - GTK under the same load: 0 of 10.
- **Why** (confirmed with probes in the QA runs):
  - The press was aimed and routed right: tab-2's `row-1`, with its listing there.
  - The column's collection dropped the press as outside its viewport. That viewport, kept
    from `on_layout`, said x=0 where the column was drawn at x=201.
  - A new tab's sidebar started only in its first `poll`, so the tab's first frame laid the
    files out where the sidebar goes (x=0), and the next one at x=201.
  - Rune lost the second report: once layout feedback falls quiet (`ECHOES` feedback-only
    commits in a row), a moved rect is recorded without its `on_layout` and never reported
    later. Under load that move landed in a quiet frame. Rune packet entry
    [106](../upstream/rune/106-quiet-layout-loses-a-move.md).
- **Concerns:** `tabs/reopen-closed-tabs/columns` (required), and any click in a tab just
  opened.
- **State:** fixed in strata-app by [#2478](https://github.com/lgse/strata/pull/2478): a new tab's sidebar starts before its
  first frame (`Sidebar::tab_shown`), so the files never lay out at x=0. 30 of 30 runs pass
  loaded and 30 of 30 unloaded. The Rune gap stays: any other move laid out while feedback is
  quiet can still lose its `on_layout` report.

### #2479: the Rune Customize dialog's Clear is enabled where GTK's is insensitive

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2479), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **What differs:** GTK's Customize dialog makes Clear insensitive while the item has no
  saved colour or icon, and sensitive once a choice is made (`build_clear_button` in
  `src/ui/browser/customization.rs`), as driving GTK shows. C15's `StrataDialog` gives its
  secondary button no disabled state, so the Rune dialog's Clear is always enabled, and
  pressing it with nothing saved forgets nothing.
- **Concerns:** [A6](packages/A6.md)'s Customize dialog (`crates/strata-app/src/customize.rs`).
  No run reads Clear's state.
- **State:** accepted for now by the coordinator (2026-11-14). The fix is a disabled state for
  C15's secondary button, as `primary_enabled` gives the primary: a component `contract`
  change.

### #2480: the Rune Customize dialog's header has no palette icon

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2480), filed on
  2026-11-14 as the finding's reference and closed at once: its state is kept here.
- **What differs:** GTK's Customize dialog shows the palette icon in its header's icon box
  (`modal_layout(PALETTE, …)` in `src/ui/browser/customization.rs`). C15's
  `StrataDialog::icon` takes a semantic `IconName`, and strata-kit's set has no palette, so
  the Rune dialog's header has none. The body's theme-colour button draws the catalog's
  `strata-palette` by name.
- **Concerns:** [A6](packages/A6.md)'s Customize dialog. No run reads the header icon.
- **State:** accepted for now by the coordinator (2026-11-14). The fix is a `Palette` name in
  the semantic set (a W2 contract change, with an icon in every catalog) or a C15 builder
  that takes a catalog icon.

### #2494: an open pane-header popover reports `focused_pane` on GTK, `null` on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2494), filed on
  2026-11-15 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A10](packages/A10.md)'s draft `filter` v1 differential smoke run (seeds
  `0x01`, `0x05`, `0x06`, `0x0b`, `0x0c`, `0x0e` and `0x13`). While a pane header's popover or
  menu ("Choose sort field" in Icons and List, or [A5](packages/A5.md)'s "Thumbnail size" in
  Icons) is open and holds the keyboard, GTK's `app.focused_pane` reports the pane's folder
  name (`"$root"`, `"documents"`, …) and Rune's reports `null`. Once Escape or a click outside
  closes the popover, `app.focused_pane` matches again.
- **Why:** GTK's `GtkMenuButton` parents its `GtkPopover` / `GtkPopoverMenu` under the pane
  header inside the `Pane` widget, so `strata-qa`'s `is_ancestor(pane, focus)` is `true`. On
  Rune, `popover::Plain::build` mounts popovers and menus on `Frame::layer` (`Place::Overlays`),
  outside the `Mark::Pane` subtree.
- **Concerns:** `crates/strata-app/src/popover.rs`, `crates/strata-app/src/sort/`,
  `crates/strata-app/src/appearance/icon_size.rs`, and `tests/qa/fuzz/normalisation.map`
  (should the owner rule it widget ancestry rather than behaviour). No scenario run checks
  `focused_pane` while a header popover is open; `filter v2` exercises header sort controls
  without holding the popover open across steps.
- **State:** open, [A10](packages/A10.md)'s.

### #2499: Icons and List filter results populate `app.panes` on Rune, leave it empty on GTK

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2499), filed on
  2026-11-15 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A10](packages/A10.md)'s `filter v2` differential smoke run, seed `0x06` at
  step 24, shrunk from 54 steps to 6 and confirmed 3 of 3 (`Ctrl+F`, `documents`, `/`, `o`,
  `Ctrl+3`, `Down`). With `"Search results"` replacing the List or Icons listing and `"notes.txt"`
  focused, both targets report `app.focused = "notes.txt"` and
  `app.matches = ["notes.txt", "report.md"]`, and `Escape` restores the folder's `app.panes` on
  both; while `"Search results"` holds focus, GTK reports
  `app.panes = [{"focused": null, "name": "documents", "selected": []}]` and Rune reports
  `app.panes = [{"focused": "notes.txt", "name": "documents", "selected": ["notes.txt"]}]`.
- **Why:** GTK's `src/ui/inline_search/collection.rs` does not call `describe_entry_container`
  (`Mark::EntryContainer`), and the folder's own `EntryContainer` is hidden while `"Search
  results"` shows (`pane_entries` in `crates/strata-qa/src/engine/scan.rs`). On Rune,
  `crates/strata-app/src/qa/tree.rs` marks `PaneHits` with `Mark::EntryContainer` (`a22af3f08`)
  so [F3](packages/F3.md)'s `tree-view` invariant (`Presented::drawn`) checks the results'
  selection against the tree.
- **Concerns:** `src/ui/inline_search/collection.rs`, `crates/strata-app/src/qa/tree.rs`,
  `crates/strata-qa/src/engine/scan.rs`, and `fuzz-filter-v2-0000000000000006`.
- **State:** open; `v2-0000000000000006` parked in `tests/qa/fuzz/corpus/filter/parked`.

### #2507: open C21 submenus are absent from `app.menu` and `Target::MenuItem` on Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2507), filed on
  2026-11-15 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A4](packages/A4.md)'s draft `tabs` differential smoke run (seeds `0x03`, `0x07`,
  `0x08`, `0x09`, `0x0c`, `0x0f`, `0x12`, `0x13` and `0x14`). Opening `"Open in…"` inside an
  entry's context menu lists `"New Tab"` and `"New Window"` in `app.menu` on GTK and omits both
  on Rune, and `{"click":{"menu_item":"New Tab"}}` fails to locate the submenu row on Rune.
- **Why:** GTK's `src/ui/context_menu.rs` expands a submenu inline inside the single
  `GtkPopoverMenu` (`GtkRevealer`). On Rune, C21
  (`crates/strata-components/src/context_menu/view.rs:208-224`) mounts each open submenu as a
  sibling `role = "menu"` surface on `Frame::layer`, while `strata-qa`'s `open_menu`
  (`crates/strata-qa/src/engine/scan.rs:161-169`) and `Target::MenuItem`
  (`crates/strata-qa/src/engine/locate.rs:241-245`) inspect only `first_rendered(backend,
  presented, "menu")`.
- **Concerns:** `crates/strata-qa/src/engine/scan.rs`, `crates/strata-qa/src/engine/locate.rs`,
  `crates/strata-components/src/context_menu/view.rs`,
  `dialogs_and_menus/submenu-arrows-return-to-parent` (line 12), and `tabs.toml` (`Open in… →
  New Tab` left out until `strata-qa` reads sibling submenu surfaces).
- **State:** fixed in the Rune QA tree, `crates/strata-app/src/qa/submenus.rs`:
  [A13](packages/A13.md)'s [#2536](https://github.com/lgse/strata/pull/2536) moved each open
  submenu surface under its menu, and [A14](packages/A14.md)'s [#2550](https://github.com/lgse/strata/pull/2550) under the
  row that opens it, as GTK's nested popover is, so `app.menu` lists its rows right after their
  trigger in GTK's order and `Target::MenuItem` reaches them.
  `dialogs_and_menus/submenu-arrows-return-to-parent` (all three views) is `required`;
  `tabs.toml`'s `Open in… → New Tab` is [A4](packages/A4.md)'s to put back.

### #2509: the Rune sheet preview is a table of cells where GTK's GtkColumnView is named rows under a tree table

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2509), filed on
  2026-11-15 as the finding's reference and closed at once: its state is kept here.
- **What differs:** what a screen reader hears of a CSV/workbook preview. GTK's
  `GtkColumnView` (`src/ui/preview/table/grid.rs`) reports a `tree table` containing a `list`
  of `table row` nodes whose accessible names concatenate all cells in the row (`"a 1"`).
  C11b's `SheetGrid` (`crates/strata-components/src/sheet_grid/render.rs`) reports a `table`
  named `"Table"` whose `table row` children have no concatenated row name and hold a
  `row header` (1-based row number) plus `table cell` children with each cell's `label`.
- **Concerns:** `quick_preview/table-preview-keeps-secondary-sorts/columns` (lines 6, 8, 10,
  12, 14 and 16 read `child_names` of `table row` under `list` within `tree table`).
- **State:** open, carried as `parallel` under an `a11y` twin from [A15](packages/A15.md),
  which clicks the same `column header` controls and reads the ordered cell labels of C11b's
  `table`.

### #2510: inline rename in Icons and List omits the edited row from `app.matches` and `app.panes` on GTK

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2510), filed on
  2026-11-15 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A4](packages/A4.md)'s `tabs v2` differential smoke run, seed `0x0f` at step 39
  (`F2` on `"notes.txt"` in `Icons`). While the inline `"Rename"` field is open on `"notes.txt"`,
  Rune reports `app.matches = ["notes.txt", "report.md", "spreadsheet.csv", "zz-tab-b.txt"]` and
  `app.panes[0].selected = ["notes.txt"]`, whereas GTK omits `"notes.txt"` from `app.matches` and
  reports `app.panes[0].selected = []` until the rename ends.
- **Why:** `src/ui/collection_edit.rs` hides the primary name widget
  (`target.widgets.display.set_visible(false)`) while `"Rename"` is visible. In `Columns`
  (`src/ui/browser/columns/rows/widgets.rs:108`), `bind_size_visibility` hides the secondary
  `size` label during rename too, so the row has no presented `"label"` widget and
  `row_label_matches` (`src/qa/state.rs:152-171`) returns `true`. In `Icons`
  (`src/ui/icons_cell.rs:41-58`) and `List` (`src/ui/browser_modes/list_pane.rs:488-508`),
  secondary metadata `"label"` widgets (`icons-card-details` in `Icons`, and visible metadata
  columns in `List`) stay presented outside `image.parent()`, so `row_label_matches`'s
  `.or_else(...)` fallback compares a metadata label against `row.name`, returns `false`, and
  `container_rows` (`src/qa/state.rs:127`) drops the edited row.
- **Concerns:** `src/qa/state.rs`, `src/ui/icons_cell.rs`,
  `src/ui/browser_modes/list_pane.rs`, and `tabs.toml` (`rename-switch` and `rename-back` scoped
  to `Columns` until `row_label_matches` checks only the primary name widget).
- **State:** open, [H3](packages/H3.md)'s.

### #2525: `Ctrl+V` after clicking a folder in Columns pastes into the parent under the parked pointer on GTK

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2525), filed on
  2026-11-16 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A12](packages/A12.md)'s `clipboard v1` differential smoke run (15 of 20 seeds;
  `0x0f` shrunk from 55 steps to 4, confirmed 3 of 3 against Rune and 24 of 24 on GTK: click
  `todo.txt` in `$root`, `Ctrl+C`, click `archive` in `$root`, `Ctrl+V`). Clicking `archive`
  selects it in `$root` and opens `archive` as the active, focused child column while the mouse
  pointer remains parked over `$root`. Pressing `Ctrl+V` duplicates `todo (1).txt` into `$root`
  on GTK, whereas Rune pastes `todo.txt` into `archive/todo.txt`.
- **Why:** `BrowserView::paste_location` (`src/ui/browser/view_edit.rs:132-148`) has the same
  parked-pointer `destination_depth` fallback diagnosed and fixed for
  `BrowserView::open_terminal` in [#867](https://github.com/lgse/strata/issues/867)
  (`src/ui/browser/view_edit.rs:234-252`): `selected_entries()` reads only the newly opened empty
  `archive` column (`selected = []`), while `destination_depth()` returns `Some(0)` (`$root`)
  because `last_navigation == NavigationInput::Pointer` and `hovered_column == Some(0)`. (The
  shared `clipboard/` and `copy_conflicts/` scenarios work around it with `{"move":{"pane":"archive"}}`
  before `{"key":"ctrl+v"}`.) On Rune, `ColumnsState::set_active_quiet` sets `targeted` to the
  newly opened active column.
- **Concerns:** `src/ui/browser/view_edit.rs` (`paste_location`), and `fuzz-clipboard-v1-000000000000000f`.
- **State:** open, set aside: Rune behaves as intended, and any fix belongs on `main`.
  `v1-000000000000000f` is parked in `tests/qa/fuzz/corpus/clipboard/parked`; `clipboard v4` moves
  the pointer onto a clicked folder's opened column and pastes into `archive` via the
  selected-folder route from `$root`.

### #2547: "Run this action?" ignores `Escape` and drops focus when closed on GTK

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2547), filed on
  2026-11-16 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A14](packages/A14.md)'s `open-with` differential smoke runs. With "Run this
  action?" open for a custom action that asks to confirm, `Escape` leaves the dialog open on GTK
  (`v1` seed `0x08`, step 23), and Cancel or Close dialog closes it with focus nowhere
  (`app.focused = null`; `v2` seeds `0x01`, `0x0a` and `0x0d`). Rune dismisses it on `Escape`
  and returns focus to the entry either way.
- **Why:** `confirm_and_run` (`src/ui/actions.rs`) builds the dialog with
  `message_dialog_layout` and `modal_layer` and wires Close and Cancel, but calls neither
  `close_on_escape(&layer, ...)` nor `remember_modal_focus(&layer, &overlay)`, as every other
  modal does (`src/ui/modal/messages.rs`, `src/ui/settings/actions.rs`). C15's `StrataDialog`
  does both.
- **Also seen (2026-11-17), the same fault in Settings → Keyboard's "Reset all keybinds?":** in
  the review of [#2564](https://github.com/lgse/strata/pull/2564) ([A18](packages/A18.md)), driving
  the pinned GTK build with `./scripts/qa-drive.sh`: with the confirmation open, `Escape` leaves
  it open and focus on Reset (`app.dialog = "Reset all keybinds?"`). Cancel and Close dialog
  close it and give "Reset all" the keyboard back, as on Rune, so only the `Escape` half applies.
  `confirm_reset_all` (`src/ui/settings/keyboard.rs`) builds `message_dialog_layout` and
  `modal_layer` and never calls `close_on_escape`. Rune's confirmation closes on `Escape`
  (`crates/strata-app/src/settings_dialog/keyboard/reset_all.rs`, and the unit test
  `each_table_edits_its_own_map_and_reset_all_asks_first`).
- **Concerns:** `src/ui/actions.rs` (`confirm_and_run`), and `fuzz-open-with-v1-0000000000000008`,
  `fuzz-open-with-v2-0000000000000001`, `-000000000000000a` and `-000000000000000d`, shrunk to
  2, 4, 5 and 4 steps, each confirmed 3 of 3 GTK against Rune, and parked in
  `tests/qa/fuzz/corpus/open-with/parked`. `src/ui/settings/keyboard.rs` (`confirm_reset_all`):
  no run or seed reads it yet; A18's `settings` alphabet leaves `Escape` on that confirmation
  out.
- **State:** open, set aside: Rune behaves as intended, and any fix belongs on `main`.
  `open-with v3` leaves `Escape`, Cancel and Close dialog on "Run this action?" out and confirms
  only with Run; the custom actions' unit tests in `crates/strata-app/src/custom_actions/tests.rs`
  cover Cancel.

### #2554: "Clear finished" in the Jobs dashboard leaves focus nowhere on GTK

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2554), filed on
  2026-11-16 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A14](packages/A14.md)'s `open-with` alphabet's drafts, which left the action out
  for it, and in the review of [#2550](https://github.com/lgse/strata/pull/2550). With no job
  running or queued, "Clear finished" empties the Jobs indicator; on GTK the dashboard closes with
  focus nowhere (`app.focused = null`). Rune closed nothing and kept the popover open on "No
  background jobs." until the review; it now closes the dashboard and its button as GTK does, and
  returns focus to the file view.
- **Why:** `refresh_indicator` and the service observer in `src/ui/jobs.rs` hide the whole
  `GtkMenuButton` once `indicator_label` is empty, which unmaps the open popover holding focus,
  and nothing hands focus back. C16 keeps an emptied list open by design; the adapter
  (`close_when_emptied` in `crates/strata-app/src/custom_actions/jobs.rs`) closes it.
- **Concerns:** `src/ui/jobs.rs`, and `open-with.toml`, which leaves "Clear finished" out.
- **State:** open, set aside: Rune behaves as intended, and any fix belongs on `main`.

### #2551: Rune anchors a Shift+click at a load's cursor and at the cursor after a clear

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2551), filed on
  2026-11-16 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A9](packages/A9.md)'s `pointer` alphabet: seed `0x05` of `pointer v1` and seed
  `0x01` of `pointer v3`, both drafts and never merged, GTK against Rune. GTK's Shift+click
  (`pointer_selection`, `src/ui/collection_interaction.rs`) ranges from the anchor the shared
  browser keeps, and selects the clicked entry alone when there is none. After a folder's load
  in List or Icons (cursor on `notes.txt`, nothing selected), a Shift+click on `report.md`
  selects `[report.md]` on GTK and selected `[notes.txt, report.md]` on Rune. After a click on
  `diagram.txt`, a Shift+click on `photo.txt` and a click on the pane's empty space, a
  Shift+click on `diagram.txt` selects `[diagram.txt]` on GTK, whose empty-space press
  (`unselect_all`) and Escape (`clear_active_selection`) leave the anchor where it was, and
  selected `[diagram.txt, photo.txt]` on Rune.
- **Why:** the first visit's cursor took the anchor with it (`single/first_visit.rs`,
  `AnchorRule::Move`), `Selection::sync` gave a missing anchor the cursor on every sync of a
  listing, and `Selection::clear` moved the anchor to the cursor (`crates/strata-components/src/collection/selection/mod.rs`).
- **State:** fixed by [A9](packages/A9.md)'s closing pull request. The first visit keeps the
  anchor missing, `sync` moves only an anchor whose entry left, and `clear` keeps the anchor.
  The keyboard's Shift+arrow still starts at the cursor when nothing is selected, as GTK's
  `resume_native_selection` does, since the keyboard machine moves the anchor there first
  (`keyboard::tests`). Held by `qa::tests::clicks::a_shift_click_after_a_load_*` and
  `a_shift_click_after_a_background_click_*` in List and Icons, and by
  `collection::selection::tests::clear_keeps_the_anchor_for_the_next_shift_click`.

### #2552: after a marquee, GTK's browser cursor is the band's end while its focus stays where it was

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2552), filed on
  2026-11-16 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A9](packages/A9.md)'s `pointer` alphabet, GTK against Rune. GTK's selection
  sync (`SelectionSync::changed`, `src/ui/browser_modes/selection.rs`) gives the shared browser,
  as its cursor, the last entry each selection change newly selected. During a band, that is the
  entry the band reached last, while the list's own focus, which GTK's snapshot reports, stays on
  the row it was on. What reads the browser's cursor afterwards then starts from the band's end:
  a return to the folder (click `pictures`, click `photo.txt`, a marquee from empty space up to
  `diagram.txt`, Alt+Up, click `pictures`: GTK focuses `diagram.txt`, Rune `photo.txt`), a
  reload (a marquee from empty space up to `readme.md` with `todo.txt` focused, then a file
  removed: GTK refocuses `readme.md`), and the next arrow (GTK's `j` selected the focused
  `notes.txt` in place where Rune moved on to `report.md`). Rune keeps its cursor on the
  focused row.
- **Concerns:** `fuzz-pointer-v3-000000000000000c` and `-0d`, each shrunk to 5 steps, confirmed
  3 of 3 against Rune and agreeing with itself in 24 of 24 GTK replays, parked in
  `tests/qa/fuzz/corpus/pointer/`; the arrow cases, seeds `0x0a`, `0x0e` and `0x14` of
  `pointer v1`, never merged. Not driven by hand on GTK.
- **State:** open, set aside: no Rune change. `pointer v3` sends no keyboard cursor moves, and
  any fix belongs on `main`.

### #2546: after a drag-move out of the folder, GTK gives the keyboard to the neighbouring row, Rune to none

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2546), filed on
  2026-11-16 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A19](packages/A19.md)'s survey of
  `cross_volume_drop/cross-volume-enter-choice/{columns,icons,list}-move` on Rune. With
  `todo.txt` selected and holding the keyboard, a drag onto `drop-volume` and the "Copy or
  move?" prompt's Move take the file out of the shown folder; line 25 waits for
  `focused: "readme.md"` without a key. Driven on GTK (`qa-drive.sh`, List, 2026-11-16): the
  focus ring is on `readme.md`, nothing is selected, and the next Down acts from `readme.md`.
  On Rune no row has the keyboard (`focused: null`).
- **Why:** a drag-move out is one of the shared `Browser`'s selectionless removals
  (`set_selectionless_removals`), so it places no cursor; GTK's `GtkListBase` moves its
  widget focus to the neighbour anyway. Rune's views let go of a cursor whose entry left a
  complete listing (`handover::sync_rule`, `CursorGone::Clear`, as C1e documents for a plain
  move out). It is not `main`'s bug [#2361](https://github.com/lgse/strata/issues/2361): GTK
  draws the cursor where its snapshot says.
- **Concerns:** `crates/strata-app/src/handover.rs` and
  `crates/strata-components/src/collection/selection/mod.rs` (`CursorGone`), the shared
  handover contract. Giving the neighbour the keyboard unselected must not hand the browser a
  cursor GTK's browser never gets: in Columns a browser cursor on a folder opens it. Any
  other move out of the shown folder (a cut and paste, A12's) likely shows the same.
- **State:** fixed by [A19](packages/A19.md)'s
  [#2571](https://github.com/lgse/strata/pull/2571), as the owner ruled on 2026-11-16
  ([`owner.md`](owner.md#ruled)): after a selectionless removal the view keeps the keyboard on
  the neighbour, unselected, without handing the browser a cursor (`handover::kept_neighbour`
  and `handover::Kept`). The three runs are `required`. Held by
  `columns::tests::moved`, `handover::tests::a_move_out_leaves_the_keyboard_on_the_neighbour_*`
  and `qa::tests::invariants::*_after_a_move_out`.

### #2557: the Rune still-image preview names its picture for the file where GTK's is unnamed

- **Code:** `deviation`, ruled by the owner on 2026-11-16 ([`owner.md`](owner.md#ruled)). [Issue](https://github.com/lgse/strata/issues/2557),
  filed on 2026-11-16 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A15](packages/A15.md)'s Slice 4, comparing the settled `app.preview` of a
  previewed `pic.png` on both targets. GTK's picture (`render_sandboxed_media`,
  `src/ui/preview/media_render.rs`) has no accessible name: `rasterized_label`
  (`src/ui/preview/render.rs`) names only covers and models. Rune's C14b `ImageView` is an
  `image` named for the file (`image_label`, `crates/strata-app/src/preview/image.rs`), with a
  drawn caption of the name and dimensions that assistive technology does not read.
- **Why:** C14b's work package asks for the name and dimensions
  (`docs/rune/work/phase-3-views/C14b-viewers.md`; `docs/rune/work/STATUS.md`, "image caption
  new"), a coordinator's choice, not an owner's ruling.
- **Concerns:** A15's `preview` fuzz alphabet, which leaves non-cover still images out.
- **State:** ruled: Rune keeps the name and the caption (the owner, 2026-11-16). A run or fuzz
  step that reads it takes a `deviation` twin or a cited rule.

### #2558: the Rune text and document previews name their body and split the text where GTK reads one buffer string

- **Code:** `deviation` for the two names, ruled by the owner on 2026-11-16
  ([`owner.md`](owner.md#ruled)); the line split stays a `divergence` for the QA backend. [Issue](https://github.com/lgse/strata/issues/2558), filed on
  2026-11-16 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A15](packages/A15.md)'s Slice 4, comparing the settled `app.preview` on both
  targets. For `notes.txt` GTK's `GtkSourceView` reads its buffer as one string, newlines
  included (`"the quick brown fox\n"`); Rune's C10b `CodeView` is a `document` named
  `"Source"` followed by each line's text. For a rendered `page.md` both read the blocks
  (`"Heading"`, `"Body text."`), and Rune's C11 `DocumentView` adds a `document` named
  `"Document"` before them. The names come from `labels::code` and `labels::document`
  (`crates/strata-app/src/preview/labels.rs`).
- **Concerns:** A15's `preview` fuzz alphabet, which leaves text and document bodies out while
  open at step boundaries; `quick_preview/rendered-document-word-selection-rune`, which finds
  the rendered document by its `"Document"` name.
- **State:** the names are ruled: Rune keeps `"Source"` and `"Document"` (the owner,
  2026-11-16). Open for the line split, which the QA backend may absorb by reading a code
  view's lines as one string.

### #2559: GTK logs a `gtk_orientable_get_orientation` critical after a previewed tar is extracted

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2559), filed on
  2026-11-16 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A13](packages/A13.md)'s `archives` alphabet, GTK calibration. In Columns, the
  sequence compresses `todo.txt` into `packed.tar`, enters its preview with `l`, then extracts
  it with `Extract here`, which selects `todo (2).txt`. A move with `k` to `readme.md` then
  makes GTK log `Gtk-CRITICAL **: gtk_orientable_get_orientation: assertion
  'GTK_IS_ORIENTABLE (orientable)' failed` while the preview switches to the Markdown file.
  It needs all four parts, and about 700 ms between steps when run as a scenario.
- **Concerns:** `fuzz-archives-v1-000000000000000d`, shrunk on GTK from 76 steps to 12 and
  confirmed 3 of 3, parked in `tests/qa/fuzz/corpus/archives/`. Rune logs no GLib warnings,
  and its replay agrees with GTK on everything else.
- **State:** open, set aside: no Rune change. `archives` v4 keeps the drawer closed, for
  [#2560](#2560-the-preview-drawers-accessible-tree-differs-between-gtk-and-rune)'s reasons, so it
  no longer reaches the sequence. Any fix belongs on `main`.

### #2560: the preview drawer's accessible tree differs between GTK and Rune

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2560), filed on
  2026-11-16 as the finding's reference and closed at once: its state is kept here.
- **What differs:** `app.preview`, the names and texts of every node in the open drawer. The
  drawer reads the same on both targets, and the scenario predicates are not affected. Two of
  its parts are [A15](packages/A15.md)'s, two [A13](packages/A13.md)'s:
  - *The header (A15's), fixed:* GTK exposes the `·` separators as labels, and Rune drew them
    `a11y_hidden` (`crates/strata-components/src/preview_drawer/header.rs`). A15's
    [#2548](https://github.com/lgse/strata/pull/2548) exposes them.
  - *A text preview (A15's):* GTK has one label with the file's text, and Rune has `Source`
    and the text's lines. That is
    [#2558](#2558-the-rune-text-and-document-previews-name-their-body-and-split-the-text-where-gtk-reads-one-buffer-string)'s,
    and A15's `preview` alphabet leaves text bodies out for it.
  - *The archive member rows and crumbs (A13's), fixed in [#2561](https://github.com/lgse/strata/pull/2561):*
    GTK's row has no name and holds a name label and a size label. Rune names the option for
    screen readers and draws its size `a11y_hidden`, as the row's description. GTK's crumb
    `GtkButton` holds a `GtkLabel`, and Rune's button is one node. In the Rune QA backend
    (`crates/strata-app/src/qa/tree.rs`), the size is now keyed as a row's badge and presented
    under the row as the name is, a member row is presented without its own name, and a crumb
    holds a label with its text. `archives` v5 opens the archive tree, and it agrees with GTK
    there.
  - *The password prompt (A13's), half fixed:* GTK's `GtkButton` holds a `GtkLabel`, now
    presented under Rune's `Unlock` too. GTK's `GtkPasswordEntry` holds an inner `GtkText`, and
    both are named by the placeholder, so GTK reads `Password` twice and Rune once. Presenting
    that inner node would add a second field-like node under every secure field the backend
    targets, so it is [A13](packages/A13.md#open)'s open item, and `archives` never opens the
    prompt.
- **Concerns:** 12 of 20 smoke seeds of `archives` v1 (never merged), each first differing
  there on a preview opened by `Space`, `Right` or `l`, and seed `0x07` of v3 by `Right` on
  `packed (1).tar`.
  - *The PDF (A16's), a ruled `deviation`:* GTK presents the page as one `image`, "PDF page
    text". Rune's C12 `PdfView` adds the document's name, "PDF document"
    (`PdfLabels::document`), each page's, "Page 1 of 1" (`PdfLabels::page`), and the page's
    text layer, its text as one string ("Hello PDF text layer\nsecond line here" for
    `tests/fixtures/documents/text.pdf`). Seen in A16's `media` fuzz alphabet, version 2, on
    every PDF opened (`app.preview`). The owner ruled on 2026-11-19 that Rune keeps the three,
    as for #2557, #2558 and #2606 ([`owner.md`](owner.md#ruled)). The fuzz map's `rune_adds`
    rule names exactly those three, and the `media` alphabet, version 7, opens PDFs again.
- **State:** open. A13's rows, crumbs and `Unlock` are fixed. Its password field is open, in
  [`A13.md`](packages/A13.md#open). The header is fixed by #2548, and the text preview is
  #2558's. The PDF's names are a ruled `deviation`, 2026-11-19.
  The same run found one visible difference, which #2561 fixed: until an archive opens, the
  header named its kind `ZIP` (`extension_label`, `crates/strata-app/src/preview/load.rs`)
  where GTK writes the extension as it is, `zip`. `archives` v5 opens the archive tree, and
  keeps the drawer closed on text files, folders and the password prompt. The nine
  `archive_preview` runs hold the prompt.

### #2562: `keyboard-open-selects-first-child/columns` did not open `documents` once in a full Rune run

- **Code:** none of the plan's: a flaky Rune run, labelled `testing`.
  [Issue](https://github.com/lgse/strata/issues/2562), filed on 2026-11-16 as the finding's
  reference and closed at once: its state is kept here.
- **Seen:** in a full `./scripts/qa-parity.sh all` for
  [#2561](https://github.com/lgse/strata/pull/2561)'s gate at `60c062c9a`, on a loaded host
  (rune offscreen 617 of 618). The run failed at line 4: after `select_with_keyboard` reached
  `documents` and Return was sent, the browser still worked in the fixture's root after
  15 s. Alone, it passed 3 of 3 on both targets right afterwards, and those rounds overwrote the
  failing run's artifacts. #2561 changes nothing in Columns or the keyboard.
- **Concerns:** `click_modes/keyboard-open-selects-first-child/columns`, `required`,
  [A9](packages/A9.md)'s.
- **Resembles:** [#2322](#2322-scrolled-away-crumbs-and-columns-stay-reachablecolumns-left-focus-nowhere-once-in-a-full-rune-run)
  (a Rune Columns keyboard step left undone after 15 s in a full run) and
  [#2162](#2162-new-folder-with-selection-groupscolumns-is-flaky) on GTK. Not traced.
- **State:** open, unowned. The ledger is unchanged: the run stays `required`.

### #2563: `drag-file-onto-folder/columns-copy` did not show the `archive` row's count once in a full Rune run

- **Code:** none of the plan's: a flaky Rune run, labelled `testing`.
  [Issue](https://github.com/lgse/strata/issues/2563), filed on 2026-11-16 as the finding's
  reference and closed at once: its state is kept here.
- **Seen:** in the rerun of `./scripts/qa-parity.sh all` for
  [#2561](https://github.com/lgse/strata/pull/2561)'s gate at `fa4c1e64a`, on a loaded host
  (rune offscreen 617 of 618). It failed at line 2, its first wait: the `archive` folder row's
  `No items` label did not show within 15 s, and the kept tree has the row with its name label
  alone, after two loads of the folder and before any drag. The same run passed in the set's
  first round. #2561 changes nothing in the listing, item counts or drag and drop.
- **Seen again:** as `drag-file-onto-folder/list-copy`, in the re-gate of
  [#2566](https://github.com/lgse/strata/pull/2566) at `fe1b8a077` (rune offscreen 642 of
  643): the same first wait, the `archive` row's `No items` label, timed out after 15 s. The
  scenario passed alone three times in a row (6 of 6 runs each). #2566 changes the portal
  chooser and leaves the browser's listing, item counts and drag and drop as they were.
- **Seen again:** as `drag-file-onto-folder/list-move`, in round 3 of
  [#2571](https://github.com/lgse/strata/pull/2571)'s three rounds of `drag_and_drop` at
  `815298327`: the same first wait, the `archive` row's `No items` label, timed out after
  15 s. The run passed in that pull request's `qa-parity.sh all`. #2571 changes the views'
  keyboard handover after a move out of the folder, not the listing or its item counts.
- **Seen in [#2569](https://github.com/lgse/strata/pull/2569)'s gate too:** as
  `drag-file-onto-folder/list-move`, in its review round at `f14ea347d` (rune offscreen 663 of
  664), at the same first wait. The scenario then passed alone three times in a row (6 of 6 runs
  each). #2569 adds the window's zoom wheel and the PDF preview, and leaves the listing, item
  counts and drag and drop as they were.
- **Seen in [#2583](https://github.com/lgse/strata/pull/2583)'s gate too:** as
  `drag-file-onto-folder/list-copy`, in its re-gate at `32b544323` (rune offscreen 673 of 675),
  at the same first wait. The scenario then passed alone three times in a row (6 of 6 runs
  each). #2583 changes how a drop from another application is read, not the listing or its item
  counts, and this run's drags come from the window itself.
- **Seen in [#2594](https://github.com/lgse/strata/pull/2594)'s gate too:** as
  `drag-file-onto-folder/columns-copy`, in its `qa-parity.sh all` at `e631cad2f` (rune
  offscreen 681 of 682), at the same first wait. The scenario then passed alone (6 of 6 runs).
  #2594 shares the settings between windows and opens a window per launch, and leaves the
  listing, item counts and drag and drop as they were.
- **Concerns:** `drag_and_drop/drag-file-onto-folder/columns-copy`, `list-copy` and `list-move`,
  `required`, [A19](packages/A19.md)'s.
- **Seen again:** as `drag-file-onto-folder/list-copy`, in the re-gate of
  [#2602](https://github.com/lgse/strata/pull/2602) at `5d50d32e8` (rune offscreen 704 of 705):
  the same first wait timed out. Alone it then passed 6 of 6 on each target. #2602 changes
  strata-qa's dialog lookup, not the listing, item counts or drag and drop.
- **State:** open, unowned. The ledger is unchanged: the run stays `required`. Not traced.

### #2568: `undo-restores-completed-move/columns` did not put the file back once in a full Rune run

- **Code:** none of the plan's: a flaky required Rune run, labelled `testing`.
  [Issue](https://github.com/lgse/strata/issues/2568), filed on 2026-11-16 as the finding's
  reference and closed at once: its state is kept here.
- **Seen:** in [A16](packages/A16.md)'s gate for its first pull request,
  `./scripts/qa-parity.sh all` with `strata-next` at `a5563b5a2` merged in. On Rune the run
  failed at line 11: after Ctrl+X on `todo.txt`, Ctrl+V into `archive` and
  `archive/todo.txt` on disk (line 10), Ctrl+Z left `todo.txt` absent for the step's wait.
  Every other run of it passed: the gate before the merge, and the run alone on Rune 3 of 3.
  The solo rounds overwrote the failing run's artifacts.
- **Concerns:** `entry_management/undo-restores-completed-move/columns`, [A7](packages/A7.md)'s,
  `required`. The branch mounts the PDF preview, and its QA backend change leaves a PDF row's
  zoom generation out of node paths. The run opens no preview, and its fixture holds no PDF.
- **Not traced.** One reading to check: line 10 waits only for the moved file on disk, so under
  load Ctrl+Z can arrive before the move is recorded as the operation to undo.
- **Also seen** in the gate of [#2573](https://github.com/lgse/strata/pull/2573) (A18): once in
  a full Rune run on clean `strata-next` at `1370c6f7b`, and once in the re-gate at `cad74ce7e`
  (rune offscreen 674 of 675). Alone it then passed on both targets.
- **Also seen** in the re-gate of [#2583](https://github.com/lgse/strata/pull/2583) at
  `32b544323` (rune offscreen 673 of 675), at line 11 again. Alone on Rune it then passed three
  times in a row. #2583 changes drops from other applications; this run cuts, pastes and undoes
  with keys.
- **Seen again** in [A16](packages/A16.md)'s [#2585](https://github.com/lgse/strata/pull/2585)
  review gate at `6501200fd`, at the same line (Rune offscreen 676 of 677). Alone it then passed
  3 of 3. The pull request changes the preview's viewers and the media keys' guard, not undo.
- **Also seen** in the re-gate of [#2597](https://github.com/lgse/strata/pull/2597) at
  `8c69f50b3` (rune offscreen 683 of 684), at line 11 again. Alone it then passed three
  rounds, 1 of 1 on each target. #2597 changes the view's cursor after an undo (A19's
  `handover::Kept`), not what reaches the disk, which is what line 11 waits on.
- **Seen again** in [A16](packages/A16.md)'s Slice 4 gate ([#2614](https://github.com/lgse/strata/pull/2614))
  at `432f809b9`, at the same line (Rune offscreen 707 of 708). Alone it then passed 3 of 3.
  The pull request changes the preview's media player, C13's header and the fuzz runner's
  rules, not undo.
- **State:** open, unowned.

### #2570: 10xer's find and filter highlights are not drawn in Rune's rows

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2570), filed on
  2026-11-16 as the finding's reference and closed at once: its state is kept here.
- **What differs:** in 10xer mode GTK draws the matched ranges of each displayed name in the
  accent: a `/` or `?` find's until Escape dismisses them (`src/ui/browser/find.rs`:
  `find_highlight`, `refresh_name_highlights`), and an `f` filter's terms and a search's hit
  ranges (`highlight_listing_name`, `highlight_hit_name`). Rune draws none. Since
  [#2567](https://github.com/lgse/strata/pull/2567) the find's highlight is kept as state
  (`FindState::highlighted`, `crates/strata-app/src/tenxer/prompts/find.rs`) and the Escape
  ladder dismisses it before the selection, as GTK's does (`src/ui/window/keyboard/escape.rs`),
  so only the look differs.
- **Why:** the rows of C1, C3 and the Columns view take a name as plain text
  (`strata_components::contract::Entry::name`, `RowMarks`). Drawing ranges needs a row
  contract field and spans in all three views' name cells, coloured from the Kit's tokens.
- **Concerns:** no ledger run; found in the review of #2567. [A11](packages/A11.md)'s fuzz
  alphabet (Slice 5) should exercise `/` then Escape with a selection.
- **State:** fixed by [A11](packages/A11.md)'s `contract` change for #2570,
  [#2582](https://github.com/lgse/strata/pull/2582). `RowMarks::name_matches` carries a name's matched
  byte ranges, which C1e's default label (the Columns view's names), C3's caption and C4's name
  cell paint in the Kit's accent. The app fills them as GTK's `refresh_name_highlights` does
  (`crates/strata-app/src/tenxer/highlight.rs`): a highlighted 10xer find marks its query,
  case folded, in every name, the listing's and the results', until Escape or the find
  prompt's Escape dismisses it, and it goes with 10xer mode. Without one, a fuzzy `f` or `s`
  marks what its query matched in each result (`PathMatcher::name_highlight`, GTK's
  `search_hit_ranges`; for a direct child the same ranges as GTK's in-place
  `filter_term_ranges`). Covered by `tenxer::highlight::tests` (Columns, List and Icons) and
  the component's `collection::element::matches::tests`.

### #2575: `preview-blank-click-keeps-selection/columns` was still loading its preview once in a full Rune run

- **Code:** none of the plan's: a flaky Rune run, labelled `testing`.
  [Issue](https://github.com/lgse/strata/issues/2575), filed on 2026-11-17 as the finding's
  reference and closed at once: its state is kept here.
- **Seen:** in the re-gate of [#2571](https://github.com/lgse/strata/pull/2571) at
  `fc58473eb`, `./scripts/qa-parity.sh all` on a loaded host (rune offscreen 661 of 662). At
  line 6 the preview still showed its "Loading…" bar. The scenario passed 5 of 5 alone
  afterwards. #2571 changes the views' keyboard handover after a move out of the folder, not
  the preview's loading path.
- **Concerns:** `background_selection/preview-blank-click-keeps-selection/columns`,
  `required`, [A15](packages/A15.md)'s (done).
- **Also seen** in the gate of [#2573](https://github.com/lgse/strata/pull/2573) (A18), in the
  rerun of `./scripts/qa-parity.sh all`: `preview-blank-click-keeps-selection/columns` and
  `/icons` both showed "Loading…" at line 6 (rune offscreen 661 of 663). Both passed in the run
  before. #2573 changes Settings and the QA snapshot's nested dialogs, not the preview.
- **State:** open, unowned. The ledger is unchanged: the run stays `required`. Not traced.

### #2600: the floating chooser's second right-click opened no menu in a busier `window::` suite

- **Code:** none of the plan's: a flaky Rune window test, labelled `testing`.
  [Issue](https://github.com/lgse/strata/issues/2600), filed on 2026-11-18 as the finding's
  reference and closed at once: its state is kept here.
- **Seen:** on the head of [#2594](https://github.com/lgse/strata/pull/2594) (A21's first
  slice), in `mise run test-platform window::`, whose suite there has 9 tests where
  `strata-next` has 6. Its review saw 2 failures in 27 runs, and its author 3 in 10, then 2 in
  10 with the floating chooser left out of the windowed host's `wake_pending`. Each time
  `window::qa::the_floating_chooser_comes_forward_and_closes_with_its_browser`
  (`crates/strata-rune/tests/window/qa.rs`) failed at the second `ask_to_copy`, after
  `s.focus(&browser)`: the right-click on `notes.txt` reached the browser (the session host
  aimed it from the browser's sway area, 0,0 640x800, and the compositor delivered the press),
  but no entry menu opened, and `{"wait":{"menu_items":{}}}` timed out with `"menu":null`.
  The failing suite took about 19 s against about 4 s.
- **Not reproduced:** alone, 0 of 12 (the review); on `strata-next`, 0 of 12 (the review); and
  on #2594's own code with its three new window tests skipped, the 6 tests `strata-next` has,
  0 of 10. So it tracks the three extra sway sessions running beside it, not the code in its
  path. Not traced further.
- **Seen again** by the review of [#2604](https://github.com/lgse/strata/pull/2604) (A21's Slice
  2a) at `dbf11553d`: 1 failure in 3 runs of `window::`, whose suite there has 9 tests. It
  failed at `qa.rs:53`, the second `ask_to_copy`'s `menu_items` wait, the same symptom, and the
  failing suite took 20 s against about 4 s.
- **Concerns:** `strata-rune`'s window test above, [A20](packages/A20.md)'s.
- **State:** open, [A20](packages/A20.md)'s to watch (its "Open", 2026-11-19).

### #2580: `rendered-document-word-selection-rune` lost the selection's first characters twice in a full session-host run

- **Code:** none of the plan's: a flaky Rune run, labelled `testing`.
  [Issue](https://github.com/lgse/strata/issues/2580), filed on 2026-11-17 as the finding's
  reference and closed at once: its state is kept here.
- **Seen:** in the re-gate of [#2569](https://github.com/lgse/strata/pull/2569) at `0914908a8`,
  `./scripts/qa-parity.sh all` with `strata-next` at `5e6c05568` merged in. The session-host
  batch now has 33 runs, since A19's content drops (#2577), and two of them failed (31 of 33).
  `columns-docx` failed at line 21: the clipboard held `uarterly Report`. `columns-md` failed
  at line 20: it held `pha bravo charlie delta`. Each selection started a character or two past
  the drag's start. Alone on the session host the twin then passed four rounds, 12 of 12.
- **Sighting on [#2579](https://github.com/lgse/strata/pull/2579):** the same two runs at the
  same lines, `columns-docx` holding `uarterly Report` and `columns-md` holding
  `pha bravo charlie delta`, in `./scripts/qa-parity.sh all` at `2bd56840c` (31 of 33 on the
  session host; gtk 705 of 705, rune 672 of 672). Alone on the session host the twin passed
  three rounds, 9 of 9. Again after merging `44a600075`, at `9a8df7673`: `columns-docx` alone,
  `uarterly Report` (34 of 35 on the session host; gtk 715 of 715, rune 680 of 680), then 9 of 9
  alone. And after merging `5c107d947`, at `193c69af6`: both runs as at first (33 of 35 on the
  session host; gtk 715 of 715, rune 680 of 680), then 9 of 9 alone. #2579 changes no pointer
  drag or document selection.
- **Concerns:** `quick_preview/rendered-document-word-selection-rune`, [A15](packages/A15.md)'s
  `parallel` twin. #2569 changes no pointer drag or document selection.
- **Sighting on [#2577](https://github.com/lgse/strata/pull/2577):** in its gate at
  `1ba2778c1909325066429fdf368128030d2ddea1`, `./scripts/qa-parity.sh all`: the session host
  passed 31 of 32, and `columns-docx` failed at line 21 with `uarterly Report`, as above. Alone
  it then passed three rounds, 3 of 3 each (`md`, `rtf`, `docx`). That run's selection comes
  from `click` steps (a double click at line 14, a click and a Shift-click at lines 17 and 18),
  each a press and release at one point, with no drag between them.
- **Sighting on [#2578](https://github.com/lgse/strata/pull/2578):** in its re-gate at
  `32db8aa4d`, the full session-host batch (35 runs) failed all three of the twin's runs,
  `columns-md` and `columns-rtf` at line 20 (`pha bravo charlie delta`) and `columns-docx` at
  line 21 (`uarterly Report`); alone on the session host it then passed three rounds, 9 of 9.
  #2578 changes no pointer drag or document selection.
- **Sighting in [A16](packages/A16.md)'s Slice 2 gate** at `0d38d2eb8`, before #2584 merged:
  the same three runs at the same lines, in the full 35-run session batch (32 of 35). Alone on
  the session host the twin then passed three rounds, 9 of 9.
- **Sighting on [#2582](https://github.com/lgse/strata/pull/2582)** (at `5a37f26d0`): the
  same three runs failed in the full session-host batch, with the same messages; alone on the
  session host the twin then passed three rounds, 9 of 9. The change draws name highlights
  only.
- **Also seen** in the gate of [#2573](https://github.com/lgse/strata/pull/2573) (A18): in two
  full runs (`columns-md` and `columns-docx`, then `columns-docx`), and once in two full Rune
  runs on clean `strata-next` at `1370c6f7b`, and in the re-gate at `cad74ce7e` (`columns-docx`,
  session host 34 of 35). Alone it passed each time on both targets. Filed again at the same
  time as [#2581](https://github.com/lgse/strata/issues/2581), a duplicate.
- **Sighting on [#2583](https://github.com/lgse/strata/pull/2583):** in its gate at
  `90a405bbc`, the session-host batch passed 33 of 35: `columns-md` failed at line 20
  (`pha bravo charlie delta`) and `columns-docx` at line 21 (`uarterly Report`). Alone on the
  session host the twin then passed three rounds, 9 of 9. #2583's session-host change paces
  only motions after a press outside the window; these clicks are inside it.
- **Cause** (traced by [A15](packages/A15.md), 2026-11-17): the twin, not Rune. Line 17 of
  `rendered-document-word-selection-rune` clicked 17 points into C11's first text `label`, where
  GTK's original clicks 17 points into the row's `panel`, whose padding puts the point at the
  text's start. On the label, 17 points is two glyphs into `alpha` (one into `Quarterly`), so a
  plain click there puts the caret at `al|pha`, and line 18's Shift+click selects
  `pha bravo charlie delta`. The run passed only while that click came within Rune's
  double-click interval (`Runtime::double_click_interval`, 0.5 s) and `CLICK_SLOP` (4 points) of
  line 14's double-click at the same point: a third press there selects the paragraph, from its
  start. Ctrl+C and the clipboard wait between them take longer than 0.5 s under load. Shown both
  ways with a 700 ms `sleep` before line 17: all three Rune runs failed with the reported text,
  while GTK's original passed 3 of 3 with the same sleep, its click landing at the text's start.
  Under 24 busy loops (load average 21 to 39), the old twin passed 1 of 9.
- **Fix:** line 17 clicks 1 point into the label, the text's start, as GTK's click lands; 16
  points from the double-click, it is a single click whatever the timing. What the twin checks is
  unchanged. With the 700 ms sleep the fixed twin passed 3 of 3; under the same load it passed
  10 rounds in a row, 30 of 30, and two full `./scripts/qa-parity.sh all` runs had no failure of
  it ([#2584](https://github.com/lgse/strata/pull/2584)).
- **State:** fixed in the twin. #2581 is its duplicate.

### #2572: Settings is a named dialog with menus on Rune and unnamed panels on GTK

- **Code:** `deviation`, ruled by the owner on 2026-11-17 ([`owner.md`](owner.md#ruled)). [Issue](https://github.com/lgse/strata/issues/2572), filed on
  2026-11-17 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A18](packages/A18.md)'s `settings` alphabet's smoke tier (v3), in all 20
  sequences. It has three facets of `app.*`, all from one cause:
  - While Settings is open, `app.dialog` reads `null` on GTK and `"Settings"` on Rune.
  - While a choice menu ("Modified date format") is open, `app.menu` reads `null` on GTK and
    `[]` on Rune (63 steps).
  - While "Reset all keybinds?" is open, `app.dialog` reads `"Reset all keybinds?"` on GTK and
    `"Settings"` on Rune, since strata-qa's `open_dialog` names the first rendered dialog.

  With `app.dialog` and `app.menu` set aside, all 20 sequences agree at every step: focus,
  panes, the text field, `settings.toml` and the tree. The shrunk sequence, one step that opens
  Settings from the header, confirmed 3 of 3:
  `tests/qa/fuzz/corpus/settings/fuzz-settings-v3-0000000000000001.jsonl`, unparked once ruled.
- **Why:**
  - **GTK.** Its Settings is a `gtk::Box` overlaid on the browser and kept between openings
    (`build_layer` in `src/ui/settings.rs`, added by `SettingsLauncher` in
    `src/ui/window/composition/settings.rs`), with no accessible role or name. Its choice
    menus are plain `gtk::Popover`s (`choice_menu` in `src/ui/settings/bindings.rs`), role
    `generic`, holding `MenuItemRadio` rows.
  - **Rune.** Its Settings is C15's `DialogShell`, a `dialog` named "Settings", and its choice
    menus are `menu`s owning their `menuitemradio` rows (`settings_dialog/controls.rs`).

  A screen reader hears "Settings, dialog" and a menu on Rune, and unnamed panels on GTK. GTK's
  own message layers inside Settings, such as "Reset all keybinds?", do report as dialogs. It
  is the pattern of [#2405](https://github.com/lgse/strata/issues/2405), whose ruling covers
  plain popovers only. [A9](packages/A9.md)'s `pointer` drafts met the first facet and read
  GTK's Settings as a window of its own; it is a layer in the browser's window.
- **Concerns:** the `settings` alphabet, and any alphabet that opens Settings.
- **Options for the owner:**
  1. Rune keeps the named dialog and the menus, and the normalisation map gains rules that name
     their values. For `app.dialog`: GTK's `null`, or a message layer's name such as "Reset all
     keybinds?", against Rune's "Settings". For `app.menu`: GTK's `null` against Rune's `[]`.
     The map takes one rule for each field and code, so a `deviation` rule's values go on
     #2405's line; an `a11y` code needs a line of its own. Trial, not committed: with the
     first two value pairs, 9 of 20 v3 sequences agree, and the other 11 differ only at the
     third facet.
  2. Rune's Settings drops to an unnamed group and its menus to plain groups, matching GTK's
     AT-SPI. Assistive technology then loses "Settings" and the menus' roles.
  3. The QA snapshot names the innermost open dialog rather than the first. That is a change
     to strata-qa for both targets, and it leaves the first two facets.
- **State:** ruled by the owner, 2026-11-17: option 1. Rune keeps the named dialog and the
  menus, and `tests/qa/fuzz/normalisation.map` names exactly their values:
  - `app.dialog deviation lgse/strata#2572 {"gtk":null,"rune":["Settings"]}`;
  - `app.menu deviation lgse/strata#2572 {"gtk":null,"rune":[[]]}`.

  The map takes a rule of one code for each ruling on a field, each citing its own issue
  (`tests/qa/fuzz/rules.rs`), so the first sits beside #2405's. The third facet is fixed in the
  QA backend, by the owner's second ruling of 2026-11-17: Rune's snapshot names the topmost
  open dialog, the message drawn inside Settings, as GTK's does (`Backend::nests_dialogs` in
  `crates/strata-qa`, on for the Rune backend only), with no map rule. Under the two rules the
  `settings` alphabet's v5 smoke tier, Reset all included, agrees in 20 of 20, and the unparked
  v3 sequence replays clean. [A18](packages/A18.md)'s.

### #2588: Copy to or Move to that starts no transfer leaves its destination to navigate to on GTK

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2588), filed on 2026-11-17
  as the finding's reference and closed at once: its state is kept here.
- **Seen:** by reading the source, in the review of [#2579](https://github.com/lgse/strata/pull/2579)
  ([A20](packages/A20.md)); not reproduced by a run. `finish_copy_move_to`
  (`src/ui/browser/transfer/destination_dialog.rs`) sets `pending_navigate`, and adds the
  sources' names to `pending_select`, before `begin_transfer` (`src/ui/browser/transfer.rs`).
  That can return with nothing started: the request is refused, every source is a no-op (Move
  to… the folder the items are in), or the same transfer is already preparing. Nothing then
  clears the destination; only a reveal, a failure, a cancellation or a move to the dock does.
  A later foreground transfer that reveals nothing, such as a drop with "Open folder after drop"
  off, navigates there when it completes (`complete_transfer_ui`, `src/ui/browser/events/reveal.rs`).
- **Concerns:** `transfer::transfer_to_chosen` (`crates/strata-app/src/transfer/chosen.rs`),
  which ports `pending_navigate` as GTK has it, early returns included. No run or seed reads it.
- **State:** open, set aside: Rune keeps GTK's behaviour. Any fix belongs on `main`, and Rune
  follows it.

### #2586: after 10xer's `f` filter commits, GTK's listing has no cursor and Space selects nothing

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2586), filed on 2026-11-17
  as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A11](packages/A11.md)'s `tenxer` alphabet (v2, never merged), shrunk GTK
  against Rune to three steps and confirmed 3 of 3 (`fuzz-tenxer-v2-0000000000000008`): `f`,
  `txt`, Return. GTK's snapshot names `todo.txt` as focused, but its pane reports no cursor;
  Rune's pane has its cursor on `todo.txt`.
- **Driven on GTK** (`./scripts/qa-drive.sh`, 2026-11-17, 10xer List): after the commit GTK
  draws its cursor outline on `todo.txt`, and Space selects nothing, nor does `j` then Space.
  Its in-place filter (`with_filter_terms`) gives the row the keyboard without setting the
  browser's cursor. Rune's Space selects `todo.txt`.
- **Concerns:** 10xer's `f` filter; no ledger run reads Space after a commit. The sequences
  are parked in `tests/qa/fuzz/corpus/tenxer/` (`v2-…08`, and `v2-…03`, which meets it too),
  and the alphabet abandons its filter by Escape from version 3.
- **State:** open, set aside: no Rune change. A fault for `main`, from a bug issue there.

### #2587: a 10xer move keeps a selection a revisit restored, where GTK's first move drops it

- **Code:** `oracle`, by the owner's ruling of 2026-11-17 ([`owner.md`](owner.md#ruled));
  filed as a `divergence`. [Issue](https://github.com/lgse/strata/issues/2587), filed on
  2026-11-17 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by [A11](packages/A11.md)'s `tenxer` alphabet (v3, never merged), shrunk GTK
  against Rune to 13 steps and confirmed 3 of 3 (`fuzz-tenxer-v3-0000000000000003`): Space
  twice in `pictures`, `h`, back into `pictures` by a find and `l` (both targets restore
  `[diagram.txt, photo.txt]`), then `k`. GTK's selection is then `[]`, Rune's unchanged.
- **Why:** GTK's 10xer moves go through the shared browser's `place_cursor`, which clears the
  selection while the column keeps the cursor its load placed (`load_cursor`). Rune's are C1's,
  which keep any selection not marked provisional, and `events::marks` marks only a selection
  with no fill. Without 10xer a move replaces the selection on both targets.
- **Concerns:** the views' provisional mark, which [A1](packages/A1.md) and
  [A2](packages/A2.md) own with C1, and 10xer's moves. No ledger run reads it. The sequence is
  parked in `tests/qa/fuzz/corpus/tenxer/`. The alphabet left out `h` in version 4; from
  version 5 `h`, and from version 7 `l` too, leave only a folder with nothing selected,
  so a revisit restores no selection.
- **State:** ruled by the owner, 2026-11-17: GTK's fault. A fill the user made, left and came
  back to is still the user's, and 10xer's moves keep the marks elsewhere; Rune keeps its
  behaviour. A fix for GTK, if anyone takes it up, lands on `main`.

### #2590: GTK's Ctrl+drag of a row an earlier drag selected takes it out of the selection and drags it alone

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2590), filed on
  2026-11-17 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A19](packages/A19.md)'s `drag` alphabet (v1), GTK against Rune, in List.
  `fuzz-drag-v1-0000000000000011`, shrunk to 3 steps: drag `todo.txt` onto the pane's own
  background, then Ctrl+drag `readme.md` onto `archive` (both targets add it and copy both
  files), then Ctrl+drag `readme.md` onto `pictures`. GTK copies `readme.md` alone and leaves
  `[todo.txt]` selected. Rune copies both selected files and keeps both selected.
  `fuzz-drag-v1-0000000000000005` (4 steps) shows it from a single selection: GTK's selection
  is `[]` after the Ctrl+drag.
- **Why it is GTK's:** `pointer_intent/ctrl-drag-copies-keeps-selection/*-selected-stay`
  (`required`) holds GTK to keeping the dragged row selected when a `select` step selected it.
  C1b's pointer machine says "A Ctrl-drag carries the entry: it stays selected instead of
  toggling off". Rune does so either way.
- **Concerns:** both sequences, confirmed 3 of 3, parked in `tests/qa/fuzz/corpus/drag/`.
  `drag v2` drags only rows not selected.
- **By hand on GTK** (2026-11-17, `./scripts/qa-drive.sh`, List, the sequence's preferences):
  confirmed. After the third step `pictures` holds `readme.md` alone, `archive` holds both
  files, and the selection is `[todo.txt]` with the keyboard on `readme.md`.
- **State:** open, set aside: no Rune change; any fix belongs on `main`.

### #2591: releasing a drag Escape cancelled selects the row under the pointer on GTK, its focus on the dragged row

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2591), filed on
  2026-11-17 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A19](packages/A19.md)'s `drag` alphabet (v1), GTK against Rune, in List.
  `fuzz-drag-v1-000000000000000d`, shrunk to 4 steps: drag `archive` onto itself, drag
  `readme.md` onto `archive` without releasing, Escape, release. GTK then selects `["archive"]`
  with its focus on `readme.md`. Rune keeps `["readme.md"]` selected and focused.
  `escape-cancels-file-drag` checks the selection only before the release.
- **Concerns:** the sequence, confirmed 3 of 3, parked in `tests/qa/fuzz/corpus/drag/`.
  `drag v2` leaves the Escape cancel out (`escape-cancels-file-drag` owns it).
- **By hand on GTK** (2026-11-17, `./scripts/qa-drive.sh`, List, the sequence's preferences):
  confirmed. Through Escape the selection is `[readme.md]`; the release makes it `[archive]`
  with the keyboard still on `readme.md`, and nothing moves.
- **State:** open, set aside: no Rune change; any fix belongs on `main`.

### #2592: a drag from a selected row of a multi-selection gives GTK's keyboard to that row, Rune's stays

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2592), filed on
  2026-11-17 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A19](packages/A19.md)'s `drag` alphabet (v1), GTK against Rune, in List.
  `fuzz-drag-v1-0000000000000006`, shrunk to 3 steps: click `readme.md`, Ctrl+drag `todo.txt`
  onto `documents` (both select both files, the keyboard on `todo.txt`), then drag `readme.md`
  onto the pane's own background, which takes nothing. GTK's focus is then on `readme.md`,
  Rune's still on `todo.txt`; the selections agree.
- **Where:** C1b's pointer machine (`collection/pointer/machine.rs`) kept the cursor on a plain
  press of a selected row of a multi-selection, leaving the collapse to the release, so a drag
  left it where it was. GTK's press moves the keyboard at once.
- **By hand on GTK** (2026-11-17, `./scripts/qa-drive.sh`, the sequence's preferences, in
  Columns, Icons and List): the same in all three. The keyboard is on `readme.md` after the
  drag, and a Shift+click on `archive` then selects `archive` to `readme.md`, so the press moved
  the anchor too. (Columns' press is Strata's own, `columns/rows/click.rs`, which calls
  `anchor_at`; Icons' and List's reach GTK's list-item gesture.)
- **Fix:** owned by [A19](packages/A19.md), a `contract` change to C1b: the press keeps the group
  and moves the cursor and the anchor to the entry (`AnchorRule::Move`), unless the selection
  is a provisional pick. A release still collapses the group, so A9's click modes are
  unchanged. `collection::pointer::tests::a_plain_press_inside_a_multi_selection_keeps_the_group_for_the_drag`
  holds it, and fails without it.
- **Concerns:** the sequence, confirmed 3 of 3, is unparked in `tests/qa/fuzz/corpus/drag/`, and
  agrees 6 of 6 replays GTK against Rune. `drag v2` drags only rows not selected.
- **State:** fixed on `strata-next` by [#2601](https://github.com/lgse/strata/pull/2601).

### #2593: opening again a folder that was empty gives GTK's keyboard to the entry it gained, Rune's to none

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2593), filed on
  2026-11-17 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A19](packages/A19.md)'s `drag` alphabet's smoke tier (v2), GTK against Rune, in
  List with single-click folder activation. `fuzz-drag-v2-0000000000000007`, shrunk to 4 steps:
  click `archive` (the empty folder opens, the keyboard on nothing on both), Alt+Up, drag
  `readme.md` onto `archive` (a move), click `archive`. GTK's focus is then on `readme.md` with
  nothing selected. Rune's is `null`.
- **Where:** not drag and drop's. "Open folder after drop" is off, so the transfer reveals
  nothing; the difference is the List view's restore of the folder's remembered position on
  the way back in. Not checked: whether an entry made behind the application's back does the
  same.
- **Concerns:** the sequence, confirmed 3 of 3, parked in `tests/qa/fuzz/corpus/drag/`. The
  smoke tier skips its seed while it is parked.
- **State:** open, unowned.

### #2596: `camera-formats-preview-sandbox` reads a drawn pixel no Rune target can read

- **Code:** `toolkit`. [Issue](https://github.com/lgse/strata/issues/2596), filed on 2026-11-17
  as the exception's reference and closed at once: its state is kept here.
- **What differs:** `preview_media_layout/camera-formats-preview-sandbox` checks at line 5 that a
  HEIC or AVIF picture decoded in the preview sandbox by reading a white pixel of the
  preview's last `image`. The Rune QA backend answers `pixel is not supported on this target`
  (`crates/strata-qa/src/engine/check.rs`): Rune renders no element into pixels it can hand back
  (packet entry 75), and a `Pixels` cannot be read back (anomalyco/rune#148).
- **Concerns:** the `columns-heic` and `columns-avif` runs. `columns-mov` and `columns-mp4` read no
  pixel and are `required` from the shared file.
- **State:** both runs are `parallel` under a `toolkit` twin, `camera-formats-preview-sandbox-rune`
  ([A16](packages/A16.md), [#2599](https://github.com/lgse/strata/pull/2599)), which waits for the
  preview's `image` named `camera.<ext>` and keeps the fixture, the thumbnail's `fs` pixel check
  and the end state. The twin goes once a Rune target can read drawn pixels.

### #2606: the media player's volume slider is named "Volume" where GTK's is unnamed

- **Code:** `deviation`, ruled by the owner on 2026-11-18 ([`owner.md`](owner.md#ruled)). [Issue](https://github.com/lgse/strata/issues/2606),
  filed on 2026-11-18 as the finding's reference and closed at once: its state is kept here.
- **Seen:** in [A16](packages/A16.md)'s Slice 3 ([#2599](https://github.com/lgse/strata/pull/2599)),
  which mounts C13's `MediaPlayer` in the preview drawer. GTK's volume slider
  (`volume_slider`, `src/ui/preview/media_controls.rs`) is an unnamed `gtk::Scale`. C13's is a
  `slider` named by `Labels::volume`, which A16 sets to "Volume", translated in all 13 catalogs
  (`media_labels`, `crates/strata-app/src/preview/media/mod.rs`).
- **Concerns:** no run today. A16's fuzz alphabet (Slice 4) will read the slider.
- **State:** ruled: Rune keeps the "Volume" name (the owner, 2026-11-18, through the
  coordinator), consistent with [#2557](https://github.com/lgse/strata/issues/2557) and
  [#2558](https://github.com/lgse/strata/issues/2558). A run or fuzz step that reads it cites
  the ruling.

### #2595: after the floating chooser closes, GTK's browser has no focus under the QA's Xvfb

- **Code:** `exception` (`toolkit`), ruled by the owner on 2026-11-17 and extended on 2026-11-18 ([`owner.md`](owner.md#ruled)).
  [Issue](https://github.com/lgse/strata/issues/2595), filed on 2026-11-18 as the finding's
  reference and closed at once: its state is kept here.
- **Seen:** in the differential smoke tier of [A20](packages/A20.md)'s `chooser` alphabet
  (`chooser v2`, 20 seeds of 40 actions). All 20 first part on `app.focused`,
  `app.focused_pane` and `app.panes[].focused` around the floating Copy to or Move to chooser.
  Once it has closed (Copy here, Move here or Escape), GTK reports no focus in the browser, and
  Rune reports the browser's row. GTK's QA backend reports focus only for a widget with
  `has_focus()`, which needs its window active (`src/qa/backend.rs`, `focus`), and GTK's
  destination chooser leaves making the browser active again to the window manager
  (`present_destination_chooser`, `bind_host_parent` in `src/ui/chooser/`). The QA's Xvfb runs
  none. Rune's QA host gives the browser the keyboard back as a window manager does
  (`qa::windows`). It also moved `app.directory`, the focused pane's, and a step guarded on
  `focused_pane` ran on Rune only. A trial with the three focus fields set aside (v2) still
  left 17 of 20 parting on those two, not none, as an earlier version of this entry said.
- **Second facet, met under the rule (v4):** from the second chooser of a sequence on, GTK's
  browser reports its row while the chooser is open and through every later close, where
  Rune's reports none while a chooser is open, as GTK's did at the first. 19 of 20 seeds part
  there, each at the click that opens a later chooser: GTK's row against Rune's no focus, the
  direction the ruling does not cover.
- **Concerns:** the `chooser` alphabet's smoke tier. No scenario reads the browser's focus
  around the floating chooser.
- **State:** both facets ruled and excused, by eight `toolkit` rules in
  `tests/qa/fuzz/normalisation.map`.
  - The first four apply while GTK's whole window has no keyboard (`context.window_focused`
    false: no mapped widget focused, as in an inactive window) and its browser no focused
    pane: GTK's `null` against Rune's
    on `app.focused`, `app.focused_pane` and `app.panes[].focused`, and any value of
    `app.directory`. Until A20's review (2026-11-19) they applied whenever GTK's browser had
    no focused pane, which also excused GTK's keyboard on the sidebar, the header, the
    location bar or a dialog in every alphabet.
  - The second four apply while Rune's browser has no focused pane and its floating chooser is
    open (`context.toplevels` is `["Copy to"]`, `["Move to"]` or `["Extract to"]`): Rune's
    `null` against any of GTK's on the same fields, and any value of `app.directory`.

  The `chooser` alphabet (v3) leaves out the browser's arrow keys, which GTK skips once its
  browser has no focus. Under the eight rules, the smoke tier (v5) agrees in 20 of 20 seeds.
  v4's twentieth, seed `0x03`, was Up from the first row of the chooser's listing (`owner.md`
  A1 item 3), not this finding's, and v5 leaves it out ([A20](packages/A20.md)).

### #2610: GTK's player shows a sub-second clip's total as `--:--`

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2610), filed on 2026-11-18
  as the finding's reference and closed at once: its state is kept here.
- **What is wrong, on GTK:** a video or audio clip shorter than one second shows its total
  time as `--:--`. `sync_time` (`src/ui/preview/video.rs`, `audio.rs`) returns while the shown
  seconds equal the ones it last wrote, and they start at `(0, 0)`, which is what a
  sub-second clip has at its start, so the total label keeps its initial text. Rune shows
  `0:00`, as GTK does for every clip of a second or more.
- **Concerns:** [A16](packages/A16.md)'s `media` alphabet, version 3 (0.5 s and 0.7 s clips).
  Later versions use clips of 1.5 s and 2.5 s.
- **State:** open, set aside: Rune is right. A fix belongs on `main`.

### #2611: GTK's video player swallows Left and Right after Right into it

- **Code:** `oracle`. [Issue](https://github.com/lgse/strata/issues/2611), filed on 2026-11-18
  as the finding's reference and closed at once: its state is kept here.
- **What is wrong, on GTK:** after Right from the listing into a video preview, Left and Right
  in the player neither seek nor return the keys to the listing. GTK's own table for a
  keyboard-owned media preview claims them as 5 s seeks (`SEEK_AND_VOLUME`,
  `media_key_command`, `src/ui/preview/keyboard.rs`), but after Right its keyboard is on an
  unnamed widget, where the press is swallowed: a 12 s clip moved to `0:05` stayed there after
  Left. Rune puts the keyboard on C13's "Video frame" button, whose Left and Right seek.
- **Concerns:** [A16](packages/A16.md)'s `media` alphabet, version 5, seed `0x03` (Left after a
  play to the end: `0:02` on GTK, `0:00` on Rune). Version 6 sends no Left while a video shows.
- **State:** open, set aside: Rune is right. A fix belongs on `main`.

### #2612: GTK's QA snapshot reads the audio player's art and spectrum unnamed

- **Code:** `oracle`, recoded from `divergence` on 2026-11-19: GTK is at fault.
  [Issue](https://github.com/lgse/strata/issues/2612), filed on 2026-11-18 as the finding's
  reference and closed at once: its state is kept here.
- **What differs:** in the fuzz window's drawer, GTK's `app.preview` holds neither "Album art"
  nor "Audio spectrum", and Rune's C13 holds both
  (`crates/strata-components/src/media_player/audio/`). It was first read as GTK's layout
  hiding them (`src/ui/preview/audio/layout.rs`).
- **Why GTK is at fault:** GTK's layout does show both there: a probe in the same window
  allocated the art 330 by 254 and the spectrum 144 high. But GTK names them with a bare
  `update_property(Property::Label(…))` (`src/ui/preview/audio/artwork.rs` and `spectrum.rs`)
  rather than `crate::ui::accessibility::set_label`, and GTK has no public getter for an
  accessible label, so its QA harness reads only the names that helper records
  (`src/ui/accessibility/qa.rs`, `src/qa/targets/naming.rs`). Both widgets read unnamed, and
  `app.preview` leaves unnamed nodes out. Rune names them as GTK means to, so Rune is right; the
  fix, naming them through the helper, belongs on `main`.
- **Concerns:** [A16](packages/A16.md)'s `media` alphabet, version 1 (`tone.wav`). Version 7
  holds the WAV again, and the fuzz map's `oracle` rule leaves exactly the two names out of
  `app.preview`.
- **State:** open, set aside: Rune is right. A fix belongs on `main`.

### #2613: a GIF's player shows the whole media bar where GTK's has only Play/Pause

- **Code:** `divergence`. [Issue](https://github.com/lgse/strata/issues/2613), filed on
  2026-11-18 as the finding's reference and closed at once: its state is kept here.
- **What differs:** GTK's bar for a GIF is its play toggle alone, "Play/Pause (Ctrl+Alt+Space)"
  (`append_media_controls` returns before the time, the seek slider and the volume,
  `src/ui/preview/media_controls.rs`). C13's plain bar adds the "Video frame" button, the
  title, the time (`0:00/0:30` for a one-second GIF), "Playback position", "Mute/unmute" and
  "Volume".
- **Concerns:** [A16](packages/A16.md)'s `media` alphabet, version 1 (`spin.gif`). Version 7
  holds the GIF again, and agrees with GTK.
- **State:** fixed in [A16](packages/A16.md)'s Slice 4
  ([#2614](https://github.com/lgse/strata/pull/2614)): C13's `MediaKind::Animation` is the
  frame and its play toggle alone, plain media show no title and an unnamed frame, and the host
  opens a GIF as an animation (`crates/strata-app/src/preview/media/mod.rs`). As GTK's
  overlay takes none of the player's keys, Left from a GIF's frame moves to its play toggle
  and the next Left leaves; the audio view takes the player's keys, as GTK's drawer does.
  Covered by `preview::tests::media::a_gif_plays_in_the_plain_bar_and_loops` and
  `an_audio_player_keeps_left_and_right_and_a_gif_gives_left_back`.

### #2621: a fully covered X11 window is `Hidden` on Rune, where GTK keeps it `Visible`

- **Code:** `deviation`, ruled by the owner on 2026-11-19 ([`owner.md`](owner.md#ruled)). [Issue](https://github.com/lgse/strata/issues/2621),
  filed on 2026-11-19 as the finding's reference and closed at once: its state is kept here.
- **Seen:** by the review of [A21](packages/A21.md)'s Slice 2a
  ([#2604](https://github.com/lgse/strata/pull/2604)). GTK's `presence_of` (`src/ui/presence.rs`)
  reads only an unmapped window and the toplevel's `SUSPENDED` and `MINIMIZED` states, so a
  window covered whole by others stays `Visible`. Rune's `App::on_visibility` reports covering
  on X11 and macOS, and `shell::presence` follows it (`crates/strata-app/src/shell/presence.rs`),
  so the covered window's browsers are `Hidden` and recheck their folders less often until it
  is uncovered. The Wayland half, a minimised window that stays `Visible`, is a Rune gap (Rune
  packet entry 111), not this finding.
- **Concerns:** no run.
- **State:** ruled: Rune's report stands (the owner, 2026-11-19, through the coordinator).
