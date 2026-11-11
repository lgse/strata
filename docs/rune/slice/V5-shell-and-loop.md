# V5 `strata-rune` shell and loop

| | |
| --- | --- |
| Status | Not started. One owner decision is open |
| Depends on | V4b |
| Unblocks | V6 |
| Owns | `crates/strata-app`, `crates/strata-rune`, the start-up sequence in `src/main.rs`, the settings store |
| Lands as | A refactor commit per shared-code move, then the new crates |

## Goal

A second binary, `strata-rune`, that opens a themed Rune window on a directory, with
Strata's `Browser` alive on its UI thread and GLib work reaching it.

## The crates

| Crate | Kind | Holds | Depends on |
| --- | --- | --- | --- |
| `strata-app` | Library | The Rune interface: app state, the view, the `poll` hook, start-up, the host loops. Later the Columns adapter (V6) and the QA backend behind a `qa` feature (V7) | `rune`, `rune-kit`, `strata-kit`, `strata-components`, `strata-platform`, and every shared crate |
| `strata-rune` | Binary | `main`: read the arguments, call `strata-app` | `strata-app` only |

The slice plan put all of this in one `strata-rune` crate. The split is the workspace
proposal's: at parity `src/main.rs` runs `strata-app` and `strata-rune` is deleted.

Follow "Adding a crate" in `docs/workspace.md` for both: inherited package fields,
`publish = false`, workspace lints, a `[workspace.dependencies]` entry, a `deny.toml` rule,
and `strata_app` in `EXTRACTED_CRATES` so log filters reach it. `strata-app` declares
`rust_i18n::i18n!` with `strata_core::i18n::Catalogs::compiled()` as its backend. Every
string it shows is localized in all thirteen catalogs, as `AGENTS.md` requires.

## Scope

### 1. Start-up, shared

`Browser` needs things `src/main.rs` sets up before the first listing. V0's first probe
run aborted with "file calls must not run on the UI thread" because one was missing.

- Move the sequence to a function both binaries call: start the mount watcher
  (`adapters::mount_watch::start()`), then mark the UI thread (`util::ui_thread::mark()`),
  then start the stall watcher (`util::ui_thread::stalls::start()`). The order matters: the
  watcher's first read must be off the UI thread.
- Read what else `main.rs` does before `application.run`: logging, locale and
  `ui::read_install_facts()`. Move what does not need GTK.
- This changes the GTK app's start-up path. It is a refactor with no behaviour change, and
  the GTK suite proves it.

### 2. The loop

`Browser` is an `Rc` that schedules on the default GLib context from eight places under
`crates/strata-fs/src/app/browser/`. Its thread must own that context and iterate it.

**Offscreen host.** This is what V0 measured, and what the QA backend and the in-process
tests use.

- One thread acquires `MainContext::default()` and owns the `Runtime`.
- A turn runs `iteration(false)` until it returns false, then one `Runtime::frame(dt)`
  with the wall-clock `dt`.
- `BrowserEvent` observers run inside GLib's dispatch, where the `Runtime` owns the app
  state. They push onto a queue; the `poll` hook drains it at the start of the next frame.
- When a turn finds nothing to do, block in `iteration(true)` with a GLib timeout at
  `Runtime::wake_deadline()`. V0's probe slept a millisecond instead.

**Windowed host.** Rune runs winit's loop, and nothing iterates GLib while it sleeps.

- `poll` iterates GLib until nothing is ready, then drains the queue.
- Something must wake Rune when GLib has work. That is owner decision 8:

| Option | How | Cost |
| --- | --- | --- |
| A tick | `poll` asks for `Tx::wake_after` while the browser has loads, timers or monitors in flight | Polls. A file-monitor change waits for the next tick |
| GLib's embedding | The UI thread calls prepare and query. A helper thread polls the file descriptors and calls Rune's `Waker`. The UI thread calls check and dispatch | glib 0.22 binds neither `query` nor `check`, so it needs `glib::ffi` and an `unsafe` shim in `strata-platform`, with the safety notes `docs/unsafe-code.md` asks for |

