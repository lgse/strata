# Testing

The Rust suite covers state transitions, filesystem edge cases, and individual
widgets. The QA scenarios in `tests/qa/scenarios` drive the real GTK
application on a headless X display, send real keyboard and pointer input from
inside the process, and check both what the window shows and what happened on
disk. Both run in one pinned container image.

## Running the suites

```bash
./scripts/quality.sh                          # formatting, Clippy, and every Rust test
./scripts/test-headless.py relevant::module   # a native Rust selection on a private display
./scripts/qa.sh                               # every QA scenario, every mode
./scripts/qa.sh copy-leaves-source            # one scenario, all of its runs
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
`./scripts/quality.sh clippy` on the final code at that checkpoint; rerun affected
checks if fixes change the code before the push. Even when full Rust tests are
needed during iteration, use `./scripts/quality.sh test` and defer lint/format
phases until pre-push. CI's lint and formatting checks remain unchanged.

The repository owner may explicitly authorize pushing a PR without the normally
required local tests for the current change/push. A general push request,
urgency, silence, or consent for another push does not qualify. Record the
explicit authorization, omitted suites, and known failures or unverified behavior
in the handoff and PR description; never report skipped tests as passing. This
exception does not waive lint/format checks, GUI isolation, or required CI/merge
checks. Renew consent if the authorized scope changes, or run the required tests.

For native Rust selection, `scripts/test-headless.py` starts the private display
and session buses, then appends its arguments to the fixed
`cargo test --all-targets --all-features` command. Use module/name filters:

```bash
./scripts/test-headless.py services::operations::tests
./scripts/test-headless.py basenames_reject_empty_reserved_nested_absolute_and_nul_names
```

These select test execution across targets, not compilation to one target.
Targeted runs can still incur a full test build; Cargo's cache helps subsequent
iterations. There is no automatic changed-code dependency-to-test mapping.

Controller regressions live under `src/app/browser/tests/`, grouped by navigation,
location input, selection, monitor updates, preferences, operations, undo, metadata,
staging, event dispatch, and archive activation. Add cases to the matching owner
rather than to `src/app/browser/tests.rs`, which holds shared source/provider
fixtures. Existing adjacent feature test modules retain their ownership. For
example, `./scripts/test-headless.py app::browser::tests::undo::` selects undo
history/claim regressions; `app::browser::tests::metadata::` selects metadata-fill
and sort-terminal regressions. Keep distinct input routes and lifecycle cases
separate even when they share fixtures.

Local-operation regressions have owners under `src/adapters/local_operations/tests/`
for path resolution, copying, moving, replacement, conflicts, progress, naming,
paste results, undo, deletion, and restore safety. Shared fixtures and the manual
deletion benchmarks remain in the parent `tests.rs`; keep the benchmark names
stable for `scripts/benchmark-delete.sh`. For example,
`./scripts/test-headless.py adapters::local_operations::tests::replacement::`
selects replacement safety cases. Preserve independently named filesystem races,
symlink, non-UTF8, cancellation, and conflict scenarios when sharing setup.

Window regressions have policy owners under `src/ui/window/tests/` for keyboard
classification, sidebar places, bookmarks, devices, and trash. Startup and live
preference cases stay in `preferences`; real key-routing scenarios stay in
`keyboard_dispatch` and `type_to_search`. The `ui::window::` filter covers these
owners and the adjacent window modules. When moving a GTK case, update its
`gtk_test` subprocess name to match its compiled test path and verify that the
child actually executes the case; an unchanged outer test count is not enough.

A filter must collect at least one test; use Cargo's output or a collection
check to verify that it did. `scripts/quality.sh` only accepts `all`, `fmt`,
`clippy`, or `test` and does **not** forward test filters, so it cannot be used
for a targeted test selection.

`./scripts/qa.sh` takes scenario names, or `<scenario>/<run>` for one run, and
refuses a name no scenario file has, listing the names it knows; see
"Scheduling and run costs" for `--skip` and the other options. A selection
that runs no scenario fails. Preserve the verified image provenance and the
`target/qa-container` and `target/quality-container` caches. CI still runs its
complete gate.

Every GUI or delegated check must clear inherited display variables and use a
private Xvfb and private D-Bus session. `scripts/test-headless.py` and
`scripts/qa.sh` enforce this isolation. Rust tests set `GTK_A11Y=none`,
`NO_AT_BRIDGE=1`, and `STRATA_REQUIRE_GTK_TESTS=1`; QA runs set `GTK_A11Y=none`
and `NO_AT_BRIDGE=1` too, since the process reads its own widget tree. Other
commands must arrange equivalent isolation. A missing isolated display or bus is
a hard failure. Never run against the desktop or an inherited session bus,
silently skip GTK tests, or fall back to the desktop.

### The pinned test image

The runners prefer Podman when available; select an engine explicitly with
`STRATA_CONTAINER_ENGINE=podman` or `docker`. CI explicitly selects Docker; an
image in one engine's store is not visible to the other. Normal runs verify and
reuse the local base image. If missing, they pull the published environment
once, verify its input label and platform, and record it locally. They **never
automatically build images or fetch distribution packages**. A missing
publication fails with an actionable message rather than silently bootstrapping.

When intentionally updating the environment, or before its first publication,
explicitly build the local base once:

```bash
STRATA_CONTAINER_ENGINE=podman python3 scripts/image_base.py build
./scripts/qa.sh
```

The recipe is `tests/container/Dockerfile`: a digest-pinned Arch Linux base, the
dated 2026-09-11 Arch package snapshot, GTK 4.22.5, GLib 2.88.3, GStreamer
1.28.7, and Rust 1.98.1. Pacman verifies the snapshot's signed packages. The
image replaces only GTK's versioned shared library with a matching-source build
carrying the accessibility-coordinate patch documented in
[`tests/container/gtk/README.md`](../tests/container/gtk/README.md); an
image-build probe checks the loaded library and rendered/accessibility
coordinates on a private display. `scripts/image_inputs.py image-key` hashes
those inputs, so changing them selects a new base; application edits do not.
The runners still compile the checked-out application and retain Cargo's
worktree cache. No host GTK libraries, fonts, desktop sockets, or Rust binaries
are mounted. Python in the image is the system `python3` with PyGObject, for the
coordinate probe and the helpers some scenarios start; there is no Python test
environment.

The checkout is mounted at `/workspace`. QA build and Cargo caches live in
`target/qa-container` (quality's in `target/quality-container`), separately
from native builds, and artifacts are owned by the invoking user. Minimal
generated passwd/group files provide the invoking UID/GID to D-Bus, so one
published environment works across local user IDs without rebuilding it or
mounting the host's account database.
The image includes bubblewrap for sandboxed thumbnail decoding, FFmpeg/ffprobe,
GTK's GStreamer media backend with the base/good/libav plugins, and strace for
the opt-in [main-thread I/O audit](#main-thread-io-audit). Rust media
regressions exercise actual normalization, playback, and long-source duration
limits rather than skipping when optional host tools are missing. These packages
come from the dated Arch snapshot. The accessibility patch is test-only and does
not apply or retire the version-specific GTK/GstPlay work in
`packaging/media-runtime/`; that opt-in kit remains unchanged and is not shipped
by this image.

Rootless Podman
runs unmask `/proc/*` inside the test container so bubblewrap can mount its own
private `/proc`; the decoder's sandbox and the container's seccomp policy remain
enabled. Docker's outer seccomp/AppArmor profiles and system-path masks must be
disabled for the nested namespace and mount operations. This applies only to the
disposable test container; Strata still launches its normal bubblewrap decoder
sandbox. Neither engine uses privileged mode or mounts desktop sockets.
Updating the image inputs is an intentional rendering
environment change and requires reviewing the visual baselines.

### Shared environment for formatting, lint, and Rust tests

`./scripts/quality.sh` runs all three quality phases in the same verified build
base. Use `./scripts/quality.sh fmt`, `clippy`, or `test` for an individual phase.
No separate quality image is necessary: the published environment already has
Rust 1.98.1, rustfmt, Clippy, native development libraries, and Xvfb. Quality now
uses that pinned compiler rather than moving `stable`.

CI resolves the public environment to a manifest digest, checks its input labels
and platform, and pulls that exact digest without registry credentials. Each
phase verifies the loaded image again and runs its immutable image ID. An
unavailable unchanged environment fails explicitly instead of installing packages.
For a deliberate recipe-input change relative to the PR base (or previous main
commit), CI may explicitly build that unpublished candidate from the pinned
recipe, without publishing it. This lets environment-update PRs pass before the
trusted-main publisher runs; it does not turn registry outages into repeated
distribution bootstraps. Locally, environment builds still require the explicit
command in "The pinned test image".

Quality's Cargo home and build directory are under `target/quality-container`,
separate from the QA cache and native builds. Its Actions cache is keyed by environment,
Cargo manifests, and source revision, with same-environment/manifest restoration.
Only successful quality builds on main pushes save caches; PR consumers cannot
populate main's cache. These are compilation caches, not evidence of passing tests;
the aggregate gate separately requires all test shards to pass. Cold quality
compilation includes test-only and all-feature dependencies, so the published
dependency layers are not a full quality hit.

The Rust suite runs with `--all-targets --all-features --locked` inside private
Xvfb, with `GTK_A11Y=none`, `NO_AT_BRIDGE=1`, and `STRATA_REQUIRE_GTK_TESTS=1`.
The shared Xvfb server uses `-noreset` so consecutive test processes do not
race a server reset when the last display client exits. GTK initialization
failures cannot silently skip tests. The container's `/tmp`, which holds `HOME` and
the tests' temporary folders, is an executable tmpfs: copy tests need `O_TMPFILE`
there, which an overlay `/tmp` may refuse. Formatting, compiler,
lint, and test failures remain blocking. Lightweight policy/helper jobs retain
their existing runners rather than downloading a large GUI image unnecessarily.

### Rust quality shards

CI's **Quality build and lint** job runs formatting and Clippy, then compiles
`cargo test --locked --all-targets --all-features --no-run` once. It exports only
test executables and a plan, not Cargo caches. `scripts/quality_ci.py` collects
each libtest inventory, including the explicitly ignored tests, and assigns every
entry to one of four shards. Timing hints in `scripts/quality-durations.json`
come from successful GTK child runs in
[run 34560353003](https://github.com/lgse/strata/actions/runs/34560353003).
Tests are balanced longest-first across the shards; unknown tests receive a
one-second weight and always participate. Timing hints are not an allowlist.
The deferred-scroll timing was refreshed to 1.32 seconds from
[run 34811363582](https://github.com/lgse/strata/actions/runs/34811363582).
`ui::search::tests::deferred_scroll_restoration_yields_to_updates_wheel_scrollbar_and_query_reset`
runs in its own libtest process before the other tests assigned to its shard,
not on a dedicated runner. Each process must pass its complete selection before
an aggregate shard receipt is written. Validation rejects a missing, duplicated,
or ignored isolated test; renaming or removing it requires updating `ISOLATED_TEST`.
Sharding does not shorten an individual test.

Each **Rust tests shard N** verifies the checkout revision, application/Rust-test source
fingerprint, image inputs, executable checksums, and the entire libtest inventory
before selecting exact names. Tests share neither a display nor a session bus
with another shard. The existing per-process GTK serialization still applies.
The runner checks libtest's actual selection and final passed/ignored counts
before writing a success receipt; nonzero exits, empty runnable shards, or changed
inventories fail. Existing `#[ignore]` entries stay explicitly accounted for;
sharding neither enables them nor silently ignores additional tests. Unsupported
non-libtest harness inventories fail closed rather than disappearing from coverage.
The QA runner, `tests/qa.rs`, has its own harness (`harness = false`, see
"Scheduling and run costs") that keeps libtest's `--list`, `--ignored`,
`--exact`, and `test result:` formats, so its scenario runs are inventoried as
explicitly ignored tests and its unit tests run in the shards like any other.

Published environments are pulled by the build job's exact manifest digest and
verified again by `quality.sh`. Deliberate unpublished recipe updates transfer
the same locally built environment as an attempt-scoped artifact instead; shards
never rebuild it. **Format, lint, and test** retains the required-check name,
requires successful build/lint and every shard, and verifies no missing, extra,
duplicate, or stale receipts. Failed shards are not retried, and `fail-fast: false`
preserves the other shards' results. Reports and logs are attempt-scoped artifacts.

To reproduce the handoff locally using the same pinned container:

```bash
STRATA_QUALITY_TASK=build ./scripts/quality.sh test
for shard in 0 1 2 3; do
  STRATA_QUALITY_TASK=shard STRATA_QUALITY_SHARD="$shard" ./scripts/quality.sh test
done
python3 scripts/quality_ci.py verify
```

The example runs shards sequentially for convenient local diagnosis; CI runs them
in parallel on separate runners. These environment variables are CI handoff modes,
not test filters; ordinary `./scripts/quality.sh test` still runs the complete
unsharded suite. Rebuild the bundle after changing source or checkout revision.
The four-shard matrix and `SHARDS` constant must be updated together if tuning
fan-out. Each shard has a ten-minute hang bound; timing is otherwise informational.

### Keep Rust test windows off the local desktop

Some Rust tests also create GTK windows when a display is available. Run the
complete suite, or a justified targeted selection, on the same private
Xvfb/private D-Bus infrastructure with:

```bash
./scripts/test-headless.py
./scripts/test-headless.py -- --nocapture
./scripts/test-headless.py relevant_module_or_test_name
```

This requires Xvfb, D-Bus, and AT-SPI on the host. It isolates application
preferences while retaining access to the installed Cargo/Rust toolchains, and
disables accessibility bridging for Rust tests. A startup failure aborts; it
never falls back to the real display or an inherited session bus. Its private
display, buses, and throwaway HOME come from `scripts/headless/`, which
`scripts/benchmark-scroll.py` uses as well.

### Native debugging dependencies

These are installed inside the canonical image; only `scripts/test-headless.py`
and other native debugging need them on the host.

| Purpose | Arch | Debian / Ubuntu |
| --- | --- | --- |
| Headless X server | `xorg-server-xvfb` | `xvfb` |
| Accessibility bus and registry | `at-spi2-core` | `at-spi2-core` |
| Private session bus | `dbus` | `dbus-daemon`, `dbus-bin` |
| Keyboard layouts | `xorg-setxkbmap` | `x11-xkb-utils` |
| Screen capture | `imagemagick` | `imagemagick` |
| Font | `cantarell-fonts` | `fonts-cantarell` |

## QA scenarios (`--qa` mode)

A build with the `qa` Cargo feature (never in releases) accepts
`strata --qa <directory>`: the normal window opens on the directory, then the
process reads one JSON command per line on stdin, runs it on the GTK main loop,
and writes one JSON reply per line on stdout after the main context drains.
Input is real XTEST sent from inside the process, and state is read from the
widget tree by the names `src/ui/accessibility.rs` sets, so no AT-SPI client is
involved.

```bash
./scripts/qa.sh                              # every scenario, every mode
./scripts/qa.sh arrow-keys-move-focus        # one scenario, all of its runs
./scripts/qa.sh arrow-keys-move-focus/columns-arrows   # one run of it
STRATA_QA_KEEP_ARTIFACTS=1 ./scripts/qa.sh   # artifacts for passing runs too
```

`scripts/qa.sh` runs `cargo test --features qa,invariant-checks --test qa` in
the pinned image,
with the mounts, account files, and `target/qa-container` cache it shares with
`scripts/qa-drive.sh` and `scripts/qa-mutation.sh` (through
`scripts/lib/qa-container.sh`). Because the cache is shared, run them one after
the other, never concurrently. `tests/qa.rs` refuses to start without the
`STRATA_QA_CONTAINER=1` that script sets. A host run needs the owner's explicit
consent for the task and `STRATA_QA_HOST=1`; it is labelled unsupported, is
never validation evidence, and must be recorded in the handoff with the
container run it replaced. The QA runner is not wired into
`scripts/test-headless.py`.

### Wayland and input methods (host runs)

Two optional parts of a host run reproduce bugs the pinned image cannot reach,
such as input-method composition (issue #1715). Neither is validation evidence,
and the pinned image has neither sway nor fcitx5.

- `STRATA_QA_DISPLAY=wayland` gives every run a private headless sway
  (`WLR_BACKENDS=headless`, no input devices, no XWayland, software rendering)
  in the run's runtime directory instead of Xvfb, and launches the application
  with `GDK_BACKEND=wayland`. The process then sends its input through a
  Wayland virtual keyboard (with a US keymap it compiles) and virtual pointer
  instead of XTEST. sway floats every window in the output's top-left corner,
  where the window-manager-less X display leaves it, and `screenshot` uses
  `grim`. `layout` steps other than `us` and pixel sampling (pixel
  predicates and drop samples) are X11-only and fail with a message. It needs
  `sway`, `swaymsg`, and `grim`.
- A header's `"input_method": "<name>"` (with `session_bus`) starts a private
  fcitx5 on the run's display and bus, with a profile holding the US keyboard
  and that input method; Ctrl+Space switches to it. Choose the toolkit path
  with the header's `env`, for example `"GTK_IM_MODULE":"fcitx"` for
  fcitx5-gtk's module or `"GTK_IM_MODULE":"wayland"` for text-input-v3. The
  input method's addon must be installed (`fcitx5/inputmethod/<name>.conf`),
  and its log is kept as `fcitx5.log`.

```bash
STRATA_QA_HOST=1 STRATA_QA_NAMED=1 STRATA_QA_DISPLAY=wayland \
  cargo test --features qa,invariant-checks --test qa -- \
  --include-ignored --exact input-method-composes-over-selection
```

Scenarios that need an input method carry a `skip` reason, so the default
container run leaves them out.

`invariant-checks` turns on exact but slow consistency assertions (for example,
the selection position cache against a full listing scan). `scripts/qa.sh` and
`scripts/qa-drive.sh` build with it, and `cargo test --all-features` enables it
too; plain `cargo build` leaves it off so development builds keep their normal
speed.

### How a run is set up

Each run of a scenario gets:

- its own Xvfb, at 1440x900x24 and 96 DPI;
- a fresh fixture tree in its own temporary directory under `/tmp`, with fixed
  modification times, materialised from the shared manifest
  `tests/fixtures/harness.json`, which also defines the seeded preferences,
  GTK settings, static environment, and keyboard layouts;
- a throwaway `HOME` and XDG directories, seeded with a complete
  `settings.toml` so no preference is inherited from a default change, and with
  the first-run file-chooser portal offer already answered;
- with `"session_bus":true`, a private D-Bus session with no service
  directories, so nothing is D-Bus-activated behind the run's back; without
  it, a bus address that cannot connect.

Rendering is pinned by the manifest: `GSK_RENDERER=cairo`, software GL,
`GDK_SCALE=1`, the Adwaita icon theme, Cantarell 11, `C.UTF-8`, `UTC`,
animations off, and `reduce_motion` on. Application preferences and file
operations use only the run's temporary HOME/XDG directories and fixture.
Both are under `/tmp` so trash capabilities do not vary with the caller's
`TMPDIR`. System libraries, fonts, and icon assets are provided by the
container; desktop endpoints and user configuration overrides are not
inherited.

The application's `MESA_SHADER_CACHE_DIR` names one directory the runner makes
under `/tmp` and removes when it exits, so the software GL shaders GTK compiles
at start-up are built once per `./scripts/qa.sh` rather than in every run's
fresh `XDG_CACHE_HOME`; helper and `foreign` processes a step starts run
without it. `scripts/qa.sh` mounts a tmpfs on the container's `/tmp`, which
holds every run's fixture, HOME, and runtime directory. `/tmp` is then a mount
point of its own, the one Strata resolves for a path there instead of `/`;
fixtures and HOME share one device, so Trash and its restore work within it,
and `/dev/shm` is another volume. Nothing in `/tmp` outlives the container,
since artifacts and the Cargo cache are under `/workspace/target`. The tmpfs
takes the kernel's default limit, half of RAM; a full run at 24 threads peaked
near 150 MB.

### Scenario files

A scenario is a `.jsonl` file under `tests/qa/scenarios/<feature area>/`, a
folder named in snake_case without a `test_` prefix, such as
`tests/qa/scenarios/clipboard/copy-leaves-source.jsonl`. The runner finds every
file there itself. Scenario names are lowercase words joined by hyphens, since
`./scripts/qa.sh` reads `_` in a name as `-`, and are unique across folders.
The first line is a header and every later line one step:

```json
{"scenario":"arrow-keys-move-focus","modes":["columns"],"prefs":{"type_to_search":false},"layout":"us","mutation":"keyboard-navigation","known_bug":null,"variants":[...]}
```

`modes` runs the file once per view, and optional `variants` run it again with
their own preferences, layout, tree, or variables; a run is named
`<scenario>/<label>`, as in `PASS copy-leaves-source/columns-ru`. `mutation`
names the patch that must make the scenario fail (see "Proving a scenario
catches a regression"), `known_bug` an issue that makes a failure the expected
result, `skip` keeps a scenario out of the default run (see "Skipped
scenarios"), `cost` orders the runs (see "Scheduling and run costs"), and
`quality` rates the scenario as the `qa-quality` skill defines. Steps run
either in the process on the GTK main loop (input, waits, and checks on the
window) or in the runner (filesystem setup and checks, restarts, layouts, and
peers such as a portal request or a foreign drag source).

The runner validates every file before any run starts: the header rejects a key
it does not know, each step goes through the process's own parser, and the
`every_scenario_file_is_planned_and_well_formed` test checks that each file
sits in a well-named folder, loads, names an existing mutation patch, and has
every run planned as a test. A typo fails as a harness error, never as a step a
`known_bug` could excuse.

The full catalogue of header keys, variables, steps, `fs` kinds, targets,
predicates, replies, the state snapshot, the input log, and artifacts is
[`.agents/skills/qa-scenarios/reference.md`](../.agents/skills/qa-scenarios/reference.md);
the parsers it names are the source of truth. The `qa-scenarios` skill beside
it covers writing, running, and debugging a scenario.

### Accessible names are product surface

The runner finds an entry because Strata names it. Those names live in
`src/ui/accessibility.rs` and exist for screen readers first: an entry row is
labeled with its name and described as `Folder` or `File`, a pane is labeled
with its directory and described with its presentation, the focusable surface
of an empty, unreadable or loading pane is labeled with its directory and
described with its status text, menu items carry the
menu role with the accelerator in the description, and modal dialogs carry the
dialog role and a name. When a scenario cannot identify a control, the fix is
to name it in the application, not to reach around the accessibility layer.
Locate controls by role and name, never by a literal screen coordinate, and
synchronize on conditions (`wait`, `fs` with a timeout), never on `sleep`
where a condition can be read. Check the filesystem as well as the window for
every file operation.

### Keyboard layouts

A `{"layout":"ru"}` step switches the run's Xvfb to a layout from the
manifest while the application runs: `setxkbmap -display` loads the layouts,
and their XKB variants when the layout names them, and an XKB group lock
selects the active one, as a layout switch shortcut does. The header's
`layout` is switched to once the process is ready. A `{"layout":"us"}` step,
and the end of a run under another layout, unlock the group and every
modifier and restore the US layout without variants; a keyboard not back in
that state fails the run. Never
change the desktop's layout, and never run `setxkbmap` without `-display`.

Key names mean US key positions: under `ru`, `{"key":"ctrl+c"}` presses the
key that types с, and `type` types what those keys produce. The `altgr`
modifier presses the US right Alt key, which layouts with AltGr turn into it:
under `cn-altgr-pinyin`, `altgr+1` types the macron dead key. The runner starts
no input method daemon. GTK's built-in input method composes dead keys in text
fields, so a pending Pinyin tone mark stands in for an input method's
unfinished composition when a scenario checks which keys reach it.

### Scheduling and run costs

`tests/qa.rs` makes every run of every scenario a test of its own, named
`<scenario>/<run>` as in its `RUN` line (`copy-leaves-source/columns-ru`), so
the runs of one scenario spread over the threads, each with its own Xvfb and
process. Its harness, `tests/qa/harness.rs` (`harness = false`), takes
libtest's command line and output but starts the tests in cost order, not
alphabetically: costliest first, a run without a `cost` before all of them
(it may be costly), and equal costs by name. It runs `--test-threads` tests at
once, one per CPU by default. A run with a `fixture_root` never starts while
another such run is going; the next run free to start takes the thread
instead of waiting. A run's cost outside its steps (Xvfb, the launch up to
the first reply, and shutdown) is about 0.4 s at 24 threads, so it takes no
display lock and no 50 ms exit poll: concurrent Xvfb starts begin at
different display numbers, skip one with a lock file or socket, and try the
next when another server takes theirs first, as `-displayfd` shows; an
exiting application, Xvfb, or bus is awaited on its pidfd; and `strata --qa`
checks that its window is ready after each frame it paints. With `--exact`,
which `./scripts/qa.sh` adds when it names anything, a scenario name selects
all of its runs and never another scenario it prefixes, and
`<scenario>/<run>` selects one run; without it a filter is a
substring, as in libtest. `./scripts/qa.sh --skip <scenario>` (or
`<scenario>/<run>`, in either spelling) passes the harness's own
`--skip-exact`, which leaves out that scenario or run and never another
scenario it prefixes; any other `--skip` is libtest's substring, or a whole
name once a scenario is named. Nothing is captured: as each test finishes, the
harness prints its result line with the time it took,
`test <name> ... ok <12.345s>` (or `FAILED <12.345s>`), and the run's `PASS`
or `KNOWN BUG` line, each line whole, and at the end the failure details and
libtest's `test result:` summary. `--list` prints the selected tests in cost
order, which is the order they are offered in; a run with a `fixture_root`
may start later while another such run is going. `--help` lists the options;
libtest's `--report-time`, `--shuffle`, and `--shuffle-seed` are accepted and
change nothing, as cost order wins.

The runner's own unit tests live in `tests/qa/scenario_tests.rs` and
`tests/qa/step_tests.rs`, and each must be listed in the `unit_tests!` macro in
`tests/qa/unit_tests.rs`: with the harness replaced, `#[test]` and
`#[cfg(test)]` there are stripped without a warning and never run, so the
`no_test_attribute_hides_a_runner_test` test rejects them in `tests/qa.rs` and
`tests/qa/*.rs`.

A header's optional `cost` maps every run label of the scenario, and nothing
else, to a positive number: the run's median wall-clock time over several full
runs divided by the median of all runs' medians, so 1.0 is a typical run,
rounded to one decimal place and at least 0.1. The runner rejects a `cost`
that misses a run or names another. Refresh the costs after adding scenarios
or runs, or when the order no longer fits:

```bash
python3 scripts/qa_cost.py run                               # three full runs, then every header's cost
python3 scripts/qa_cost.py run --repeat 5
python3 scripts/qa_cost.py write target/qa-cost/run-*.log    # again from saved qa.sh logs
```

`run` times full `./scripts/qa.sh` runs at the default thread count, naming
every scenario so a skipped one is timed too, and keeps their logs in
`target/qa-cost/`. A failed run's time counts like a passing one's, and
nothing is written unless every planned run has a time. Only the `cost` value
of each header line changes, added last where there is none; every other byte
of the files stays. The order matters more than the numbers: a new run without
a cost starts early until the next refresh.

### Replies and failure artifacts

Every process command gets one JSON reply carrying the `app` state snapshot,
and a GLib warning or critical in any reply fails the run. A failed run, or
every run under `STRATA_QA_KEEP_ARTIFACTS=1`, writes
`target/qa-artifacts/<scenario>/<run label>/`: a screenshot, the widget tree,
the command and reply transcript, the application, Xvfb, and input logs, the
fixture listing, the final snapshot, and the result. The reference's
"Protocol, replies, and the snapshot" and "Artifacts" sections define each.

### Driving a session by hand

`scripts/qa-drive.sh` keeps one `strata --qa` session open in the pinned
image, for checking a change interactively before (or instead of) writing a
scenario. `start` starts a detached container with the same engine, image,
mounts, account files, and `target/qa-container` cache as `scripts/qa.sh`
(both take them from `scripts/lib/qa-container.sh`), makes the
build `qa.sh` makes (`cargo test --features qa,invariant-checks --test qa --no-run`), seeds a
fresh HOME, XDG directories, and fixture from `tests/fixtures/harness.json` as
the runner does, starts Xvfb on `:90`, and launches the application in the
fixture with the runner's environment; it prints the launch reply. A start
that fails at any point removes its container. Each later subcommand is a
short `docker exec` (or `podman exec`) that returns at once:

```bash
./scripts/qa-drive.sh start --tree '{"notes":{"plan.txt":"plan\n"}}' --pref browser_mode=list
./scripts/qa-drive.sh send '{"select":{"entry":"notes"}}' '{"query":"state"}'
./scripts/qa-drive.sh screenshot menu.png      # target/qa-drive/menu.png
./scripts/qa-drive.sh restart --pref text_size=large
./scripts/qa-drive.sh exec 'ls -A; cat "$XDG_CONFIG_HOME/strata/settings.toml"'
./scripts/qa-drive.sh log
./scripts/qa-drive.sh stop                      # removes the container
```

`send` writes each process command on the application's stdin and prints its
one-line reply, waiting 90 s, or the command's `timeout_ms` plus 30 s; it
reports a process that exited with its log tail. Replies are matched to
commands by number, so the reply to a command whose wait gave up is skipped
when it comes, never printed as a later command's. Runner steps (`fs`,
`layout`, `screenshot`, `restart`, and the others the reference lists as runner
and `fs` steps) are not process commands: use the subcommands, or a scenario.
`--tree` takes a tree as the manifest writes one (a name, or JSON, or `@file`), `--pref KEY=VALUE` changes a
seeded preference (the value is JSON, or else a string), `--env NAME=VALUE`
adds a variable, and `--bus` starts the runner's private session bus in place
of the address that cannot connect. `exec` runs a shell command in the
fixture with the session's HOME, XDG directories, display, runtime directory,
and bus. `restart` closes the application's stdin, signals its group if it
has not quit within ten seconds (the runner's `EXIT_TIMEOUT` and
`TERMINATE_GRACE`), rewrites the given
preferences in its saved `settings.toml`, and launches it again on the same
fixture and HOME. Every exchange is appended to
`target/qa-drive/transcript.jsonl` in the artifacts' transcript format, and the
input log, build log, screenshots, and (after `stop`) application log land in
`target/qa-drive/`, which `start` empties. There is one session per checkout,
and it shares the build cache, so never run it while `scripts/qa.sh` runs.
The values it copies from `tests/qa.rs` (the QA environment, XDG layout,
Xvfb arguments, bus configuration, and timeouts) are checked by
`scripts/test_qa_drive.py`.

### Visual baselines

The scenarios in `tests/qa/scenarios/visual_baselines/` compare a small set of
stable states with the images in `tests/qa/baselines/gtk-4.22` through a
`baseline` step: one canonical fixture in each view, the airy Icons spacing, a
hovered Icons tile, a
selection with focus, an open context menu, a confirmation dialog, and the
default Columns overflow. Local and CI runs use this one rendering profile;
other GTK versions have no baselines, so a run linking one fails rather than
silently accepting a different renderer. These scenarios claim the fixed
`fixture_root` `/tmp/strata-e2e-baseline`, because the breadcrumb and context
menu render the full path; runs that claim it take turns.

A capture matches when no more than 0.5% of pixels differ by more than 24 in
any channel, which absorbs the subpixel antialiasing that software rendering
varies between runs. On a mismatch the run's artifacts keep the expected,
actual, and diff images.

Baselines are never accepted automatically. When a change is intended,
regenerate them deliberately:

```bash
STRATA_QA_UPDATE_BASELINES=1 ./scripts/qa.sh columns-view-baseline
```

`scripts/qa.sh` forwards the variable; with it, each `baseline` step writes
its capture as the committed image and passes, and without it a run never
writes one. Name the baseline scenarios you mean to regenerate, then review
the new images and commit them with the change, so the difference is visible
in the pull request.

### Proving a scenario catches a regression

`scripts/qa-mutation.sh` applies one deliberate defect at a time from
`tests/qa/mutations`, rebuilds, and checks that the scenario whose header names
it as its `mutation` fails:

```bash
./scripts/qa-mutation.sh                      # every scenario with a mutation
./scripts/qa-mutation.sh clipboard            # the scenario a patch belongs to
./scripts/qa-mutation.sh copy-leaves-source   # a scenario by its name
```

Each patch breaks a single critical workflow: drop onto a folder, Ctrl+V
paste, arrow-key selection, single-click activation, the appearance menu,
filtered Space preview, filter-row updates, popover outside-wheel, the
long-name editor staying in the viewport, a live click-mode preference, cut
deferring its move to the paste, Return in the location editor, Keep Both
selecting the numbered copy, Enter, click-away and Escape in the inline
editor, a new entry colliding with an existing name, the rename error dialog,
Delete moving to the trash, and Shift+click ranges. Each patch belongs to
exactly one scenario.

The chosen scenarios first run once on the unmodified tree, and every run of a
scenario must pass, or its patch is not tried. Under the patch, the mutation
counts as caught only when at least one run fails a scenario step, no run
could not run or lost its result line, and `qa.sh` exits nonzero; a patch
that no longer builds runs nothing and is reported as missed. Logs land in
`target/qa-mutation/baseline/` and `target/qa-mutation/mutations/<patch>/`,
and `target/qa-mutation/summary.md` has one row per patch. The script restores
only the files the patch touched, refuses a patch whose files have uncommitted
changes, and refuses to run with `STRATA_QA_UPDATE_BASELINES=1`. It exits 0
when every mutation is caught. It shares the build cache with `qa.sh`, so
never run it alongside another QA command. Run it after changing the runner,
and when adding a scenario for a workflow that does not have a mutation yet.

### Skipped scenarios

A scenario whose pass or fail the app leaves to chance, so neither
`known_bug` nor a pass can be relied on, carries a `skip` reason until the app
is fixed; it stays in the suite and runs when named. No scenario is skipped
at present.

### Waits for the app's own timers

QA input arrives faster than a person's, fast enough to land between the app's
own timers. Where that pace exposes a race the app does not yet guard against,
the scenario waits for the state a person's slower pace would reach, and is
listed here:

- `same-folder-copy-numbered-duplicate` (folder): waits for the documents
  column the keyboard cursor mirrors after 75 ms before Ctrl+D.
- `paste-targets-selected-directory` (Columns): waits for the archive column
  the cursor mirrors before Ctrl+V.
- `escape-preserves-name-remaining` (existing-folder) and
  `rename-and-undo-restores` (folder): wait for the folder's column the
  cursor mirrors before F2.

These expose an app race: Ctrl+D, Ctrl+V, or F2 within 75 ms of the arrow key
that moved the cursor onto a folder leaves a stale mirrored column, and a paste
then leaves the pasted file neither selected nor focused.

- `rename-and-undo-restores`: settles the renamed row before Ctrl+Z. The
  directory monitor can list the new name, and the listing select and focus
  it, before the rename's own completion publishes its undo and ends the
  operation; a Ctrl+Z in between is dropped. The app shows nothing a run can
  read to tell that the undo is ready (Undo has no menu item or action
  state), so the wait is the settle. Under the same parallel load, 3 of 60
  runs without it lost the Ctrl+Z and none of 60 with it did (6 of 116 and 0
  of 84 over every probe).
- `new-folder-with-selection-groups` (Columns): waits for the renamed
  `grouped` row to count its two items before clicking it. The row first
  shows the optimistic rename, and a click on it before the listing catches
  up leaves focus in the parent column (16 of 24 runs without the wait, none
  of 36 with it).
- `tenxer-d-without-trash-confirms`: waits for the `no-trash` link to be
  listed before walking the cursor onto it.
- `narrow-window-keeps-last-column`: settles the `folder` column before
  comparing its right edge with the preview's. Once the 900-pixel window first
  fits the column, the column strip scrolls it clear of the preview a moment
  later, and an immediate comparison falls in between (the column ended at
  802 against the preview's 600, then at 600).
- `empty-directory-focus-and-tab-order` (pointer): once the new `empty` row
  is listed after F5, waits up to a second for the listing to replace that
  row once more (it always did within a second over 36 probe runs) before
  clicking it; a click on the row the listing replaces mid-press is
  cancelled, and 4 of 12 runs without the wait lost it.
- `invalid-archive-reports-damage`: waits for the error detail label once the
  progress dialog has turned into `Unable to complete operation`, before
  asserting it; the label can show a moment after the dialog takes its new
  name, and one run in about fifty checked in between.

## Main-thread I/O audit

Strata must not touch the disk on the GTK main thread (see
[`docs/architecture.md`](architecture.md)). Runtime asserts catch calls that go
through Strata's own seams; two opt-in tools catch the rest, including GLib and
GTK internals and closes triggered by drops. Neither is a CI job: run them before
pushing a change that moves work on or off the UI thread.

`scripts/trace-main-io.py` runs a QA selection (every scenario by default)
through `scripts/qa.sh` with `STRATA_QA_MAIN_IO` set, so the runner
(`tests/qa/main_io.rs`) starts each Strata launch under `strace`: the
application, its restarts, the portal backend and the second instances `launch`
steps start. Only the initial thread, which runs GTK's main loop, is traced, so
workers keep their normal timing. Arguments other than the script's own go to
`qa.sh`:

```bash
./scripts/trace-main-io.py                                  # every scenario
./scripts/trace-main-io.py delete-to-trash
./scripts/trace-main-io.py --stacks rename-commits-on-enter/list
./scripts/trace-main-io.py --analyze target/main-io/<run>   # classify again
```

Traces, each launch's log and the report go to
`target/main-io/<run>/<scenario>__<run label>/launch-<n>/`. Each file
call is classified by its path, after resolving descriptors and `*at` folders:

- **user**: the scenario's fixture tree, `HOME`, the Trash, the user's
  `gtksourceview-5` and `gstreamer-1.0` folders, `/tmp`, `/run/media`, `/media`,
  `/mnt` and GVfs mounts. Always a violation.
- **settings**: Strata's own settings, state, caches and runtime files (`strata`
  under the XDG folders, the shared thumbnail cache, GTK bookmarks, portal files,
  Omarchy's theme state and Strata's install folder). A violation unless a
  documented exception below covers it.
- **toolkit**: system paths and the toolkit's own files (fonts, icons, Mesa,
  audio configuration, GTK settings, MIME associations, recently
  used files, the rest of `XDG_RUNTIME_DIR`, the launch's `XDG_DATA_DIRS`, pipes,
  sockets and event descriptors), and the runner's own places that `meta.json`
  lists under `harness` (the shared shader cache, the launch's logs and the
  `PATH` folders). Ignored. QA mode's own disk reads, for `pref` and file
  predicates, run on a helper thread, so they never appear.
- **unclassified**: anything else, reported so new paths get a deliberate
  category.

A metadata call (`stat`, `access`, `readlink`) on a folder above a settings or
toolkit place takes that place's category, because path walks and `mkdir -p`
stat every ancestor; the XDG folders themselves belong to the toolkit. Each call
is a read, a write or, for `inotify_add_watch`, a watch, so no read exception
covers a file monitor. The phase is startup until the first QA reply whose
snapshot shows a pane, once the first window (the chooser for the portal
backend) is up, then running; a second instance's whole trace is startup. The script exits
non-zero when it finds a violation. `tests/qa/main-io-allowlist.toml` holds
only documented exceptions, each with its reason; the script refuses entries for
user paths. The documented exceptions are:

- Strata's reads of its settings and state at startup, which stay synchronous.
- The bundled font, which Strata writes at startup and fontconfig reads whenever
  it lays out text.
- GIO's checks of an application's program while it lists or launches
  applications (`GDesktopAppInfo`), a toolkit exception.

`--stacks` adds the Strata frames of every reported call, named from the binary's
symbol table; it leaves plain `read` and `write` untraced and is slow, so use it
on a selection.

Limits: page faults on mapped files and waits on another thread, such as a
synchronous D-Bus call or a mutex held by a worker, make no traced call. The stall
watchdog covers those. Set `STRATA_UI_STALL_MS` (for example `250`) to start a
thread that posts to the main context every 100 ms; when the main loop has not run
it within the limit, Strata logs the call the UI thread is blocked in, read from
`/proc`:

```text
UI thread stalled 250 ms in openat("/mnt/nas/folder") wchan=…
UI thread ran again after 2400 ms
```

The watchdog is off unless the variable is set. `scripts/qa.sh` forwards it to
every Strata launch, and `--stall-ms` passes it to an audit run, whose report
lists the logged stalls per scenario. Debug builds stall at start-up while the
first window is built, and more so under `strace`.

## In CI

### QA in CI

The **QA scenarios** job in `ci.yml` runs every QA scenario only when CI is
started by hand (**Run workflow** on the CI workflow, or
`gh workflow run ci.yml --ref <branch>`); pull requests and pushes skip it. A
failing scenario fails the run, but no required check needs the job. It waits
for **Quality build and lint**, pulls (or loads) the same pinned build
environment the Rust shards use, restores main's quality Cargo cache, and runs
`./scripts/qa.sh` with `STRATA_TEST_CARGO_CACHE=target/quality-container`.
`--features qa,invariant-checks` equals `--all-features`, so that cache already
holds every dependency built in the same image at the same paths and only
`strata` and the runner compile. The job then checks the `qa` build contains
`usage: strata --qa`, the prefix of `USAGE` in `src/qa.rs` that the release
workflow rejects in release binaries; `scripts/test_qa_mutation.py` requires
both checks' pattern to prefix `USAGE`. A failed run uploads
`target/qa-artifacts` as `qa-artifacts-<attempt>`. CI does not run
`scripts/qa-mutation.sh`.

`scripts/qa.sh` accepts `STRATA_TEST_CARGO_CACHE`, a directory under `target/`
holding `cargo/` and `build/` (default `target/qa-container`); leave it unset
locally. To read a failure, start with the job log (`PASS` and failed lines
name each run), then the artifacts: `qa-artifacts-<attempt>` has the per-run
bundle described in "Replies and failure artifacts".

### Persistent published environments

`Publish the pinned test image` (`test-image.yml`) publishes
`ghcr.io/lgse/strata-test-image` from trusted main when the image inputs, Cargo
manifests, or `.dockerignore` change, or on manual dispatch. The image holds the
pinned GUI/font environment, Rust, and compiled locked Cargo dependencies. A
stub application warms the dependencies; it and its fingerprints are removed,
and no tested application binary is published.

Tags include SHA256 input identifiers, architecture, UID, and GID. Environment
inputs are independent of application source; the build identifier additionally
includes `Cargo.toml`, `Cargo.lock`, and `.dockerignore`, and the publisher uses
those build and BuildKit cache tags to skip or speed up its own rebuilds.
Existing publications are checked before rebuilding. A separate
`base-<environment-inputs>-linux-amd64-1001-1001` tag is the reusable
environment that `scripts/image_base.py` pulls and CI's quality and QA jobs
run. It is published once per environment input set and is not retagged on
ordinary Cargo changes. GHCR storage is independent of Actions cache eviction;
retain tags needed by supported revisions rather than treating them as
disposable per-run artifacts.

`install-packages.sh` configures only the dated HTTPS Arch Linux Archive mirror
and requires Pacman's package-signature policy. It refreshes the signed `core`
and `extra` databases once, in the `package-indexes` stage, then performs a
full snapshot upgrade and requested package installation as one transaction;
the runtime, patched-GTK, and toolchain stages inherit those exact indexes, and
none refreshes from a moving mirror.

Only the main-only publisher has `packages: write` and registry login credentials.
PR CI pulls anonymously and cannot publish shared images. After the first
publication, **make the package public in GHCR package settings** and rerun
the publisher. Its anonymous-access check fails descriptively until it is
readable without credentials. Do not give fork PRs package-write credentials.
A pull request that changes `tests/container/` builds its unpublished
candidate environment in **Quality build and lint** and hands it to the shards
and the QA job as an attempt-scoped artifact; the publisher publishes it once
the change reaches main.

### Maintaining behavioral coverage

Before consolidating tests, identify the retained behavioral owner for every
assertion. Fewer test functions or scenario runs do not establish a runtime
improvement. Preserve functional layout, input-routing, filesystem-safety,
live-preference and lifecycle regressions, even when they share fixtures.
Record task-specific inventories and consolidation decisions in the issue or PR,
not in a committed audit report.
