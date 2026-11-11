# V5 `strata-rune` shell and loop

| | |
| --- | --- |
| Status | Done. Owner decision 8 is settled: the GLib context is embedded in Rune's loop |
| Depends on | V4b |
| Unblocks | V6 |
| Owns | `crates/strata-app`, `crates/strata-rune`, `strata-platform`'s `glib_loop`, the start-up sequence in `strata-fs`, the settings store ([V5b](V5b-settings-store.md)) |
| Commits | `651ea8bfb` (start-up), `b5218acd5` (`glib_loop`), `7bbab97e8` (the two crates), `ce3b5b83d` (the settings store), `a8f13eef9` (reading the settings), `679fb74c9` (one catalog string). After the record: a refactor of `glib_loop`'s helper thread into smaller functions, for code health, with its tests and the two compositor tests rerun |

`strata-rune [DIRECTORY]` opens a Rune window with Strata's header on a directory. Strata's
`Browser` lives on its UI thread, and GLib work reaches it whether the window shows or is
covered.

## The crates

| Crate | Kind | Holds | Depends on |
| --- | --- | --- | --- |
| `strata-app` | Library | The Rune interface: the state, the view, the event queue, start-up and the two hosts. Later the Columns adapter (V6) and the QA backend behind a `qa` feature (V7) | `rune`, `rune-kit`, `strata-kit`, `strata-components`, `strata-platform`, `strata-core`, `strata-fs`, `strata-desktop` |
| `strata-rune` | Binary | `main`: hand the arguments to `strata-app` | `strata-app` only. Its window tests add the compositor harness as dev-dependencies |

Both are in the Rune lane (`scripts/lanes.py`), in `deny.toml` and under the Rune side's
`clippy.toml`. `strata_app` is in `EXTRACTED_CRATES`, so log filters reach it.

## What landed

### Start-up, shared

`strata_fs::startup` holds the sequence `src/main.rs` ran inline: start the mount watcher,
mark the UI thread, start the stall watchdog. The GTK application and the portal call it.
`strata-app` calls it through `startup::UiThread::claim`, which also acquires the default
GLib context for good. Building the application state takes the `UiThread` it returns, so
a `Browser` cannot exist on a thread that skipped the sequence.

What else `src/main.rs` does before `application.run`, and where it stands:

| Step | In `strata-app` |
| --- | --- |
| Program name and application name | Yes, with its own id, `io.github.lgse.Strata.Rune`, so a window rule or desktop entry for one binary does not match the other |
| `metrics::initialize`, `logging::initialize` | Yes |
| The language, from the saved settings | Yes ([V5b](V5b-settings-store.md)) |
| Leaving the launch folder | Yes, after the argument is resolved |
| `ui::read_install_facts` | No. Half of it is under `src/ui`, and nothing the slice shows reads either fact |
| The GVfs probe and its restart with local backends | No. It needs a `--gvfs-probe` mode in the binary. A hung `gvfsd` therefore blocks `strata-rune` at start, as it would block the GTK application without the probe |
| `portal_setup::refresh_stale_portal`, the renderer restart, `assets::prepare` | No: portal installation and GTK |

### The loop

`Browser` schedules on the default GLib context, so its thread owns that context and
dispatches it. An observer runs inside that dispatch, while Rune holds the application
state, so it only queues (`events::Inbox`). `shell::apply_queued` applies the queue before
the next frame.

**Offscreen host** (`host::offscreen`). One thread owns a `Runtime` and the context.

- A turn dispatches what GLib has ready, for 8 ms at most, then runs one `Runtime::frame`
  stepped by the wall clock, whose `poll` applies the queue.
- When a turn finds nothing to do, `wait` blocks in `iteration(true)` with a GLib timeout
  at `Runtime::wake_deadline()`.
- It is compiled for tests only. V7 gives it the `qa` feature.

**Windowed host** (`host::windowed`). Rune runs winit's loop, and nothing there looks at
GLib. `strata-platform`'s new `glib_loop` shim embeds the context:

- On the UI thread, `GlibLoop::pump` dispatches what is ready, then asks the context what it
  waits for: `g_main_context_prepare` and `g_main_context_query` give file descriptors and
  a timeout.
- A helper thread waits for exactly that with `g_poll`, and then sends the next pump to
  Rune's loop through `AppHandle::update_if`.
- `poll` pumps too. A source the UI thread attaches in a handler does not wake the helper,
  because GLib signals only a context owned by another thread.
- Nothing polls. An idle window has no wake-ups.