- strata-components recorded that at Rune `5d421fd` a `Waker` call alone did not run
  `poll` while a Wayland window was fully covered, and a `wake_after` timer did
  (Rune #139). Measure it again at the pinned rev before choosing.
- `strata-platform`'s GLib bridge runs a context on its own thread. It does not fit:
  `Browser` is not `Send`, and its callers read it synchronously.

If neither option can be made to work, that is the slice plan's stop condition: write up
the Rune feature that is missing and stop.

### 3. The window

- Build the header shell and the default look as strata-components' X1 does:
  `crates/strata-gallery/src/scenes/x1/shell.rs` and `examples/shell/`. Buttons are inert.
- `strata-rune <directory>` opens that directory; with no argument, the home directory.
  The location goes through `Browser::navigate`, as a launch argument does in the GTK app.
- The body shows the directory's name and nothing else until V6.
- Fonts and the look come from `strata-kit` (`fonts::FONTS`, `Look::fallback`). Default
  theme only.
- Keep `usage: strata --qa` out of this binary's help until V7 defines the mode.

### 4. Settings

The slice plan says: read `settings.toml` through Strata's settings store for the keys in
scope. The store is `PreferenceManager`, in `src/ui/preferences`: 22 files, 8 of which use
GTK. `persistence.rs` reads and salvages the file.

Start with a survey of that folder: which files a toolkit-free store needs, and what
`PreferenceManager` itself holds that is GTK. Then choose:

- **Move the store.** The GTK-free files go to a shared crate, and `PreferenceManager`
  keeps its place as a wrapper. This is the workspace proposal's intent ("shared state
  under `ui/` moves"). It touches a path every GTK view uses, so it needs the full GTK
  suite and the saved-preferences coverage `docs/preferences.md` describes.
- **A read-only reader in `strata-app`** for the keys below. Less to move, but a second
  parser of one file. Not recommended.

Keys in scope: those the parity set's `prefs` headers name, plus what launch and Columns
need.

| Key | Used by |
| --- | --- |
| `type_to_search`, `single_click_previews`, `tenxer_mode`, `arrow_navigation_scoped` | Headers in G2 and G3 |
| `sort_key`, `sort_direction`, `group_by_type`, `filter_include_subfolders` | Headers in G2 |
| `browser_mode`, `show_hidden`, the folder sort settings | Launch, and the hidden-files toggle |
| The Columns width and "Show sizes in Columns view" | The Columns view |

A key the Rune interface reads but cannot honour yet, such as `tenxer_mode`, is read and
ignored. The runs that need it become `pending` in V8.

## Out of scope

- The Columns view. V6.
- The `--qa` mode. V7.
- Sidebar, tabs, location bar, menus, settings UI.
- Packaging, a desktop entry, single-instance handling.

## Tests

- **A GLib idle callback reaches the UI state within one frame.** In the offscreen host:
  schedule `glib::idle_add_local_once`, run one turn, and assert the state it changed. The
  slice plan names this test as the gate.
- **Start-up.** A test that the first listing after the shared start-up does not trip the
  UI-thread guard.
- **A directory argument.** In the offscreen host, launch on a temporary directory and
  assert `Browser::active_location()`.
- **No GTK.** `cargo tree -p strata-rune -i gtk4` matches no package, and the same for
  `gdk4` and `sourceview5`.
- **The GTK app.** `./scripts/qa.sh` and `./scripts/quality.sh test`, because start-up and
  the settings store moved.

These tests need no display. Run them with the display variables cleared.

**The window itself is not checked by an agent.** `AGENTS.md` forbids using the desktop
display, and the pinned image has no compositor: V0 found no `sway` in it. Opening the
window on Hyprland is the owner's hand check (decision 7). It does not gate this package.

## Done when

- `strata-rune <directory>` opens a themed window titled for that directory.
- The idle-callback test passes.
- The GTK suite passes with the shared start-up and settings store.
- The windowed wake is chosen and recorded, with the measurement that backs it.

## Risks

- **The windowed wake.** The least certain part of the slice. The offscreen host cannot
  show whether it works.
- **Moving the preference store is larger than it looks.** It is the one piece of this
  package that can break the GTK app. If the survey shows more than a few files, give it
  its own document and commits.
- **Two binaries, one config.** Both read and one day write the same `settings.toml`. In
  the slice the Rune binary only reads.
