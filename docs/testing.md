# Testing

The Rust suite covers state transitions, filesystem edge cases, UI components, and
application behavior headless on Rune's virtual clock. The compositor suite (`mise run
test-platform`) tests `strata-platform`'s Linux shims and `strata`'s real window inside
private headless `sway` sessions. The end-to-end QA scenarios in `tests/qa/scenarios/`
drive `strata --qa` inside the pinned container image (offscreen by default, or on a
private headless `sway` compositor for `"host": "session"` runs) and verify both
interface state and filesystem outcomes.

## Running the suites

```bash
mise run test                                 # headless workspace Rust tests (gallery unit tests only)
mise run test services::operations::tests     # filtered headless Rust test selection
mise run test-platform                        # compositor, D-Bus, and window tests in headless sway
mise run test-platform window::               # windowed binary tests (host, shell, key routing, QA session)
./scripts/quality.sh                          # formatting, Clippy, and all Rust tests in pinned container
./scripts/quality.sh test --jobs 4            # all Rust tests across 4 local lanes in pinned container
./scripts/test-headless.py relevant::module   # native Rust selection in isolated headless environment
./scripts/qa.sh                               # every QA scenario, every mode
./scripts/qa.sh --smoke                       # critical-path QA smoke list (tests/qa/smoke.txt)
./scripts/qa.sh copy-leaves-source            # one scenario, all of its runs
./scripts/qa.sh arrow-keys-move-focus/columns-arrows  # one run
```

### Targeted local validation

Choose checks from the behavior and caller mapping, not from changed-file names
alone. Bounded changes may use a non-empty, relevant selection; broad,
cross-cutting, shared infrastructure, dependency/build/CI/harness changes, or
uncertain coverage require the full pinned quality phases and the full QA run.
Include relevant views, callers, and preference behavior, and add or run the
regression test that proves the change. Record the scope rationale, commands,
results, and intentional omissions in the handoff. After editing, rerun affected
tests rather than unrelated suites.

Run local lint and formatting checks only before pushing, not during the
edit/test loop. For Rust changes, run `./scripts/quality.sh fmt` and
`./scripts/quality.sh clippy` (or `mise run fmt` and `mise run clippy`) on the
final code at that checkpoint; rerun affected checks if fixes change the code
before the push. Even when full Rust tests are needed during iteration, use
`./scripts/quality.sh test` and defer lint/format phases until pre-push. CI's
lint and formatting checks remain unchanged.

The repository owner may explicitly authorize pushing a PR without the normally
required local tests for the current change/push. A general push request,
urgency, silence, or consent for another push does not qualify. Record the
explicit authorization, omitted suites, and known failures or unverified behavior
in the handoff and PR description; never report skipped tests as passing. This
exception does not waive lint/format checks, GUI isolation, or required CI/merge
checks. Renew consent if the authorized scope changes, or run the required tests.

For native targeted Rust tests, both `mise run test [filter]` and
`./scripts/test-headless.py [filter]` run headless across the unified workspace
(`--workspace --all-targets --all-features`; `mise run test-rune` is a
compatibility alias). Use module/name filters:

```bash
mise run test services::operations::tests
./scripts/test-headless.py services::operations::tests
./scripts/test-headless.py basenames_reject_empty_reserved_nested_absolute_and_nul_names
```

These select test execution across targets, not compilation to one target.
Targeted runs can still incur a full test build; Cargo's cache helps subsequent
iterations.

The unit tests need the `test-support` feature, which `--all-features` turns on. It
compiles the test hooks and test-only helpers of the workspace crates under `crates/`,
because a crate's `#[cfg(test)]` code is invisible to another crate's tests. The QA
runner builds `strata` with `--features qa,invariant-checks` and without `test-support`,
so the application the scenarios drive carries no test hooks.

Controller regressions live under `crates/strata-fs/src/app/browser/tests/`, grouped by
navigation, location input, selection, monitor updates, preferences, operations, undo,
metadata, staging, event dispatch, and archive activation. Add cases to the matching
owner rather than to `crates/strata-fs/src/app/browser/tests.rs`, which holds shared
fixtures. For example, `mise run test app::browser::tests::undo::` selects undo
regressions. Local-operation regressions live under
`crates/strata-fs/src/adapters/local_operations/tests/` (keep benchmark names in the
parent `tests.rs` stable for `scripts/benchmark-delete.sh`). UI shell, slot, preference,
and view regressions live in adjacent `tests.rs` modules under `crates/strata-app/src/`
and `crates/strata-components/src/`.