glib 0.22 binds neither `g_main_context_query` nor a poll, so the shim makes those two
calls through `glib::ffi`, each with its safety note. `docs/unsafe-code.md` lists them.

### The window

- The header is the one strata-components' X1 demo draws, with Strata's catalog strings as
  the accessible names. Its five actions do nothing yet. The window buttons go to the
  window manager through `strata-platform`'s `csd`.
- The body shows the directory's name. The title is "name — Strata", the GTK window's rule.
- The argument resolves through `gio::File::for_commandline_arg`, as the GTK application's
  does, and goes to `Browser::navigate`. Without one, the home directory opens.
- The look is `Look::fallback(true)` and the bundled font.
- `strata-rune --help` prints `usage: strata-rune [DIRECTORY]`. It has no `--qa`. An
  unknown option or a second directory is a usage error, exit code 2. The usage text is
  English, as the GTK binary's command-line errors are.

### Settings

The store moved, by the owner's ruling. [V5b](V5b-settings-store.md) has the survey, what
moved and what stayed.

## Decision 8: the windowed wake

Measured on 2026-11-11 at Rune `2a44156`, in a private headless sway on the host, with
`crates/strata-platform/examples/wake-probe.rs` and `tests/session/wake.rs`. The probe has
something due every 16 ms, and shows its keyboard focus, so a cover leaves it owing a frame
it cannot draw. Three rounds of each, with the same result every round:

| Way into Rune's loop | Window shown, 2 s | Covered by a full-screen window, 3 s |
| --- | --- | --- |
| A `Waker` call, then `poll` | 123 runs, 7 to 10 ms late: it waits for a frame | No runs |
| A `Tx::wake_after` timer, then `poll` | 120 runs, under 1 ms late | 12 runs, one every 250 ms |
| `AppHandle::update_if` | 124 runs, under 1 ms late | 186 runs, under 1 ms late |

So Rune #139 still holds at the pin: a `Waker` alone never delivers under a cover, which
rules out the plan's second option as written. A timer is delayed to four a second.
`update_if` is the one path that stays on time.

The same question asked of `strata-rune` itself: how long after a file is created in the
open folder do the browser's events reach the interface? Eight changes each:

| Windowed host | Shown | Covered |
| --- | --- | --- |
| The plan's tick: `poll` dispatches GLib and asks for `wake_after(16 ms)` | 120 ms | 750 ms |
| GLib embedded, notified through `update_if` | 102 ms | 102 ms |

About 100 ms of each figure is the browser's own delay before it reads a changed folder.

The owner chose the embedding. `strata-rune`'s `tests/window.rs` keeps the second table as
a test: a covered window may take 300 ms longer than a shown one, no more.

## Tests

| Test | Where | Shows |
| --- | --- | --- |
| `a_glib_idle_callback_reaches_the_ui_state_within_one_frame` | `strata-app`, offscreen | The slice plan's gate. An idle callback navigates; one turn later the state has the location |
| `the_first_listing_after_start_up_stays_off_the_ui_thread_guard` | `strata-app`, offscreen | A three-entry folder lists after the shared start-up. Debug builds only: the guard is a `debug_assert!` |
| `a_launch_opens_its_directory_and_titles_the_window_for_it` | `strata-app`, offscreen | An absolute path, a relative one, a `file://` URI and no argument each reach `Browser::active_location()` and the title |
| `saved_settings_reach_the_browser_and_the_language` | `strata-app`, offscreen | [V5b](V5b-settings-store.md) |
| `waiting_ends_when_glib_has_work_or_a_rune_timer_is_due` | `strata-app`, offscreen | The idle wait |
| `launch::tests` | `strata-app` | The command line, a non-UTF-8 argument included |
| `glib_loop::tests` | `strata-platform` | A source from another thread, a timeout attached on the owning thread, an idle context, a pass that runs out of budget, a thread that does not own the context |
| `window::a_folder_change_reaches_the_window`, `window::a_cover_does_not_slow_a_folder_change` | `strata-rune`, headless sway | The real binary: its title, its app id, and a folder change reaching the interface, shown and covered |
| `wake::an_update_reaches_runes_loop_while_the_window_is_covered` | `strata-platform`, headless sway | The first table above. It asserts only what `glib_loop` relies on and prints the rest |

Each offscreen test runs in a process of its own (`strata_test_support::isolated_test`):
the default GLib context, the start-up sequence and the saved settings belong to one
process. They need no display.