A filter must collect at least one test; confirm with Cargo's output that it did.
`scripts/quality.sh` only accepts `all`, `fmt`, `clippy`, or `test`, plus the test
phase's `--jobs N` and `--refresh-timings` (see "Local lanes"), and does **not**
forward test filters.

`./scripts/qa.sh` takes scenario names, or `<scenario>/<run>` for one run, and
refuses unknown names, listing the names it knows; see "Scheduling and run costs"
for `--skip` and the other options. A selection that runs no scenario fails.
Preserve the verified image provenance and the `target/qa-container` and
`target/quality-container` caches.

Every GUI or delegated check must clear inherited display variables and use a
private display/compositor and private D-Bus session. Never run against the user's
desktop or an inherited session bus.

### The pinned test image

The container runners prefer Podman when available; select an engine explicitly with
`STRATA_CONTAINER_ENGINE=podman` or `docker`. CI explicitly selects Docker; an
image in one engine's store is not visible to the other. Normal runs verify and
reuse the local base image. If missing, they pull the published environment once,
verify its input label and platform, and record it locally. They **never
automatically build images or fetch distribution packages**. A missing publication
fails with an actionable message rather than silently bootstrapping.

When intentionally updating the environment, or before its first publication,
explicitly build the local base once:

```bash
STRATA_CONTAINER_ENGINE=podman python3 scripts/image_base.py build
./scripts/qa.sh
```

The build makes the recipe's `dependencies` stage, the one the publisher pushes
as the base: the environment, and the Cargo dependencies described under
[The image's dependencies](#the-images-dependencies). It needs the network for
Arch's archive, crates.io, and Rune's repository.

The recipe is `tests/container/Dockerfile`: a digest-pinned Arch Linux base, a
dated Arch package snapshot, GLib, Poppler, GDK Pixbuf, Fontconfig, Bubblewrap,
FFmpeg/ffprobe, ImageMagick + `libheif`, GVfs (for fake devices in QA), `sway`,
`Xwayland`, `lavapipe` (software Vulkan for headless GPU rendering in session runs),
`strace` (for the opt-in main-thread I/O audit), and Rust `1.99.1`.
`scripts/image_inputs.py image-key` hashes those inputs, so changing them selects
a new base; application edits do not. The runners compile the checked-out
application and retain Cargo's worktree cache. No host desktop libraries, fonts,
desktop sockets, or Rust binaries are mounted.

The checkout is mounted at `/workspace`. QA build and Cargo caches live in
`target/qa-container` (quality's in `target/quality-container`), separately
from native builds, and artifacts are owned by the invoking user. Minimal
generated passwd/group files provide the invoking UID/GID to D-Bus, so one
published environment works across local user IDs without rebuilding it or
mounting the host's account database.

Rootless Podman runs unmask `/proc/*` inside the test container so bubblewrap
can mount its own private `/proc`; the decoder's sandbox and the container's
seccomp policy remain enabled. Docker's outer seccomp/AppArmor profiles and
system-path masks are disabled for the nested namespace and mount operations.
This applies only to the disposable test container; Strata still launches its
normal bubblewrap decoder sandbox. Neither engine uses privileged mode or mounts
desktop sockets.

#### The image's dependencies

The base holds the third-party crates a run needs, so a first run in a new
checkout fetches nothing. The recipe's last stage loads the workspace's
manifests alone, with an empty file for each target
(`tests/container/stub-workspace.py`), and leaves under
`/opt/strata-dependencies`:

- `cargo/`: a Cargo home with every crate `Cargo.lock` names, fetched;
- `build/`: a target directory with the third-party crates precompiled for the
  QA runner build (`cargo test --features qa,invariant-checks --test qa`);
- `lock.sha256` and `cache-path`: the `Cargo.lock` they were fetched for, and
  the cache they were built for.

Nothing of Strata's own is in it. The stage removes what the stubs built with
`cargo clean -p` for each workspace package, and fails if anything named for
one is left.

`scripts/qa.sh`, `scripts/qa-drive.sh`, and `scripts/quality.sh` run
`scripts/dependency_seed.py` in the container before Cargo. It prints one
`Dependencies:` line saying what it did:

| The run's cache | What happens |
| --- | --- |
| Cargo has fetched into it before: `cargo/registry` or `cargo/git` exists | Nothing is copied. A warm cache is never reseeded or replaced |
| Empty, and it is `target/qa-container` | The fetched and the compiled crates are copied in. The run then compiles Strata's own crates only |
| Empty, and it is another cache, such as `target/quality-container` | The fetched crates are copied in. The cache compiles its own because Cargo fingerprints include source paths |
| Empty, and the base holds no dependencies | Nothing is copied, and Cargo fetches what it needs |

- **`Cargo.lock` ahead of the image:** when the lockfile moved past the image's
  snapshot, seeding prints a warning with both digests, copies what the image
  has, and Cargo fetches and builds only the difference.
- **Interrupted copies** are retried atomically (`cargo/registry` is moved in last).
- **No Cargo auto-clean:** seeded Cargo homes carry `cache.auto-clean-frequency = "never"`
  so Cargo does not prune crates inherited from an older base image.
- **`./scripts/quality.sh fmt` and a CI shard copy nothing** because neither compiles.

### Shared environment for formatting, lint, and Rust tests

`./scripts/quality.sh` runs formatting (`cargo fmt --all --check`), Clippy
(`cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`),
and the full Rust test suite in the same verified build base. Use
`./scripts/quality.sh fmt`, `clippy`, or `test` for an individual phase. Quality
runs set `GIO_USE_VFS=local`, so the image's `gvfs` (which only the QA runner's
fake devices use) does not change which GIO backend a Rust test gets.

Quality's Cargo home and build directory live under `target/quality-container`,
separate from the QA cache and native builds. The container's `/tmp` is an
executable tmpfs: file-operation tests need `O_TMPFILE` and file handles there,
which an overlay `/tmp` may refuse. `STRATA_REQUIRE_DEVICE_TESTS=1` (set in CI
and `scripts/test-headless.py`) fails those tests instead of skipping when the
filesystem lacks support.

### Rust quality shards

CI's **Quality build and lint** job runs formatting and Clippy, then compiles
the workspace test binaries once (`--workspace --all-targets --all-features
--no-run`). It exports only test executables and a plan, not Cargo caches.
`scripts/quality_ci.py` collects each libtest inventory (including explicitly
ignored tests) and assigns every entry longest-first across four shards using
the weights in `scripts/quality-durations.json`. Unknown tests receive a
one-second weight and always participate.

Each **Rust tests shard N** verifies the checkout revision, source fingerprint,
image inputs, executable checksums, and libtest inventory before running its
assigned tests across parallel headless lanes with no display and an unreachable
session bus. The runner checks libtest's actual selection and
final passed/ignored counts before writing a success receipt; nonzero exits,
empty runnable shards, or changed inventories fail.

`tests/qa.rs` uses a custom harness (`harness = false`) that preserves libtest's
`--list`, `--ignored`, `--exact`, and `test result:` formats, so its scenario
runs are inventoried as explicitly ignored tests and its unit tests run in the
shards like any other.

To reproduce the CI shard handoff locally in the pinned container:

```bash
STRATA_QUALITY_TASK=build ./scripts/quality.sh test
for shard in 0 1 2 3; do
  STRATA_QUALITY_TASK=shard STRATA_QUALITY_SHARD="$shard" ./scripts/quality.sh test
done
python3 scripts/quality_ci.py verify
```

### Local lanes

Ordinary `./scripts/quality.sh test` runs `scripts/quality_ci.py local`. It
builds the bundle once into `target/quality-local` as a single shard and runs it
in lanes on the local machine, with the same plan validation, receipts, and
exactly-once verification as CI. The default is one lane per two available CPUs;
pass `--jobs N` to choose another count. A lane's full output goes to
`target/quality-local/reports/shard-0-lane-N.log`.

### Balancing weights

`scripts/quality-durations.json` maps each test to its expected duration for
shard/lane balancing. Refresh it after adding or substantially changing slow tests:

```bash
./scripts/quality.sh test --refresh-timings
```

### Keep Rust tests off the local desktop

`mise run test` and `./scripts/test-headless.py` clear `DISPLAY`, `WAYLAND_SOCKET`,
and `DBUS_SESSION_BUS_ADDRESS` and point `WAYLAND_DISPLAY` and the bus at nonexistent
names so tests run purely headless on Rune's virtual clock with temporary `HOME`/XDG
directories:

```bash
./scripts/test-headless.py
./scripts/test-headless.py -- --nocapture
./scripts/test-headless.py relevant_module_or_test_name
```

`test-headless.py` also sets `STRATA_REQUIRE_DEVICE_TESTS=1` and supports
`STRATA_TEST_TMPDIR`.

### Private main contexts in Rust tests

A Rust test that drives async code on its own GLib main context creates it with
`test_support::PrivateContext`, never `glib::MainContext::new()` or
`glib::MainContext::with_flags()` (`clippy.toml` rejects both), and never
`<glib::MainContext as Default>::default()`. A GIO call started on the context
keeps it alive from its `GTask`, so a GIO pool thread can drop the last reference
after the test finishes and abort the process if local callbacks are finalized on
the wrong thread. `PrivateContext` destroys remaining sources on the creating
thread at drop and safely parks any late in-flight completion. Tests driving the
process-wide default context hold `test_support::ASYNC_MAIN_CONTEXT_DEFAULT`.

### Native debugging dependencies

These are installed inside the pinned image; only host runs of
`mise run test-platform` need them locally:

| Purpose | Arch | Debian / Ubuntu |
| --- | --- | --- |
| Headless Wayland compositor (`test-platform`) | `sway`, `xorg-xwayland` | `sway`, `xwayland` |
| Software Vulkan ICD (optional, for lavapipe) | `vulkan-swrast` | `mesa-vulkan-drivers` |
| Private session bus (`test-platform`) | `dbus` | `dbus-daemon`, `dbus-bin` |
| Screen capture (`test-platform` / `qa` session runs) | `imagemagick`, `grim` | `imagemagick`, `grim` |

## QA scenarios (`--qa` mode)

A build with the `qa` Cargo feature (never enabled in releases) accepts
`strata --qa <directory>`: it opens on the directory, reads one JSON command per
line on stdin, runs it through `strata-qa`'s engine and `strata-app`'s QA backend,
and writes one JSON reply per line on stdout.

```bash
./scripts/qa.sh                                       # every scenario, every mode
./scripts/qa.sh --smoke                               # critical-path smoke set (tests/qa/smoke.txt; runs on PRs in CI)
./scripts/qa.sh arrow-keys-move-focus                 # one scenario, all of its runs
./scripts/qa.sh arrow-keys-move-focus/columns-arrows  # one run of it
STRATA_QA_KEEP_ARTIFACTS=1 ./scripts/qa.sh            # keep artifacts for passing runs too
```

`scripts/qa.sh` runs `cargo test --locked --features qa,invariant-checks --test qa`
inside the pinned image, sharing `target/qa-container` with `scripts/qa-drive.sh`
and `scripts/qa-mutation.sh` (through `scripts/lib/qa-container.sh`). Because the
build cache is shared, run them one after another, never concurrently.

`tests/qa.rs` refuses to start without `STRATA_QA_CONTAINER=1`. A host run
(`STRATA_QA_HOST=1`) requires explicit owner consent for the task, is labelled
unsupported, is never validation evidence, and must be recorded in the handoff.
Do not wire the QA runner into `scripts/test-headless.py`.

`invariant-checks` turns on exact but slow consistency assertions (for example,
checking the selection position cache against a full listing scan). `scripts/qa.sh`
and `scripts/qa-drive.sh` build with it, and `cargo test --all-features` enables
it too; plain `cargo build` leaves it off so development builds keep normal speed.

### Offscreen and session hosts

Each scenario (or variant) declares its execution host via the optional header
field `"host"`:

- **`"offscreen"` (the default):** drives `strata --qa` through Rune's offscreen
  QA host (`crates/strata-app/src/qa/`). Input goes directly through Rune's input
  layer on a virtual clock, state is read from Rune's accessibility tree (roles
  mapped to standard AT-SPI role names), and keyboard layouts are sent via `keymap`
  requests. No display server is required, making runs fast and deterministic.