**A window is checked without the desktop.** The plan expected no compositor and left the
window to the owner's hand check. `mise run test-platform` starts a private headless sway
on the host, which the import brought, and `strata-rune`'s window tests joined that task.
They need a GPU or lavapipe, so they do not run in the pinned image. The owner's hand check
on Hyprland (decision 7) still stands: sway is not Hyprland, and nothing here looks at the
picture.

## Done when, and the evidence

All on 2026-11-11. The pinned-image runs are from the worktree at
`~/dev/strata-next-verify`, with their logs in its `target/`.

- **`strata-rune <directory>` opens a themed window titled for that directory.** In a
  private headless sway, `window::a_folder_change_reaches_the_window` waits for a window of
  the binary titled "Projects — Strata" with the app id `io.github.lgse.Strata.Rune`.
  Nothing looked at the picture: that is the owner's hand check.
- **The idle-callback test passes,** with the six other tests of `strata-app`. Removing the
  `poll` hook from the host makes it and the launch test fail.
- **The GTK suite passes with the shared start-up and the settings store.** In the pinned
  image at `a8f13eef9`: `./scripts/quality.sh` in all three phases (3,948 tests passed
  exactly once, 771 ignored; `v5-final-quality.log`) and `./scripts/qa.sh` (787 tests, 758
  scenario runs, 2 ignored; `v5-final-qa.log`). The start-up commit alone passed the same
  two at `651ea8bfb` (`v5-startup-quality-test.log`, `v5-startup-qa.log`).
- **The windowed wake is chosen and recorded, with its measurement,** above.
- **No GTK.** `cargo tree -p strata-rune -i gtk4` matches no package, and neither do `gdk4`
  and `sourceview5`. `cargo deny --workspace check` passes with the two crates' rules.
- **The Rune side.** On the host, `mise run test-rune`: 71 test binaries, 2,298 tests
  passed, none failed, 252 ignored. That is V4b's 2,286 and 249 plus this package's twelve
  tests and three compositor tests. `mise run test-platform wake::` and `window::` pass.
  Clippy passes on both lanes with warnings denied, and `mise run compile-features` passes
  with `glib-loop` alone.
- **Catalogs.** `./scripts/test-headless.py i18n::` passes with the new string.
- **CI on `strata-next`.** At `da519dd97` the build job and the policy job pass. The test
  and QA jobs do not run for a push to this branch, as before. The image publisher fails
  on the same missing manifest as before the package (decision 12); it names
  `strata-app` first now, because that crate sorts first.

Not run: `./scripts/qa-mutation.sh`, the rest of `mise run test-platform`, and anything on
a real desktop. The mutation patches touch no file this package changed. The other
compositor tests cover shims this package did not change.

## What it left for later packages

- **The offscreen host is test-only.** V7 un-gates it for the `qa` feature and gives it a
  way to be woken from the thread that reads commands.
- **Only the location is mirrored.** `events::Inbox` turns path and focus events into one
  `Change::Path` and counts the rest. V6 extends it with the entry events.
- **The header's actions do nothing,** and the window draws no sidebar, tabs or location
  bar.
- **`AppHandle::update_if` is load-bearing.** If a Rune rev changes how it behaves under a
  cover, `wake::` and `window::` say so. Run both when the rev moves.
- **The window tests are local.** No CI job runs the Rune lane yet (decision 13).
- **The settings keys the Columns view needs** are parsed and not used yet
  ([V5b](V5b-settings-store.md)).
- **The GVfs probe and the install facts** are not in `strata-app`, as the table above
  says.

## Deviations from the plan

- **A third wake.** The plan offered a tick or an embedding that calls the `Waker`. What
  landed is the embedding with `AppHandle::update_if` in the `Waker`'s place, because of
  the measurement.
- **Less `unsafe` than the plan expected.** The plan needed `query` and `check` through
  `glib::ffi`. The shim lets `MainContext::iteration` do the check and the dispatch, so it
  calls `query`, and `g_poll` on the helper thread.
- **The window buttons work.** The plan said the header's buttons are inert. The five
  header actions are; Close, Minimize and Maximize go through `csd`, since a borderless
  window has no other close button.
- **`strata-rune` has dev-dependencies.** The plan said it depends on `strata-app` only.
  Its window tests need the compositor harness.
- **One new catalog string.** The header reuses "Toggle sidebar", "New tab", "Search",
  "Appearance", "Settings" and the three window strings. "Maximize or restore window" is
  new, in all thirteen catalogs: it names the Maximize button while the window's state is
  unknown, which on Wayland is always.