- **`"session"` (see [The session host](#the-session-host)):** runs the scenario
  against a real `strata --qa` window (`STRATA_QA_WINDOW=1`) on a private headless
  `sway` Wayland compositor inside the pinned container, for scenarios that need
  real window focus, the system clipboard, compositor drag-and-drop, or portal
  file-chooser windows.

#### The session host

A run whose header (or variant) sets `"host": "session"` needs what the offscreen
host cannot supply: window focus, the system clipboard, or drag-and-drop through
a compositor. The QA runner automatically starts a private headless `sway`
(`WLR_BACKENDS=headless`, software `lavapipe` Vulkan rendering) in the run's
runtime directory and launches `strata --qa` with `STRATA_QA_WINDOW=1`
(`crates/strata-app/src/qa/session.rs`). Input crosses the compositor through
Wayland's virtual keyboard and virtual pointer (US layout). Portal scenarios
(`"portal": true`) run on the session host as well: `strata --qa --portal` opens
each request's file chooser in a real window, steps act on the adopted window,
and virtual input devices connect at the first `adopt`
(`crates/strata-app/src/qa/session/portal.rs`).

Outside the container, `mise run test-platform window::qa` exercises the same
session host inside `strata-platform`'s private `sway` harness.

### How a run is set up

Each run of a scenario gets:

- a fresh fixture tree in its own temporary directory under `/tmp`, with fixed
  modification times, materialized from `tests/fixtures/harness.json`, which also
  defines seeded preferences, static environment, and keyboard layouts;
- a throwaway `HOME` and XDG directories, seeded with a complete `settings.toml`
  so no preference is inherited from a default change, and with the first-run
  file-chooser portal offer already answered;
- with `"session_bus": true`, a private D-Bus session with no service directories,
  so nothing is D-Bus-activated behind the run's back; without it, a bus address
  that cannot connect;
- with `"volume_monitor": true` (and `"session_bus": true`), a fake GVfs volume
  monitor on that bus (see [Fake devices](#fake-devices));
- for `"host": "session"` runs, a private headless `sway` compositor.

`scripts/qa.sh` mounts a tmpfs on the container's `/tmp`, which holds every run's
fixture, `HOME`, and runtime directory. Fixtures and `HOME` share one device so
Trash and restore work within it, while `/dev/shm` is a separate volume for
cross-volume tests. Nothing in `/tmp` outlives the container.

### Fake devices

The container has no `udisks2` daemon and no real removable media, and GIO's
built-in unix volume monitor hides mounts a rootless container can make. A run
whose header has `"volume_monitor": true` (with `"session_bus": true`) starts
`tests/qa/volume_monitor.py` on the run's bus before the application. It serves
`org.gtk.Private.RemoteVolumeMonitor` under the name configured by
`tests/qa/volume_monitors/strata-qa.monitor`, pointed to via `GVFS_MONITOR_DIR`.

That client stays unloaded under `GIO_USE_VFS=local` for every other run. In a
`volume_monitor` run, scenarios plug and unplug devices with `dbus_send` to
`org.strata.QaVolumeMonitor.Add` and `Remove`, and read recorded calls from
`$environment/volume-monitor.calls`. Mounts are real folders under `$environment`,
and the monitor's log is kept as `volume-monitor.log`.

### Scenario files

A scenario is a `.jsonl` file under `tests/qa/scenarios/<feature area>/`, a
folder named in `snake_case` without a `test_` prefix, such as
`tests/qa/scenarios/clipboard/copy-leaves-source.jsonl`. Scenario names are
lowercase kebab-case (`./scripts/qa.sh` maps `_` to `-`) and unique across
folders. The first line is a JSON header and every subsequent line is one step:

```json
{"scenario":"arrow-keys-move-focus","modes":["columns"],"prefs":{"type_to_search":false},"layout":"us","mutation":null,"known_bug":null,"variants":[...]}
```

`modes` runs the file once per view, and optional `variants` run it again with
their own preferences, layout, host, tree, or variables; a run is named
`<scenario>/<label>`, as in `PASS copy-leaves-source/columns-ru`. `host` selects
`"offscreen"` (default) or `"session"`, `mutation` names a mutation patch under
`tests/qa/mutations/`, `known_bug` records an issue that makes a failure the
expected result, `skip` keeps a scenario out of the default run, `cost` orders
runs longest-first, and `quality` scores the scenario per the `qa-quality` skill.

The runner validates every file before any run starts: unknown header keys are
rejected, every step passes through `strata-qa`'s parser, and
`every_scenario_file_is_planned_and_well_formed` checks folder naming, parsing,
mutation patch existence, and run planning.

The full reference for headers, variables, steps, `fs` kinds, targets,
predicates, replies, snapshots, and artifacts is
[`.agents/skills/qa-scenarios/reference.md`](../.agents/skills/qa-scenarios/reference.md).
The `qa-scenarios` skill beside it covers writing, running, and debugging scenarios.

### Accessible names are product surface

The runner locates entries and controls through Strata's accessibility tree.
Those accessible names and roles exist for screen readers first: entry rows carry
their name and `Folder`/`File` description, panes carry their directory and
presentation, menu items carry menu roles and shortcut descriptions, and modal
dialogs carry the dialog role and title. When a scenario cannot identify a
control, name it properly in the application instead of working around
accessibility. Locate controls by role and name, synchronize on conditions
(`wait`, `fs` with timeout), and verify both UI state and filesystem outcomes.

### Keyboard layouts

A `{"layout": "ru"}` step (or the header's `layout`) switches the run's keyboard
layout from the manifest while the application runs. On the offscreen host, the
runner sends the manifest's layout as a `keymap` request and `strata-app`
interprets US key positions under that layout through `strata_desktop::keys::Xkb`.
Key names denote US physical key positions: under `ru`, `{"key": "ctrl+c"}`
presses the key in the US `C` position, and `type` produces the characters of
that layout. On the session host (`sway`), layout steps are currently US-only.

`strata-desktop`'s `keys::xkb` tests (`mise run test keys::xkb::`) verify key
name resolution, case mapping, accelerator parsing/formatting, and refusal tables
directly without any display.

### Scheduling and run costs

`tests/qa.rs` turns every run of every scenario into its own test named
`<scenario>/<run>`, scheduled longest-first using the header's `cost` map (runs
without a `cost` start first, then costliest-first, then by name). Runs with a
shared `fixture_root` serialize against each other automatically.

The runner's own unit tests live in `tests/qa/scenario_tests.rs` and
`tests/qa/step_tests.rs`, registered via `unit_tests!` in `tests/qa/unit_tests.rs`
(`no_test_attribute_hides_a_runner_test` ensures no stray `#[test]` is silently
stripped by `harness = false`).

Refresh scenario `cost` weights after adding or substantially changing scenarios:

```bash
python3 scripts/qa_cost.py run
python3 scripts/qa_cost.py run --repeat 5
python3 scripts/qa_cost.py write target/qa-cost/run-*.log
```

### Replies and failure artifacts

Every process command returns one JSON reply carrying the `app` state snapshot,
and any GLib warning or critical fails the run. A failed run, or every run with
`STRATA_QA_KEEP_ARTIFACTS=1`, writes `target/qa-artifacts/<scenario>/<run label>/`
with screenshots (when available), accessibility tree, transcript, application
and compositor/bus logs, fixture listing, final snapshot, and result.

### Driving a session by hand

`scripts/qa-drive.sh` keeps one `strata --qa` session open inside the pinned
container for interactive verification before or alongside writing a scenario.
`start` builds the QA binary, seeds a fresh `HOME`, XDG directories, and fixture
from `tests/fixtures/harness.json`, and prints the launch reply. Subsequent
subcommands run as fast container `exec` calls:

```bash
./scripts/qa-drive.sh start --tree '{"notes":{"plan.txt":"plan\n"}}' --pref browser_mode=list
./scripts/qa-drive.sh send '{"select":{"entry":"notes"}}' '{"query":"state"}'
./scripts/qa-drive.sh restart --pref text_size=large
./scripts/qa-drive.sh exec 'ls -A; cat "$XDG_CONFIG_HOME/strata/settings.toml"'
./scripts/qa-drive.sh log
./scripts/qa-drive.sh stop                      # removes the container
```

`send` writes each process command on stdin and prints its reply line. Runner
steps (`fs`, `layout`, `restart`, etc.) are driven by subcommands
rather than `send`. Every exchange is appended to `target/qa-drive/transcript.jsonl`,
and logs land in `target/qa-drive/`. Because it shares the
`target/qa-container` build cache with `scripts/qa.sh`, never run them concurrently.

### Proving a scenario catches a regression

`scripts/qa-mutation.sh` applies one deliberate defect patch at a time from
`tests/qa/mutations/`, rebuilds, and verifies that the scenario whose header
names it as `mutation` catches the regression:

```bash
./scripts/qa-mutation.sh                                 # every patch under tests/qa/mutations
./scripts/qa-mutation.sh shift-click-range               # every patch for a scenario
./scripts/qa-mutation.sh pdf-zoom-keeps-text-size.window-wheel  # one labelled patch
```

The chosen scenarios first run on the unmodified tree and must pass; under the
patch, a mutation counts as caught only when at least one run fails a scenario
step and `qa.sh` exits nonzero. Reports go to `target/qa-mutation/summary.md`.
The script refuses dirty target files and restores only patched files on exit.
Run it when changing a scenario with a `mutation`, when its patch stops applying,
or when changing how the runner judges a step.

### Skipped scenarios

A scenario whose outcome cannot yet be relied upon carries a `skip` reason until
fixed; it stays in the suite and runs when explicitly named.

### Waits for the app's own timers

QA input arrives faster than human interaction. Where that pace exposes a known
timer window (for example, Columns mirror delay, rename undo publication, or
deferred progress error detail), the scenario waits explicitly on the expected
state rather than racing an arbitrary sleep.

## Compositor, D-Bus, and windowed tests (`mise run test-platform`)

`strata-platform`'s Linux shims and `strata`'s real windowed binary run against
a real Wayland compositor and session bus via `crates/strata-platform/tests/support/`:

- a private `dbus-daemon` whose only service directory is the session's;
- a nested headless `sway` (`WLR_BACKENDS=headless`) with `Xwayland`;
- a virtual pointer and virtual keyboard peer (headless seats have neither);
- a private `XDG_RUNTIME_DIR` and `HOME` cleaned up on teardown (logs stay in
  `target/tmp/session/<test>/`);
- a watchdog process that scans `/proc/*/environ` for the session's
  `XDG_RUNTIME_DIR` and tears down lingering processes, sockets, and mounts if
  a test is killed abruptly.

Every process starts from a clean environment; `mise run test-platform` saves
outer `DISPLAY`, `WAYLAND_DISPLAY`, and `XDG_RUNTIME_DIR` as `STRATA_OUTER_*`
and the harness refuses any session display that matches them.

| Command | Runs |
| --- | --- |
| `mise run test-platform` | Every compositor/D-Bus/window session test |
| `mise run test-platform window::` | `strata`'s windowed tests (`tests/window.rs`), including `window::qa` |
| `mise run test-platform p2::` | One `strata-platform` shim's session tests |
| `mise run test-platform -- w3:: --test-threads=1 --nocapture` | Forwarded libtest arguments |

`mise run test-platform` builds `strata-test-peer`, `strata-platform`'s examples,
and `strata-gallery`'s `dnd_wiring`/`clipboard_wiring` examples, then runs the
`#[ignore]`d session tests in `strata-platform`, `strata-gallery` (`x2`, `x3`),
and `strata` (`tests/window.rs`). `mise run test` and `mise run check` never
start a compositor session.

| Environment variable | Effect |
| --- | --- |
| `STRATA_VK_ICD` | Vulkan ICD file for clients (defaults to lavapipe if installed, else default driver) |
| `STRATA_WLR_RENDERER` | `sway` renderer (defaults to `pixman` with lavapipe, else `gles2`) |
| `STRATA_TEST_TIMEOUT_SCALE` | Multiplies 10 s waits (`mise run test-platform` defaults to `3`) |
| `STRATA_SWAY_DEBUG=1` | Enables `sway` debug logging in `sway.log` |
| `STRATA_TEST_PEER` | Overrides path to `strata-test-peer` |
| `STRATA_TEST_NAUTILUS=1` | Opts into best-effort Nautilus drag/clipboard interop tests in `x2`/`x3` |
| `STRATA_PLATFORM_TONE=1` | Opts into the audible 440 Hz output test in `strata-platform`'s `media-audio` |

`strata-test-peer` (`crates/strata-platform/tests/support/strata-test-peer/`) acts
as the second Wayland/X11 client for clipboard, INCR transfer, Wayland/XDND drag
and drop, foreign window parenting, and virtual input injection.

### `strata-platform` feature isolation

`mise run compile-features` checks `strata-platform` with `--no-default-features`
and then with each individual feature enabled alone, ensuring no shim depends
implicitly on another shim's optional crates.

### Component gallery (`strata-gallery`)

`crates/strata-gallery` is an interactive showcase of `strata-components` pages
and wiring examples (`dnd_wiring`, `clipboard_wiring`, `shell`):

- Open interactively with `mise run gallery` (or `mise run gallery -- --scene <scene>`
  and `-- --list-scenes`).
- `mise run test` runs `strata-gallery`'s headless unit and smoke tests
  (`tests/gallery.rs` plus the unit tests in `src/` and `examples/`).
- `mise run test-platform` runs `strata-gallery`'s `#[ignore]`d compositor
  drag-and-drop (`x2`) and clipboard (`x3`) session tests.
- Primary end-to-end UI and behavioral verification lives in the QA scenarios
  (`tests/qa/scenarios/`, run via `./scripts/qa.sh`).

Give each Git worktree its own `target/` directory rather than sharing one
`CARGO_TARGET_DIR`, so dep-info paths never cause stale builds across worktrees.

## Main-thread I/O audit

Strata must not perform blocking user-filesystem I/O on the UI thread (see
[`docs/architecture.md`](architecture.md)). Runtime asserts catch calls through
Strata's own seams; two opt-in tools catch the rest, including GLib internals
and drops:

```bash
./scripts/trace-main-io.py                                  # every scenario
./scripts/trace-main-io.py delete-to-trash
./scripts/trace-main-io.py --stacks rename-commits-on-enter/list
./scripts/trace-main-io.py --analyze target/main-io/<run>   # reclassify saved traces
```

`scripts/trace-main-io.py` runs a QA selection through `scripts/qa.sh` with
`STRATA_QA_MAIN_IO` set, tracing only the UI thread under `strace`. Calls are
classified into **user** (always a violation), **settings** (violation unless
allowlisted in `tests/qa/main-io-allowlist.toml`), **toolkit** (ignored), and
**unclassified** (reported for deliberate classification).

For non-syscall stalls (such as blocking locks or synchronous D-Bus calls), set
`STRATA_UI_STALL_MS` (for example `250`, or `--stall-ms 250` on `trace-main-io.py`)
to run a watchdog thread that logs the UI thread's `/proc` wait state whenever
the loop stalls past the threshold.

## In CI

### QA in CI

The **QA scenarios** job in `.github/workflows/ci.yml` runs the QA suite in the
pinned container, verifies the `qa` build contains the QA usage sentinel (which
the release workflow rejects in release binaries), and uploads `target/qa-artifacts`
on failure. CI does not run `scripts/qa-mutation.sh`.

### Persistent published environments

`Publish the pinned test image` (`test-image.yml`) publishes
`ghcr.io/lgse/strata-test-image` from trusted `main` when `tests/container/`,
Cargo manifests, or `.dockerignore` change, or on manual dispatch. The image
holds the pinned Arch Linux snapshot, Rust toolchain, runtime/compositor
packages, and locked Cargo dependencies (with stubbed workspace manifests
cleaned after precompiling dependencies).

Only the trusted `main` publisher holds `packages: write`; PR CI pulls
anonymously and builds an attempt-scoped candidate environment only when a PR
deliberately edits `tests/container/`.

### Maintaining behavioral coverage

Before consolidating tests, identify the retained behavioral owner for every
assertion. Fewer test functions or scenario runs do not establish a runtime
improvement. Preserve functional input-routing, filesystem-safety,
live-preference, and lifecycle regressions, even when they share fixtures.
Record task-specific inventories and consolidation decisions in the issue or PR,
not in a committed audit report.
