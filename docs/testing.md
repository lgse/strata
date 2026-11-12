# Testing

The Rust suite covers state transitions, filesystem edge cases, and individual
widgets. The QA scenarios in `tests/qa/scenarios` drive the real GTK
application on a headless X display, send real keyboard and pointer input from
inside the process, and check both what the window shows and what happened on
disk. Both run in one pinned container image.

These suites cover the GTK application and the crates it shares. The Rune-side crates have
their own tests, which run on the host; see [The Rune side](#the-rune-side).

## Running the suites

```bash
./scripts/quality.sh                          # formatting, Clippy, and every Rust test
./scripts/quality.sh test --jobs 4            # every Rust test, on four private displays at once
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
`cargo test --all-targets --all-features` command for the application and the shared
crates (`scripts/lanes.py gtk` names the packages). Use module/name filters:

```bash
./scripts/test-headless.py services::operations::tests
./scripts/test-headless.py basenames_reject_empty_reserved_nested_absolute_and_nul_names
```

These select test execution across targets, not compilation to one target.
Targeted runs can still incur a full test build; Cargo's cache helps subsequent
iterations. There is no automatic changed-code dependency-to-test mapping.

The unit tests need the `test-support` feature, which `--all-features` turns on. It
compiles the test hooks and test-only helpers of the workspace crates under `crates/`,
because a crate's `#[cfg(test)]` code is invisible to another crate's tests. A plain
`cargo test` stops with an error that says so. The QA runner builds without the
feature, so the application the scenarios drive carries no test hooks.

Controller regressions live under `crates/strata-fs/src/app/browser/tests/`, grouped by navigation,
location input, selection, monitor updates, preferences, operations, undo, metadata,
staging, event dispatch, and archive activation. Add cases to the matching owner
rather than to `crates/strata-fs/src/app/browser/tests.rs`, which holds shared source/provider
fixtures. Existing adjacent feature test modules retain their ownership. For
example, `./scripts/test-headless.py app::browser::tests::undo::` selects undo
history/claim regressions; `app::browser::tests::metadata::` selects metadata-fill
and sort-terminal regressions. Keep distinct input routes and lifecycle cases
separate even when they share fixtures.

Local-operation regressions have owners under `crates/strata-fs/src/adapters/local_operations/tests/`
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
`clippy`, or `test`, plus the test phase's `--jobs N` and `--refresh-timings`
(see "Local lanes"), and does **not** forward test filters, so it cannot be used
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

The build makes the recipe's `dependencies` stage, the one the publisher pushes
as the base: the environment, and the Cargo dependencies described under
[The image's dependencies](#the-images-dependencies). It needs the network, for
Arch's archive, GNOME's sources, crates.io and Rune's repository. From nothing
it takes about five minutes on four cores; with the environment's layers
already built, two.

The recipe is `tests/container/Dockerfile`: a digest-pinned Arch Linux base, the
dated 2026-09-11 Arch package snapshot, GTK 4.22.5, GLib 2.88.3, GStreamer
1.28.7, and Rust 1.99.1. Pacman verifies the snapshot's signed packages. The
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
GTK's GStreamer media backend with the base/good/libav plugins, and strace for the
opt-in [main-thread I/O audit](#main-thread-io-audit). It also includes libheif, so
ImageMagick writes real AVIF and HEIC files and glycin's `glycin-heif` loader can
decode them.
A QA `magick` fixture step fails when an `.avif`, `.heic`, or `.heif` output lacks
the matching ISO BMFF major or compatible brand, because ImageMagick silently
writes the input format when an encoder is missing. Rust media
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

#### The image's dependencies

The base holds the third-party crates a run needs, so a first run in a new
checkout fetches nothing. The recipe's last stage loads the workspace's
manifests alone, with an empty file for each target
(`tests/container/stub-workspace.py`), and leaves under
`/opt/strata-dependencies`:

- `cargo/`: a Cargo home with every crate `Cargo.lock` names, fetched, for both
  lanes of [the workspace](workspace.md#two-lanes) and every feature;
- `build/`: a target directory with the third-party crates of the two QA
  builds, compiled, each in a cargo run of its own as the scripts build them:
  `cargo test --features qa,invariant-checks --test qa` for the application,
  and `cargo build -p strata-rune --features qa`;
- `lock.sha256` and `cache-path`: the `Cargo.lock` they were fetched for, and
  the cache they were built for.

Nothing of Strata's own is in it. The stage removes what the stubs built with
`cargo clean -p` for each workspace package, and fails if anything named for
one is left.

`scripts/qa.sh`, `scripts/qa-parity.sh`, `scripts/qa-fuzz.sh`,
`scripts/qa-drive.sh` and `scripts/quality.sh` run
`scripts/dependency_seed.py` in the container before Cargo. It prints one `Dependencies:` line saying what it did:

| The run's cache | What happens |
| --- | --- |
| Cargo has fetched into it before: `cargo/registry` or `cargo/git` exists | Nothing is copied. A warm cache is never reseeded or replaced |
| Empty, and it is `target/qa-container` | The fetched and the compiled crates are copied in, 4 GB. The run then compiles Strata's own crates only |
| Empty, and it is another cache, such as `target/quality-container` | The fetched crates are copied in, 1.2 GB. The cache compiles its own: Cargo's fingerprints hold the path of a crate's sources, so compiled crates are fresh only where they were built |
| Empty, and the base holds no dependencies | Nothing is copied, and Cargo fetches what it needs, as before |

- **`Cargo.lock` ahead of the image.** The base is published once for each set
  of environment inputs, so its crates are those of the `Cargo.lock` of that
  day. When the two differ, the seeding prints `Dependencies: WARNING:
  Cargo.lock has moved past the image.` with both digests, copies what the
  image has, and Cargo fetches and builds the difference from the network.
  The run still works; only the crates that changed are fetched.
- **A copy cut short** is made again: it is written beside the cache and moved
  in, `cargo/registry` last, and that directory is what marks a cache as
  fetched.
- **A seeded Cargo home is not cleaned by Cargo.** It carries a `config.toml`
  with `cache.auto-clean-frequency = "never"`. Cargo otherwise deletes the
  crates a command did not use for a month, and a base outlives that: the first
  run from an older base would delete most of what was copied.
- **`./scripts/quality.sh fmt` and a CI shard copy nothing.** Neither compiles.

To check that a run needs no network, give the scripts an engine that cuts it
off, as the proof in `docs/rune/slice/V9b-test-image-and-exit-check.md` does:
a wrapper that adds `--network none` to `docker run`, named by
`STRATA_CONTAINER_ENGINE`.

### Shared environment for formatting, lint, and Rust tests

`./scripts/quality.sh` runs all three quality phases in the same verified build
base. Use `./scripts/quality.sh fmt`, `clippy`, or `test` for an individual phase.
No separate quality image is necessary: the published environment already has
Rust 1.99.1, rustfmt, Clippy, native development libraries, and Xvfb. Quality now
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
the aggregate gate separately requires all test shards to pass. An empty quality
cache takes the image's fetched crates and compiles them itself: the image's
compiled crates are the QA builds', made for `target/qa-container`, and cold
quality compilation adds test-only and all-feature dependencies.

The Rust suite runs with `--all-targets --all-features --locked`, over the application and
the shared crates, inside private
Xvfb, with `GTK_A11Y=none`, `NO_AT_BRIDGE=1`, and `STRATA_REQUIRE_GTK_TESTS=1`.
Each Xvfb server uses `-noreset` so consecutive test processes do not
race a server reset when the last display client exits. GTK initialization
failures cannot silently skip tests. The container's `/tmp`, which holds `HOME` and
the tests' temporary folders, is an executable tmpfs: the file-operation tests
need `O_TMPFILE` and file handles there, which an overlay `/tmp` refuses
(overlayfs has `O_TMPFILE` only since Linux 6.10 and gives file handles only
when exported over NFS). Outside CI those tests skip on such a base;
`STRATA_REQUIRE_DEVICE_TESTS=1`, which CI and `scripts/test-headless.py` set,
fails them instead. Formatting, compiler,
lint, and test failures remain blocking. Lightweight policy/helper jobs retain
their existing runners rather than downloading a large GUI image unnecessarily.

### Rust quality shards

CI's **Quality build and lint** job runs formatting and Clippy, then compiles
`cargo test --locked --all-targets --all-features --no-run` once, for the application and
the shared crates. It exports only
test executables and a plan, not Cargo caches. `scripts/quality_ci.py` collects
each libtest inventory, including the explicitly ignored tests, and assigns every
entry to one of four shards. Tests are balanced longest-first across the shards
using the weights in `scripts/quality-durations.json` (see "Balancing weights");
unknown tests receive a one-second weight and always participate. The weights are
not an allowlist.
`ui::search::tests::deferred_scroll_restoration_yields_to_updates_wheel_scrollbar_and_query_reset`
runs in its own libtest process before the other tests assigned to its lane,
not on a dedicated runner. Each process must pass its complete selection before
an aggregate shard receipt is written. Validation rejects a missing, duplicated,
or ignored isolated test; renaming or removing it requires updating `ISOLATED_TEST`.
Sharding does not shorten an individual test.

Each **Rust tests shard N** verifies the checkout revision, application/Rust-test source
fingerprint, image inputs, executable checksums, and the entire libtest inventory
before selecting exact names. GTK tests run one at a time per display, since every
GTK child of a test process waits on one lock: children on one display share its
clipboard and pointer grabs. A display therefore keeps about one core busy, so
each shard splits its tests across three **lanes** (`--jobs 3` in the workflow),
balanced by the same weights. Each lane runs its own libtest processes under its
own Xvfb server and private D-Bus session, and lanes share neither with each other
nor with another shard. On four CPUs, one shard took about 240 seconds on one
display and about 90 on three; at one lane per CPU or more, load-sensitive tests
start to fail. The shard's
receipt still covers all of its tests, and any failed lane fails the shard after
the others finish. Lane logs are written while tests run and uploaded even when a
shard times out.
The runner checks libtest's actual selection and final passed/ignored counts
before writing a success receipt; nonzero exits, empty runnable shards, or changed
inventories fail. Existing `#[ignore]` entries stay explicitly accounted for;
sharding neither enables them nor silently ignores additional tests. Unsupported
non-libtest harness inventories fail closed rather than disappearing from coverage.
The QA runner, `tests/qa.rs`, has its own harness (`harness = false`, see
"Scheduling and run costs") that keeps libtest's `--list`, `--ignored`,
`--exact`, and `test result:` formats, so its scenario runs are inventoried as
explicitly ignored tests and its unit tests run in the shards like any other.
A test that runs in a shard therefore cannot rely on `CARGO_BIN_EXE_*`, or on
anything else under the build directory, existing at run time: the bundle carries
test binaries only, so the application binary is not on a shard's runner.

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
not test filters. Rebuild the bundle after changing source or checkout revision.
The four-shard matrix and `SHARDS` constant must be updated together if tuning
fan-out. Each shard has a ten-minute hang bound; timing is otherwise informational.

### Local lanes

Ordinary `./scripts/quality.sh test` runs `scripts/quality_ci.py local`. It builds
the bundle once into `target/quality-local` as a single shard and runs it in lanes
on this machine, with the same plan validation, isolated-test handling, receipts,
and exactly-once verification as CI. The default is one lane per two available
CPUs. Pass `--jobs N` to choose another count, for example `--jobs 1` for a single
display. With one lane per CPU, some load-sensitive tests can fail. A lane's full
output is in `target/quality-local/reports/shard-0-lane-N.log`; a failing run
prints the failed tests' panics and the end of the lane's log.

### Balancing weights

`scripts/quality-durations.json` maps each test to the seconds it occupies a lane.
Refresh it after adding or substantially changing slow tests:

```bash
./scripts/quality.sh test --refresh-timings
```

This runs the local lanes with libtest's per-test times, which stable Rust reports
only with `RUSTC_BOOTSTRAP=1 -Z unstable-options --report-time`, so it is opt-in
and never part of ordinary or CI runs. Each libtest process uses one thread so a
GTK test's time excludes waiting for the display. A test that runs in an isolated
GTK child keeps its full time; any other test shares libtest's threads, four on
CI runners, so it is weighted at a quarter of its time, with a floor of 0.01
seconds. The run fails unless it measured exactly the runnable tests, and then
rewrites the file. Review the diff like any generated file; the weights balance
lanes and shards but never select tests.

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

### Private main contexts in Rust tests

A Rust test that drives async code on its own GLib main context creates it with
`test_support::PrivateContext`, never `glib::MainContext::new()` or
`glib::MainContext::with_flags()` (Clippy rejects both).
`<glib::MainContext as Default>::default()` also calls `new()`, but Clippy
cannot catch it by path, so don't use it for a test's context either. A GIO call
started on the context keeps it alive from its `GTask`, so a GIO pool thread can
drop the last reference after the test has finished. Finalizing the context
there would drop any local task or callback still attached to it on the wrong
thread and abort the whole test process. `PrivateContext` destroys every source
left on the context when it is dropped, on the thread that created them,
repeating until no source is left. A source attached after that, such as the
completion of a GIO call still in flight, is leaked on purpose: it keeps its
task and the context alive, so nothing on the context is ever finalized on
another thread. Tests that drive the process-wide default context instead hold
`test_support::ASYNC_MAIN_CONTEXT_DEFAULT`.

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
`scripts/qa-drive.sh`, `scripts/qa-mutation.sh`, `scripts/qa-parity.sh` and
`scripts/qa-fuzz.sh` (through `scripts/lib/qa-container.sh`). Because the cache is shared, run them one after
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
scenarios"), `cost` orders the runs (see "Scheduling and run costs"),
`quality` rates the scenario as the `qa-quality` skill defines, and `parity`
and `targets` label a scenario and its Rune twin (see "Twins"). Steps run
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

The Rune target has no display to read a keymap from. After the switch above,
the runner sends it the manifest's layout as a `keymap` request of its own, and
the application reads key names by their US positions under that layout itself
("The Rune target and the parity ledger" below).

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
A run on the Rune target writes the same bundle under
`target/qa-artifacts-rune/` instead.

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
`TERMINATE_GRACE`), merges the given
preferences into its saved `settings.toml` (each key replaced whole, tables
included), and launches it again on the same fixture and HOME. A value
`settings.toml` cannot hold is refused before the application quits, and a
saved file that no longer parses is left unchanged and relaunched. Every exchange is appended to
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
Delete moving to the trash, focus returning to a Columns filter result after a
dismissed delete confirmation, and Shift+click ranges. Each patch belongs to
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

### The Rune target and the parity ledger

The runner drives two binaries with the same scenario files, fixtures and
runner steps. `STRATA_QA_TARGET` picks one:

| Target | Binary | Input and state |
| --- | --- | --- |
| `gtk`, the default | `strata --qa`, the root package's own | XTEST on the run's Xvfb; the widget tree |
| `rune` | `strata-rune --qa`, built with its `qa` feature and named by `STRATA_QA_RUNE_BINARY` | Rune's input layer on an offscreen host that needs no display; Rune's accessibility tree, with its roles mapped to the AT-SPI names scenarios use |

A `rune` launch fails with the reason when `STRATA_QA_RUNE_BINARY` is unset or
names no file. The runner still starts Xvfb for a Rune run, and the binary
ignores it, so that run's `screenshot.png` shows an empty screen. A step or
predicate the Rune backend does not answer replies "… is not supported on this
target". `docs/rune/slice/V7-rune-qa-backend.md` lists what it answers.

It reads a text field, for the `field` target and predicate, from Rune's
accessibility tree: the name, the text, and the caret and selection counted in
characters as on GTK. It reads one while the caret is on the field's first
line, which is all of a single-line field; on a later line it finds no field.
Under a layout with dead keys it composes them as a Rune window does, through
xkbcommon's Compose table: the dead key reaches the application as nothing,
and the key that completes it as the composed character. A key that breaks a
sequence off is not swallowed as GTK's input method swallows it, so Escape
with a dead key pending is an Escape press there
(`docs/rune/slice/V9-rename-parity.md`).

`./scripts/qa-parity.sh` sets both variables. It builds both binaries in the
pinned image and runs the runs of a selection on each target. A selection is a
scenario folder, a work package, a gate of the slice, or `all`:

```bash
./scripts/qa-parity.sh selection          # the required and parallel runs of tests/qa/scenarios/selection, on GTK and then on Rune
./scripts/qa-parity.sh A7                 # the required and parallel runs package A7 owns
./scripts/qa-parity.sh all                # every required and parallel run of the ledger
./scripts/qa-parity.sh g1                 # gate G1's required and parallel runs
./scripts/qa-parity.sh slice              # the slice's exit check: every gate's, then the rest, below
./scripts/qa-parity.sh A7 --counts        # the selection's runs by status and reason; runs nothing
./scripts/qa-parity.sh A7 --twins         # the selection's twins, each with what a review audits; runs nothing
./scripts/qa-parity.sh g2 --target rune   # one target only
./scripts/qa-parity.sh A7 --pending       # a survey: the selection's pending runs, on Rune
./scripts/qa-parity.sh A7 --parallel      # a survey: the selection's parallel runs from the shared file, on Rune
./scripts/qa-parity.sh g2 --scenario shift-click-range   # only that scenario's runs of the selection
./scripts/qa-parity.sh mutations          # the Rune target's mutation patches
./scripts/qa-parity.sh gtk-mutations      # the GTK application's, less the ledger's stale ones
```

A `required` run is run from its one file on both targets. A `parallel` run is
at parity by another route and is run with them: its scenario on GTK, and its
twin, `<scenario>-rune/<run>`, on Rune (see "Twins"). `--scenario` takes either
name of a pair, and each selects the pair.

It exits 0 only when every one of those runs passes on every target it ran. A
survey runs the shared files on Rune alone, lists the runs that pass and fails
only when nothing ran: `--pending` finds a pending run that is ready to be
`required` (`PASSES ON RUNE, STILL PENDING: …`), and `--parallel` a parallel
run whose exception has lapsed (`PASSES ON RUNE FROM THE SHARED FILE, STILL
PARALLEL: …`). Artifacts are kept for passing runs too
(`STRATA_QA_KEEP_ARTIFACTS=0` keeps a failed run's only). With `--target` or
`--scenario`, `slice` runs its required and parallel runs and nothing after
them; a `--scenario` that names no run of the selection is an error, and so is
a selection the ledger does not have. A folder or a package is every view's
runs; a gate is Columns runs only.

`--counts` reads the ledger and starts no container. It prints the selection's
runs by status, the parallel ones by exception and the pending ones by reason,
then by host and by view, its twins by exception, and its runs by package,
which is what a checkpoint report quotes. `twins` counts files: a twin that
holds two runs is two parallel runs and one twin. A parallel run that names no
host is counted on `offscreen`, where its twin runs. It counts the whole
selection, so it takes no `--target`, `--pending`, `--parallel` or
`--scenario`, and neither does `--twins`, which names each twin ("Twins",
below):

```text
== selection: 25 run(s) in 1 folder(s) ==
required      5
parallel      1
  timing        1
pending      19
  scope        19
excluded      0
host      offscreen 5, session 0
view      columns 11, icons 7, list 7
twins     timing 1, a11y 0, rune 0, oracle 0, toolkit 0, deviation 0
package    runs  required  parallel   pending  excluded
A1            7         0         0         7         0
…
```

`slice` alone is the exit check of `docs/rune/slice/README.md`. It has four
parts that all run whatever the earlier ones did:

1. every gate's required runs, on GTK and then on Rune;
2. the runner's own tests and the whole GTK suite, as `./scripts/qa.sh` runs
   them;
3. the Rune mutation patches, as `mutations` runs them;
4. the GTK mutation patches, as `gtk-mutations` runs them.

It ends by listing each allowance it used or refused, or saying it used none,
and then `== slice exits <status> ==`. A whole run takes about 14 minutes with
a warm cache, 11 of them the GTK mutation patches.

As a gate before a merge, `slice` covers the slice's gated runs on both
targets, both sets of mutation patches, and the whole GTK suite with its two
allowances. It runs no required run outside the gates. None exists yet; once
an area package has made one, a change to the runner, the protocol, a shared
crate or `src/` passes `slice` and `./scripts/qa-parity.sh all`, which runs
every required run of the ledger on both targets.

`mutations` proves the Rune target's runs can fail, as `scripts/qa-mutation.sh`
does for the GTK application. Each patch under `tests/qa/parity/mutations/`
breaks one behaviour of `crates/strata-app` and is named for the scenario that
must catch it, `<scenario>.patch`. For each, the scenario's runs on the Rune
target must pass unpatched, and with the patch applied at least one must fail a
step and none fail to run. Those runs are the scenario's required runs; a
twin's are the runs that stand in for its original's parallel ones, and its
patch is `<scenario>-rune.patch`. The patch is applied to the checkout and
reverted, so its files must have no uncommitted changes; logs and
`summary.md` land in `target/qa-parity-mutation/`. A patch whose scenario has
no run of its own on the Rune target is refused: one with no required run, or
one whose every run is parallel, whose patch is its twin's. The scenario may be
in any folder. Add a patch when an area gains its first required runs and with
every twin, and check it when its scenario or `strata-app`'s adapter changes.

#### The ledger

The runs come from the parity ledger, the directory `tests/qa/parity`. It has
one file for each folder under `tests/qa/scenarios`, `<folder>.toml`, which
lists every run of that folder once, in every view and in the runner's order:
760 runs in 58 files at `94ece01`, and `./scripts/qa-parity.sh all --counts`
has today's figure. One file for each folder keeps branches that work on
different folders out of each other's way. `slice.toml`, beside them, lists no
runs: it holds the slice's allowances, below.

```toml
[[run]]
scenario = "shift-click-range"
run = "columns"
package = "V8"
gate = "G2"
status = "required"
host = "offscreen"

[[run]]
scenario = "shift-click-range"
run = "icons"
package = "A1"
status = "pending"
reason = "scope"
blocker = "A1"
```

| Field | On | Holds |
| --- | --- | --- |
| `scenario`, `run` | Every entry | The run, as the runner names it: `<scenario>/<run>` |
| `package` | Every entry | The work package that owns the run: an id of the parity plan (`A7`, `H4`), or the slice's package that took the run to `required` (`V7` for gate G1, `V8` for G2, `V9` for G3). "Who owns a run", below, says which |
| `gate` | The Columns runs of the slice's seven folders, and no other | `G1`, `G2` or `G3`: what `slice`, `g1`, `g2` and `g3` select (`docs/rune/slice/README.md`) |
| `status` | Every entry | `required`, `parallel`, `pending` or `excluded` |
| `host` | A `required` entry; a `parallel` or `pending` one may name it | `offscreen`, the Rune host every required run uses today, or `session`, a real window in a headless compositor. On a `parallel` entry it is the host the twin runs on, `offscreen` when it names none. On a `pending` entry it says which host the run will need, and nothing asks for it |
| `exception`, `twin`, `ref` | A `parallel` entry | The exception the run stands under, the twin's run that stands in for it on Rune (`<scenario>-rune/<run>`), and the exception's record. "Twins", below |
| `reason`, `blocker` | A `pending` entry | Why the run does not pass on Rune yet, and what someone can act on |
| `ruling` | An `excluded` entry | The owner ruling that left the run out |

Identity comes first and what changes comes last, so that two branches that
edit neighbouring entries merge without a conflict.

- `status = "required"`: passes on both targets, and `qa-parity.sh` runs it. A
  run moves here only once it has passed on both, three times running, in the
  pinned image. `qa-parity.sh` refuses a selection with a required run on the
  `session` host until it can start that host.
- `status = "parallel"`: the shared file cannot pass on Rune under a named
  exception, and a twin beside it does. It carries the `exception`, the
  `twin` and the `ref`, and `qa-parity.sh` runs it with the required runs:
  the scenario on GTK and the twin on Rune. "Twins", below, has the rules.
- `status = "pending"`: does not pass on Rune yet, with a `reason` and a
  `blocker`. The reasons are `harness` (a step or predicate the Rune backend
  lacks), `component` (a component lacks or differs in the behaviour), `rune`
  (Rune itself, with the entry of `docs/rune/upstream/rune/`), `platform` (it
  needs a real window, compositor, clipboard or drag, or something else the
  test environment lacks), `toolkit` (it checks something GTK-specific),
  `deviation` (the port differs on purpose, by an owner ruling) and `scope`
  (it needs a part of the interface no package has ported yet). A `scope` run
  names the package that will port it as its blocker; the runs the slice left
  pending keep the blocker it recorded.
- `status = "excluded"`: left out of the migration by an owner ruling, which
  `ruling` cites. It runs on GTK only. The nine `visual_baselines` runs are
  the only ones, and a unit test holds the list to them.

#### Who owns a run

Adding, renaming, moving or removing a run means editing the ledger in the
same change, and a merge from `main` that brings such a change does it in the
merge. A new run goes into its folder's file at its place in the runner's
order: scenario files by name, and within a file the header's variants in
order, each in its modes. It is listed `pending`, reason `scope`, with its
package as the blocker. Its package is the first of these that fits:

1. **10xer mode.** The scenario's name or the run's variant id says `tenxer`:
   A11, in whatever folder and view.
2. **Visual baselines.** A run of `visual_baselines` is `excluded` under E1,
   with the ruling the folder's other entries cite.
3. **The slice's seven folders** (`keyboard_navigation`, `selection`,
   `escape_selection`, `background_selection`, `hidden_files`,
   `startup_arguments`, `inline_renaming`). An Icons run is A1's and a List
   run A2's. A Columns run takes the folder's `gate` (`G3` in
   `inline_renaming`, `G2` in the others) and the package that owns the first
   part of the interface it needs that Rune does not mount yet: the package of
   that part's folder in the table below. One that needs no such part is E1's,
   which closes the ledger.
4. **Preview files.** In `quick_preview`, a scenario about a PDF, a model or a
   route, and the PDF variant of any other scenario, is A16's. The rest of the
   folder is A15's.
5. **Settings.** In A6's folders, a run that opens the Settings window in its
   first steps and works on one of its pages is A18's. One that opens Settings
   as one overlay among several, or checks that it stays closed, is A6's.
6. **The folder**, by the tables of the parity plan
   (`docs/rune/parity/README.md`, "Work packages"):

| Package | Folders |
| --- | --- |
| A1 | `browser_workers`, `thumbnail_worker_settings` |
| A3 | `sidebar_pinning`, `sidebar_reordering`, `letter_bookmarks`, `recent` |
| A4 | `tabs` |
| A5 | `locations` |
| A6 | `dialogs_and_menus`, `keymap`, `accessibility` |
| A7 | `entry_management`, `rename_visibility` |
| A8 | `view_switching`, `text_size`, `column_background`, `created_entry_columns`, `monitor_selection` |
| A9 | `click_modes`, `pointer_intent`, `marquee_scrolling`, `popover_scrolling` |
| A10 | `search_filter_sort`, `filter_results` |
| A12 | `clipboard`, `copy_conflicts` |
| A13 | `archive_activation`, `archive_conflicts`, `archive_errors`, `archive_preview`, `archive_reveal`, `file_providers` |
| A14 | `open_with`, `terminal`, `custom_actions` |
| A15 | `preview_session`, `quick_preview` |
| A16 | `preview_media_layout`, `preview_video` |
| A17 | `properties_media`, `properties_size` |
| A18 | `preferences` |
| A19 | `drag_and_drop`, `drag_animation`, `content_drops`, `cross_volume_drop`, `sidebar_file_drops` |
| A20 | `chooser_selection`, `chooser_image_conversion` |
| A21 | `file_manager_interface` |

A new folder takes the package of the part of the interface it tests. Rules 1
and 3 to 5 are how the first ledger read the plan where it splits a folder,
and E1 for a Columns run with nothing to wait for is a default; the owner has
yet to confirm them (lgse/strata#2122).

Only a run's package edits its entry: it flips the status, and rewrites the
reason and the blocker as it learns them. `V7`, `V8` and `V9` own the runs the
slice took to `required` and no other, so such an entry cannot be demoted or
handed on without failing the check.

#### Twins

A run that cannot pass on Rune from the file both targets share is not waved
through. A **twin** covers it: a second scenario that reaches the same result
on Rune by other steps. The pair is labelled, counted and checked. This is the
one place where the two targets run different files, so the rules are narrow,
and a check holds every one of them that a check can hold.

The parity plan (`docs/rune/parity/README.md`, "Conventions") proposed the
header keys and the ledger fields and left them to be settled. They are settled
as it proposed them: `parity` with `twin`, `exception`, `ref` and `differs`,
`targets`, and the entry's `exception`, `twin` and `ref`. What this section
adds is where a twin's run is listed, what `targets` does to the runner's
counts, the form of a reference, an optional `host` on a `parallel` entry, and
the checks.

**A twin is the fourth resort of five.** A difference between the targets is a
Rune-side bug until shown otherwise. The plan's order of resort ("When a run
differs") is worked from the top, and the first step that fits is taken:

1. Fix the port: the adapter in `crates/strata-app`, or the component.
2. Absorb the difference in the Rune QA backend. Key names, role maps, settle
   rules and field offsets belong there, never in a scenario.
3. Reword the scenario on `main`, with a reason of its own, so that one file
   passes on both targets. It reaches `strata-next` by merge.
4. Write a labelled twin, under one of the exceptions below, with a reference.
5. Leave the run `pending`, with a reason and a blocker someone can act on.

The pull request that adds a twin says why steps 1 to 3 do not fit. When the
difference is a bug in Strata itself, file the `oracle` issue and go straight
to step 4. Parity findings are now recorded in `docs/rune/parity/findings.md`;
what a twin cites for one with no issue is an open question in
`docs/rune/parity/owner.md`.

**The two files.** The twin is a new file beside the original,
`tests/qa/scenarios/<folder>/<scenario>-rune.jsonl`. A name that ends in
`-rune` is a twin's and no other scenario's. Both headers carry a `parity`
label:

```json
{"scenario":"shift-click-range","modes":["columns","icons","list"],"parity":{"twin":"shift-click-range-rune","exception":"timing","ref":"lgse/strata#2200"}}
```

```json
{"scenario":"shift-click-range-rune","modes":["columns"],"targets":["rune"],"parity":{"twin":"shift-click-range","exception":"timing","ref":"lgse/strata#2200","differs":"line 3 waits for the selection, where the original waits out the highlight"}}
```

| Key | On | Holds |
| --- | --- | --- |
| `parity.twin` | Both headers | The other file of the pair: on the original its twin, and on the twin its original |
| `parity.exception` | Both headers | The exception's code, the same in both |
| `parity.ref` | Both headers | The exception's record, the same in both: an issue, `lgse/strata#<number>`, or for a `rune` twin the packet entry it expires with, `docs/rune/upstream/rune/<number>-<name>.md` |
| `parity.differs` | The twin's | One line: which steps differ from the original's, and why |
| `targets` | The twin's | `["rune"]` |

- **The label is the one edit made to a GTK scenario for parity's sake**, by
  the owner's ruling of 2026-11-11. The original's steps stay as they are, and
  it takes no `targets`: it goes on running on both targets in the runs that
  do not differ.
- **The run is the unit.** A twin holds only the modes or variants that
  differ, and they are modes and variants of its original. The other runs of
  the original stay `required` or `pending` from the shared file.
- **A twin starts from what its original starts from.** For each run it
  holds, the header's `tree`, `images`, `fixture_root`, `setup`, `teardown`,
  `prefs`, `layout`, `portal`, `env`, `session_bus` and `input_method`, with
  the variant's over them, are its original's. `gtk_settings` is the one key
  of that kind left out: the Rune target reads no GTK settings, so a twin has
  nothing to keep there. A variant's `vars` are read into the steps before
  the steps are compared.
- **A twin's steps differ, and keep the end state.** In each run it holds, a
  twin has at least one step that is not its original's: if nothing differs,
  the shared file passes on Rune and the run is `required`. A step's
  `message` and `timeout_ms` are no part of what it does or asserts, so a step
  that differs only there is the original's. And a twin keeps every `fs` step
  of its original's run and every `wait` or `expect` that reads a `pref`,
  unchanged and in the original's order: the files on disk and what
  `settings.toml` holds are among what the plan says must match with no
  exception. The `fs` steps that act are kept with the ones that check,
  because a change to the files mid-run is part of the fixture.
- **A twin is a full scenario**: messages on its waits and a proof that it
  can fail, which is `tests/qa/parity/mutations/<scenario>-rune.patch`. It
  names no `mutation`, which is a GTK patch, and carries no `known_bug`: a
  twin passes on Rune, and a run whose twin cannot is `pending`. By
  convention it has a `cost` and a `quality` as well; no check asks for
  either, of a twin or of any scenario. `scripts/qa_cost.py` times the GTK
  suite and leaves a twin's `cost` alone; give it its original's for the runs
  it holds.
- **`-rune` is reserved** across the corpus, for scenarios that arrive from
  `main` too: one named so that is no twin is refused, and is renamed on
  `main`.
- **A merge from `main` can conflict in a labelled header.** The header is
  one line, and `main` edits it too (a cost refresh, a quality note). Take
  `main`'s line and add the `parity` label back; nothing else of the line is
  `strata-next`'s.
- **Twins outlive GTK.** When GTK is retired they lose the suffix and become
  the corpus (package E4).

**`targets`.** It lists the targets a scenario has runs on, `gtk` and `rune`;
a header without it runs on both. A scenario whose `targets` leave out the
target the runner drives is no run there. It is not planned, so it is not
passed, failed, ignored or filtered out, `--list` does not have it, and the
suite's `test result:` line counts what it counted before the file was
written. The runner says which it left out, in one line:

```text
NOT ON gtk: 1 scenario(s) list other targets and are no runs here: shift-click-range-rune
```

So a twin moves no count of `./scripts/qa.sh` or of the CI shards' inventory.
Naming one there fails the selection before anything runs, whatever else the
selection names, with `error: <scenario>-rune is no run on gtk: …` and the
command that does run it: `./scripts/qa-parity.sh <selection> --target rune
--scenario <scenario>-rune`. Only a twin carries `targets`, and only
`["rune"]`: a scenario that left a target out without a pair would be a run
nobody accounts for, and the ledger's statuses are how a run is left out.

**The ledger.** The original's run is `parallel`:

```toml
[[run]]
scenario = "shift-click-range"
run = "columns"
package = "A9"
gate = "G2"
status = "parallel"
exception = "timing"
twin = "shift-click-range-rune/columns"
ref = "lgse/strata#2200"
```

A twin's own run has no entry. It is listed once, as the `twin` of the run it
stands in for, and a twin's runs are exactly its original's parallel runs. The
ledger therefore goes on listing the runs of the corpus both targets share:
its total does not move when a twin is written, a run moves from `pending` to
`parallel`, and `--counts` reports the twins beside the runs, by exception. A
`parallel` run clears the bar a `required` one clears, by its other route: it
moves there only once the original has passed on GTK and the twin on Rune,
three times running, in the pinned image, and `qa-parity.sh` runs both from
then on. An entry may name the `host` its twin runs on; `qa-parity.sh` refuses
one on the `session` host as it refuses a required one.

A run that is `pending` becomes `parallel` by the approval below. A run that
is `required` today becomes `parallel` only with the owner's agreement,
whatever the exception's code: that is the direction that loses parity, and no
check tells it from the other. (A default of the session that orchestrated
wave 0, for the owner to confirm: lgse/strata#2123.)

A twin's scenario may carry a `skip`, or stand in for a scenario that does.
`qa-parity.sh` does not set `STRATA_QA_NAMED`, so a skipped scenario is not
started by it: such a pair fails the selection on the target that skipped it,
as a `required` run of a skipped scenario does (lgse/strata#2134). A twin
makes that neither better nor worse.

**The exceptions.** Six codes of the plan's table may carry a twin. `platform`
never does: the same file runs on the session host instead, so its run is
`pending` or `required` with `host = "session"`. `scope`, `harness` and
`component` are pending reasons, not exceptions, and must reach zero.

| Code | A twin is allowed when | Its `ref` | A check holds | The reviewer holds |
| --- | --- | --- | --- | --- |
| `timing` | The scenario asserts a duration, a mid-animation state, or a GTK timer race from the "Waits for the app's own timers" list | An issue | The form of the reference | That waiting on observable state, or the backend's settle, could not absorb it, and that the issue says so |
| `a11y` | The backend could present the target only by guessing: GTK exposes a tree shape, a role or an element that Rune's tree lacks | An issue | The form of the reference | That the issue names the missing node, and that the backend's role map could not absorb it |
| `rune` | Rune lacks the capability at the pinned revision, and a workaround reaches the same result by another route | The packet entry | That the entry is a file of `docs/rune/upstream/rune/` | That the entry's notes name the workaround, and that the twin and the workaround go when the pin moves past the fix: "Keeping the exceptions few", below |
| `oracle` | Strata itself has the bug, the GTK run shows it or its scenario encodes it, and Rune behaves correctly. The twin asserts the correct behaviour | An issue | The form of the reference | That the issue has the `oracle` label and says why GTK's behaviour is the bug. If `main` later fixes it, the twin is deleted |
| `toolkit` | A GTK-only check is the point of the scenario: a pixel predicate, `gtk_settings`, a GDK detail, a coordinate tied to GTK's row heights and fonts | An issue | The form of the reference | That targeting by name instead of offset could not absorb it. Visual baselines are not twinned: they are `excluded` |
| `deviation` | The port behaves differently on purpose, and the twin asserts the new behaviour | The issue that records the owner's ruling | The form of the reference | That the ruling is there, and is the owner's: the reviewer opens the issue and reads it |

**Who approves a twin**, as the owner ruled on 2026-11-11. The reviewer of the
pull request approves `timing`, `a11y` and `toolkit` twins, on agreeing that
steps 1 to 3 do not fit, and `oracle` twins whose bug is clear-cut. A
`deviation` twin needs the owner, and so does an `oracle` twin whose bug is
arguable, which is a deviation. A `rune` twin follows its packet entry.

No check reads a `deviation` twin's issue, and it is the one code the reviewer
may not approve. Until the owner says where rulings are recorded, the reviewer
opens the issue and confirms that the ruling is there before the twin merges.
(A default of the session that orchestrated wave 0, for the owner to confirm:
lgse/strata#2123.)

**What no check holds** is left to that review:

- that the checks a twin keeps or adds still assert the original's intent. A
  check holds the `fs` steps and the `pref` checks; a twin may still drop
  another wait of its original's, and `--twins` gives the number;
- that `differs` is true, and that only the steps it names differ;
- that the exception fits the difference, and that steps 1 to 3 were tried;
- that the twin's patch breaks the behaviour its differing steps check, and
  not its first wait. That the patch exists is checked, and that the twin
  catches it is run by `./scripts/qa-parity.sh mutations` and by `slice`;
- that an issue exists, is open and has its label, and that the pair passed
  three times running before the entry was flipped.

**Keeping the exceptions few.** An exception is a counted, referenced record,
and two commands keep the count honest:

```bash
./scripts/qa-parity.sh all --twins       # every twin; no container
./scripts/qa-parity.sh all --parallel    # every parallel run from the shared file, on Rune
```

```text
== selection: 1 twin(s) for 1 parallel run(s) ==
shift-click-range-rune  timing  lgse/strata#2200  A9
  stands in for  shift-click-range/columns
  differs        line 3 waits for the selection, where the original waits out the highlight
  steps          2 the original's, 1 its own, 1 of the original's dropped
```

`--twins` is what a review of a twin and a checkpoint report read: the
exception, the reference, the package, the runs the twin stands in for, its
`differs`, and how its steps stand to its original's over those runs. A twin
that drops most of its original's steps, or adds none, shows there.

`--parallel` finds an exception that has lapsed. It runs the shared file of
each parallel run on Rune, as `--pending` does for pending runs, and prints
`PASSES ON RUNE FROM THE SHARED FILE, STILL PARALLEL: <run>` for each that
passes. Such a run needs no twin any more: the port or the backend absorbed
the difference, or Rune gained the capability. Its twin, its patch and its
label are removed, and its entry goes to `required`. Run it for a package's
own twins before the package closes, at every Rune pin bump, where `--twins`
names the `rune` twins and their packet entries, and at each checkpoint.

#### What checks the ledger

Two readers check it, by the same rules. The runner's unit tests fail on a
ledger with a fault, and `./scripts/qa-parity.sh` refuses to select anything
from one: it exits 2 and names the file and the run. Both refuse:

- a folder with no file, and a file with no folder;
- a run missing from its folder's file, listed twice, no run of that folder,
  or standing before a run the runner plans earlier;
- an unknown field or value, and an entry with no package;
- a package the plan does not have, and `V7`, `V8` or `V9` on any entry but a
  `required` one of its own gate;
- a gate that is not the run's;
- a `required` entry without a `host`, a `pending` one without a `reason` and
  a `blocker`, an `excluded` one without a `ruling`, a `parallel` one without
  an `exception`, a `twin` and a `ref`, and the fields of another status;
- on a `parallel` entry, a code that carries no twin, a `twin` that is not
  `<scenario>-rune/<run>`, and a reference of the wrong form;
- an entry for a twin's own run.

Both then read every scenario file. A header with a key the runner does not
have, or a value of another shape, is one refusal, and so is a step of a
labelled scenario that is no JSON object; nothing else is checked until it is
mended. Then both check every `parity` label and every pair, and say what to
do about each fault. A header alone is refused for:

- a `-rune` name without a label, and `targets` on a scenario that is no twin;
- a twin that names another scenario than its original, and an original that
  names another twin; a twin of a twin;
- a code that carries no twin, and a reference of the wrong form;
- a twin without a one-line `differs`, and an original with one;
- a twin whose `targets` are not `["rune"]`, so that it would also run on GTK;
- a twin with a `mutation` or a `known_bug`.

A pair is refused for:

- a `parallel` run whose twin's file does not exist, and a label that names a
  twin nobody wrote;
- a twin whose original does not exist, is in another folder, or has no label;
- two headers that differ in the code or the reference, and an entry that
  differs from them in either;
- a twin with no run, with a run its original lacks, or with a run that
  starts from another fixture, preferences, layout, environment or bus than
  its original's;
- a twin's run with no step, with its original's steps and no other, or
  without an `fs` step or a `pref` check of its original's, in its order;
- a run a twin stands in for that the ledger does not call `parallel`, and a
  `parallel` run its twin does not hold;
- a `rune` twin whose packet entry does not exist;
- a twin without its mutation patch.

Two tables hold what both accept and refuse: `tests/qa/parity_cases.toml` for
the entries' own rules, and `tests/qa/parity_pair_cases.toml` for a labelled
pair with its two files. `tests/qa/parity_tests.rs` and
`tests/qa/twin_tests.rs` run them through the runner's readers
(`tests/qa/parity.rs`, `tests/qa/twins.rs`), and `scripts/test_qa_parity.py`
and `scripts/test_qa_parity_twins.py` through the scripts'
(`scripts/qa_parity_check.py`, `scripts/qa_parity_twins.py`), and a problem is
the same line from both. A
change to the schema adds its rows to a table first, so that neither reader
can learn a field or a status the other does not know. The scripts' tests also
hold their list of header keys to the runner's `Header` and `Variant`.

Some rules are conventions that nothing checks: that only the owner edits an
entry, that a run marked `required` or `parallel` did pass three times, what a
blocker says, and what "Twins" leaves to the reviewer. A check of the ledger's
diff against the merge base, by the package id in the branch name, could hold
the first; it is not built.

#### What `slice` lets stand

The ledger's `slice.toml` names what the GTK suite already did wrong before
the slice, so that `slice` fails for parity's reasons only. These allowances
are `slice`'s alone: `./scripts/qa.sh` and `./scripts/qa-mutation.sh` do not
read them and fail as before.

- `[[flaky]]` names one run, `new-folder-with-selection-groups/columns`, by an
  owner ruling of 2026-11-11. When it fails in the whole-suite part, `slice`
  prints `ALLOWANCE USED: … failed in the whole GTK suite; the ledger names it
  flaky before the slice.`, keeps the failed run's artifacts in
  `target/qa-parity/allowance/<scenario>/<run>/` and goes on: the failure does
  not fail `slice`. The run is not run again. It fails alone more often than in
  the suite, eight times in nine against two in five, so a rerun would say
  nothing (`docs/rune/slice/V9b-test-image-and-exit-check.md`). Any other
  failure of the whole suite fails `slice`, with or without the flaky run
  beside it: it prints `NO ALLOWANCE: … failed in the whole GTK suite, and no
  allowance names them.` So does a suite that did not run to its end: the
  runner's closing count has to be the failures it named.
- `[[stale_patch]]` names one GTK mutation patch, `cut`, which stopped applying
  before the slice; the owner confirmed on 2026-11-11 that this is what the
  exit check's "as before" means for it. `gtk-mutations` asks `git apply --check` of every patch a
  scenario names, runs the ones that apply through `scripts/qa_mutation.py`,
  and prints `ALLOWANCE USED: cut.patch does not apply, as before the slice; …`.
  It refuses to start when a patch fails to apply without being named, or a
  named patch applies again: then its entry is removed.

An allowance is not a place to park a failure. The runner's unit tests hold the
committed list to those two entries, and refuse an entry that names no run of
the suite or no patch a scenario names, names a run that is `required`,
`parallel` or in the slice's set, gives no reason or no `issue`, or cites
under `recorded` a document that does not name it. The two entries cite their
`oracle` issues, lgse/strata#2162 and lgse/strata#2161. A new one
needs an owner ruling recorded in `docs/rune/slice/README.md`, and a change to
that test.

### Fuzzing

The fuzz driver is the second half of the parity plan's verification method
(`docs/rune/parity/README.md`, "Differential fuzzing"). A seed picks a sequence
of actions from an area's alphabet, the runner replays it on a fresh fixture,
and after every step it records what a user could observe once the application
has come to rest. Two replays of one sequence are then compared field by field,
and a sequence whose replays differ is shrunk to the fewest steps that still
differ.

Today both replays are on one target, which is what calibration needs: a field
on which GTK does not agree with itself cannot be compared with Rune. Comparing
the GTK target with the Rune target is package F2's (lgse/strata#2138).

```bash
./scripts/qa-fuzz.sh run --alphabet calibration --seeds 1..20   # the smoke tier: generate, replay twice, compare
./scripts/qa-fuzz.sh run --alphabet calibration --seeds 7 --length 80 --repeats 3
./scripts/qa-fuzz.sh replay                                     # every kept sequence that is not parked
./scripts/qa-fuzz.sh replay --alphabet calibration              # one area's
./scripts/qa-fuzz.sh replay --parked --repeats 24               # a survey: the parked sequences, on purpose
./scripts/qa-fuzz.sh shrink --alphabet calibration --seeds 7    # shrink one seed's divergence and confirm it
./scripts/qa-fuzz.sh shrink tests/qa/fuzz/corpus/calibration/fuzz-calibration-v3-0000000000000007.jsonl
./scripts/qa-fuzz.sh generate --alphabet calibration --seeds 7  # print the sequence, alone on stdout; replay nothing
./scripts/qa-fuzz.sh scenario <file>                            # run a sequence file as the scenario it is
./scripts/qa-fuzz.sh compare <stream> <stream>                  # compare two kept streams
./scripts/qa-fuzz.sh check                                      # load the alphabets, the map, the mask and the corpus
```

`scripts/qa-fuzz.sh` runs in the pinned image with the plumbing and the
`target/qa-container` cache of `scripts/qa.sh`, so it is one more QA script to
run one at a time. It takes no lock of its own: on a machine several sessions
share, the caller holds theirs, as for `qa.sh`. It builds the runner as `qa.sh`
does and passes it `--fuzz` and the command: the driver is part of
`tests/qa.rs`, in `tests/qa/fuzz.rs` and `tests/qa/fuzz/`. `--fuzz` is an
option and never a word, so no test filter can be taken for it. A host run is
refused and labelled as a scenario's is. It does not run in CI, by the owner's
ruling that parity and fuzzing are run by the agent before merging.

| Selection | Meaning |
| --- | --- |
| `--alphabet <area>` | An alphabet of `tests/qa/fuzz/alphabets`. `run`, `shrink` and `generate` need one; `replay` takes one to replay that area's corpus only. |
| `--seeds <seeds>` | `7`, `1..200` with both ends, or `3,9,0x2a`. A seed is a 64-bit number, and names a sequence only together with the alphabet's version. |
| `--length <n>` | Actions in a generated sequence; 40 by default. An action can be several steps. |
| `--repeats <n>` | Replays of each sequence; 2 by default. Every replay is compared with the first. |
| `--jobs <n>` | Sequences replayed at once; one per CPU by default, 256 at most. A sequence's own replays run one after another. |
| `--target gtk\|rune` | The target every replay starts; `gtk` by default. `rune` builds `strata-rune` first, as `qa-parity.sh` does. |
| `--keep` | Keep the artifacts of sequences that agree too. |
| `--parked` | `replay`: the parked sequences and no other, as a survey. |
| `--corpus` | `shrink`: write a confirmed sequence into the area's corpus. |
| `--limit <n>` | `shrink`: how many shorter sequences to try before the confirmations; 200 by default. |
| `--perturb <json>` | Change one observation on purpose: see "Proving the comparator and the shrinker". |
| `--rest-samples`, `--rest-poll-ms`, `--rest-budget-ms` | How rest is told. Calibration set the defaults; a run that changes them is not evidence. |

What a command's exit status says:

| Command | Exits 0 when |
| --- | --- |
| `run`, `replay` | It replayed at least one sequence, every one agreed and came to rest at every step, none failed to replay and none showed a finding. A selection that replays nothing fails, as `qa.sh` fails one that runs no scenario. |
| `replay --parked` | Every parked sequence could be replayed. What each does now is in its line. |
| `shrink` | It found a divergence and the shortest sequence confirmed it three times out of three. It fails when there is nothing to shrink, and when the divergence does not confirm. |
| `compare` | The two streams agree. |
| `scenario` | The ordinary runner passes the file. |
| `generate`, `check` | The alphabet, and for `check` everything else the driver reads from the tree, loads. |

#### Alphabets

An alphabet is `tests/qa/fuzz/alphabets/<area>.toml`. It belongs to an area:
each area package adds its own with its first required runs.

```toml
area = "calibration"
version = 3
about = "one line on what it reaches and what it leaves out"
header = '{"modes":["columns"]}'

[[action]]
name = "down"
weight = 10
steps = ['{"key":"Down"}']

[[action]]
name = "click"
weight = 4
steps = ['{"click":{"entry":"<entry>"},"when":{"present":{"entry":"<entry>"}}}']
choose.entry = ["archive", "documents", "readme.md"]
```

| Key | Meaning |
| --- | --- |
| `area` | The file's stem: lowercase words joined by hyphens. |
| `version` | Raised by every change that makes a seed write another sequence. It is part of every sequence's name. |
| `about` | One line. |
| `header` | The scenario header of every sequence, as JSON, without `scenario`: the mode, and the tree and preferences when they are not the defaults. |
| `action.name`, `action.weight` | A unique name, and how often the action is picked against the sum of the weights. |
| `action.steps` | One or more scenario steps, each a line of JSON exactly as a scenario file has it, in TOML's single quotes. |
| `action.choose.<name>` | The values `<name>` in the steps may take. One is drawn for each name, in name order, every time the action is picked. |

**A seed means nothing across versions of an alphabet.** Seed 7 of version 2
and seed 7 of version 3 are two sequences. So `version` is held to what seeds
write: `tests/qa/fuzz/alphabets/pins` has one line for each alphabet,
`<area> v<version> <digest>`, where the digest is of the generator's version,
the header and every action's name, weight, steps and choices. The runner
refuses an alphabet whose line does not match, and says what the line should
be. Changing what seeds write means raising `version` and writing the new
line; changing a comment or `about` means neither.

The rules an alphabet keeps, the first three checked when it loads:

- **A step acts or waits.** It is a process step that does something (`key`,
  `type`, `click`, `select`, `activate`, …), an `fs` action (`write`, `mkdir`,
  `remove`, …), or a `wait` or `settle` where the area knows what the
  application shows when it is done. Steps that only ask (`expect`, `holds`,
  `query`, `note`, `during`) have no place: every step is observed anyway.
  Runner steps other than `fs` are not replayed yet.
- **A step stands alone.** It takes no `if`, `unless`, `as`, `modes` or
  `variants`, since a shrunk sequence keeps a step without its neighbours.
  `when` is its one condition.
- **One run.** The header gives one mode and no variants, and none of
  `session_bus`, `portal`, `input_method`, `fixture_root`, `layout`,
  `teardown`, `images`, `mutation`, `known_bug`, `skip`, `parity` or
  `targets`.
- **Targets by role and name.** Never a screen point or an offset tied to one
  toolkit's row heights, so that each target resolves the step in its own
  geometry.
- **Guard what may not be there.** The generator does not look at the
  application, so it will pick a click on an entry that is not listed. Without
  a `when`, the target waits 15 seconds for it and then refuses the step. With
  `"when":{"present":…}` the step is skipped at once, and skipping is an
  outcome the two replays must agree on.
- **Stay inside the fixture.** A replay ends at the step that leaves it, so
  the rest of the sequence is wasted. Guard a step that leaves on its own,
  such as Left of the fixture's own column, with
  `"when":{"not":{"directory":"$root"}}`.

`calibration` is the driver's own alphabet, not an area's: a little of every
kind of action in Columns on the canonical fixture, so that every field of an
observation moves sometimes. The Columns pilot alphabet is F2's.

#### Sequences

A sequence is a scenario file: the header line, then one step to a line. It is
named for where it came from, `fuzz-<area>-v<version>-<seed>`, the alphabet's
version and the seed as sixteen hex digits, and the ordinary loader checks it
as it checks every scenario. `generate` prints one, alone on stdout, so
`generate … > <file>` writes a scenario file; what the image and Cargo say
goes to stderr.

`scenario <file>` runs one through the ordinary runner, to read a transcript
or a screenshot of it. That run is not a verdict: the ordinary runner stops at
the first step the target refuses, and a generated sequence has about one such
step in forty actions, usually before whatever it was kept for.

The same seed, alphabet version and generator version (`GENERATOR` in
`tests/qa/fuzz/generate.rs`) write the same bytes on every machine: the
generator is SplitMix64 in integer arithmetic and copies each step's text from
the alphabet. A longer sequence of a seed starts with the shorter one. A runner
unit test holds both to a sequence worked out by a second implementation.

#### Replay and outcomes

A replay seeds a fresh fixture and home and starts the target on them exactly
as a scenario run does, then sends the steps. Unlike a scenario run it does not
stop at a step that goes wrong: how the step ended is its outcome, and the next
step is sent.

| Outcome | Means |
| --- | --- |
| `ok` | The target took the step. |
| `skipped` | The step's `when` did not hold, so it was not sent. |
| `refused` | The target answered with an error: a target that is not there, a step it cannot run. An `fs` action the runner could not do is refused too. |
| `failed` | A check inside the step did not hold. |
| `crashed` | The target panicked, exited with an error or stopped answering. The stream ends here. |
| `closed` | The target exited as it does when its window is closed. The stream ends here, and the tree and the settings it left are compared. |
| `left` | The step took the application to a folder outside the fixture. The stream ends here. |

A replay that leaves the fixture ends because what is listed out there is the
machine's, not the sequence's: `/tmp` holds every other run's folders. The
application has left once its first pane is neither the fixture's own nor a
folder of the fixture's tree, and that observation keeps the tree and the
settings and no `app`. Two replays must agree on the step that left, as on a
step that closed the window. So an area whose behaviour is out there (the
trash, Recent, Home, search across folders) cannot be fuzzed past that step
yet, and `fixture_root`, which would give such a place a fixed name, is among
the header keys a sequence may not have.

A GLib warning or critical is recorded on the step that logged it, with its
addresses taken out. A crash and a GLib warning are findings: the command
reports them and fails whether or not the replays agree on them.

#### The observation

After every step, and after the launch, the runner takes one observation. It
is a fixed projection, and nothing else of a reply is kept:

| Field | Holds |
| --- | --- |
| `result` | The step's outcome. |
| `criticals` | What GLib logged since the last observation. |
| `app.*` | The ten keys of the reply's `app` snapshot: `view_mode`, `directory`, `panes` (each pane's `name`, `selected` and `focused`), `focused`, `focused_pane`, `dialog`, `menu`, `preview`, `field` and `matches`. A runner unit test holds the list to the snapshot's own keys, so a key added there is not left uncompared by oversight. |
| `fs.<path>` | The fixture tree: `dir <mode>`, `file <mode> <size> <digest of its bytes>` or `link <target>` for each path. |
| `settings.<key>` | `settings.toml`, parsed, so two files that say the same in another order are equal. |

The fixture's random name and the paths around it are written `$root`,
`$fixture` and `$environment` wherever they appear. A stream line carries the
digest of the tree and of the settings (`fs_digest`, `settings_digest`) beside
the listings, which are what says which path differs.

What the projection cannot see, whatever the mask says:

- **Anything `app` does not report**: the entries of a pane other than the
  deepest (`matches`), scroll position, the sidebar, tabs, the location bar,
  the footer, and what the window looks like.
- **File times**, which follow the clock.
- **Anything outside the fixture**: the trash under `$data`, the thumbnail
  cache, the per-folder view settings.
- **A reply's `changed` and `result`**: the first is which keys moved since
  the reply before, which depends on when that one was written.

#### Rest

A reply is written while the application may still be working, so a single
read is not an observation: a pane is listed before or after its entries
arrive, a dialog is still closing. The runner cannot see the application's
timers, so rest is what it can see from outside: it reads the snapshot, the
tree and the settings again and again, and the step is at rest once
**eight reads in a row, 50 ms apart, are equal**, which is a quiet window of
350 ms. "Calibration" below says where the number comes from.

The wait is a budget, never an assertion. A step that has not come to rest
after 10 seconds is recorded as unsettled, and the comparator then compares
its outcome and its criticals only. That is not agreement: a target that
never rests would agree on everything. So a sequence with an unsettled step
is reported as `UNSETTLED` and fails the command, as a difference does.
Calibration saw none on GTK.

#### Comparison, the map and the mask

The comparator reads two streams step by step and stops at the first step
where a field differs: it reports the step, the action just taken, and every
field that differs there, the first one first, in the order of the table
above and then by name. A list of names (`selected`, `matches`, `menu`,
`preview`) is one value; `panes` is compared pane by pane while both sides
have as many. A stream that ends early differs from the other at the step
that ended it. A GLib warning and a crash are divergences in themselves,
reported at their step even when both replays show the same one, and no map
or mask entry covers them.

Two files say which fields a comparison leaves out, and they are kept apart on
purpose:

| File | Holds | Applies to |
| --- | --- | --- |
| `tests/qa/fuzz/normalisation.map` | Exceptions: a field on which the two targets may differ, under one of the plan's exception codes (`timing`, `a11y`, `rune`, `oracle`, `toolkit`, `deviation`, `platform`) and an issue | Comparisons between two targets only |
| `tests/qa/fuzz/calibration.mask` | Noise: a field on which two replays on one target differ, with its cause (`unsettled`, `clock` or `environment`) and an issue | Every comparison |

Both have one entry to a line, `<field> <code> <issue>`, sorted by field, with
`#` lines for comments, so two branches that each add a line merge cleanly:

```text
# The size and date of a preview arrive after its text; nothing shows that they are due.
app.preview unsettled lgse/strata#NNNN
```

A field is named as an observation names it: `app.focused`,
`app.panes[].selected` with `[]` for every element of a list,
`fs.documents/notes.txt`, `settings.show_hidden`, `settings.keymap.default`.
An entry covers its field and what is below it at a boundary of the path, and
no more: `app.focused` does not cover `app.focused_pane`, `fs.documents`
covers every file of the folder, and `fs.documents/notes` does not cover
`fs.documents/notes.txt`. What is below `fs.` is a path, taken whole with
whatever dots and brackets its names have.

An entry names a key of `app`, a path of `fs` or a key of `settings`. A whole
root (`app`, `fs`, `settings`), `result` and `criticals` are refused: one line
must not blind a run. The issue is written `lgse/strata#<number>`. A mask is
not an exception and cannot carry an exception's code, nor the reverse. The
exception codes are the ones a twin's label takes, from one list in
`tests/qa/twins.rs`, with `platform`.

The map starts with no rule. **A mask grows only when two GTK replays of a
seed differ**, and the comment above it says what the differential mode can no
longer see. Every summary prints both counts, by code, and `check` prints them
without replaying anything: they are the "normalisation rules by code" of a
checkpoint report.

`compare` takes two kept streams. Each stream's first line says what it is a
stream of: the target, the sequence, the alphabet and its version, the rest
rule, and whether the harness perturbed it. `compare` refuses two streams of
another sequence, alphabet version or rest rule, and applies the map when
their targets differ.

#### Shrinking

`shrink` replays the sequence twice, at once, and takes the divergence it
finds. It drops the steps after the divergence, then takes runs of steps out,
halves first and single steps last, keeping each removal after which the same
divergence still shows first: the same field, after the same action, between
the same two values, so that it cannot slide to some other difference on that
field. Then it replays the shortest sequence three more times: a divergence
is confirmed when it shows **three times out of three**.

One that does not confirm is noise. It goes to calibration (a longer rest, or a
mask), never to an issue, and the command says so and fails. The exception is
a race in the target itself, which the shrinker cannot tell from noise: when
24 replays of the whole sequence go two ways from the same input, each at
rest, it is the target's fault. It is filed as an `oracle` issue, recorded in
`docs/rune/parity/findings.md`, and its sequence is parked unshrunk, as for
lgse/strata#2177 and lgse/strata#2201. A confirmed one
prints the steps, the first differing field and the title a `divergence` issue
takes (`[fuzz <area>] <field> differs after <action>`), and with `--corpus` is
written into the area's corpus.

#### Proving the comparator and the shrinker

With one target there is no real divergence to find, so the harness makes one.
`--perturb` changes one field of one observation of the last replay, and
leaves the application alone:

```bash
./scripts/qa-fuzz.sh shrink --alphabet calibration --seeds 0x13 \
  --perturb '{"after":{"key":"Up"},"while":{"app.focused":"report.md","app.directory":"documents"},"field":"app.focused","value":"perturbed"}'
```

The change is made after the step `after` (compared without its `when`), and
only when the observation before that step held every `while` field. So the
divergence depends on what the application really did, and the shrunk sequence
has to keep the steps that bring it there.

A perturbed run says so wherever it leaves a trace: a `FUZZ PERTURBED` line at
the start and the end, `PERTURBED` in front of every `DIFFER` and `SHRUNK`
line, `"perturbed": true` in `divergence.json` and in the stream's first line,
and `-perturbed` in the names of the changed replay's files. It prints no issue
title, and `--corpus` refuses it: a perturbed divergence is the harness's and
is evidence of nothing else.

#### The corpus and parked sequences

`tests/qa/fuzz/corpus/<area>/` keeps the sequences of an area that showed a
divergence, each a scenario file under its own name. While the finding of one
is open it is parked: the file `parked` in the area's folder lists it as
`v<version>-<seed> <code> <issue>`, which reads as the end of its file's name,
sorted by version and seed. The code is an exception code, or `divergence`
while the finding has none yet. The issue is the finding's reference, and may
be closed: the finding's state is in `docs/rune/parity/findings.md`. Once the
finding is fixed its line goes, and from then on `replay` holds the sequence
to agreeing with itself, which is what the smoke tier adds to fresh seeds. A sequence becomes a named scenario only when
no scenario owns the behaviour, as the test-value rules in `AGENTS.md` ask.

What a parked line stops:

- **`run`** names and skips a seed only when it is parked on the alphabet's
  own version, where the seed writes what was parked. A line for another
  version stops nothing: that seed writes another sequence now, and it runs
  like any other.
- **`replay`** names and skips every parked file, whatever version it is of.
- **`replay --parked`** replays the parked files and no other, on purpose,
  and says of each whether it `STILL DIFFERS` or `AGREES IN <n> REPLAYS, STILL
  PARKED`. One that agrees is not thereby fixed: a race agrees most of the
  time. Give the survey as many `--repeats` as the issue says the race needs.

The runner holds the corpus to this when it loads it: every file's name says a
version the alphabet has had, every parked line has its file, and a file
parked on the alphabet's own version is made of steps its seed still writes,
in their order.

#### Summaries and artifacts

Each sequence prints one line, and the command ends with a summary:

```text
FUZZ run: alphabet calibration v3, generator 1, 40 actions; 300 sequences
FUZZ replays: target gtk, 2 of each sequence, 24 sequences at once
FUZZ rest: 8 equal samples 50 ms apart, budget 10000 ms
FUZZ rules: 0 normalisation rules (tests/qa/fuzz/normalisation.map); 0 calibration masks (tests/qa/fuzz/calibration.mask)
AGREE fuzz-calibration-v3-0000000000000001: 40 steps, 2 replays
DIFFER fuzz-calibration-v3-000000000000002a at step 14 {"key":"Right"}: app.matches (["a"] | ["a","b"]); kept target/qa-fuzz/calibration/fuzz-calibration-v3-000000000000002a
FUZZ result: 300 sequences: 299 agree, 0 agree under a mask, 1 differ, 0 unsettled, 0 could not be replayed; 0 parked; 0 with a finding
FUZZ steps: 24600 observations in 600 replays, 12300 compared; 270 refused, 1800 skipped, 0 failed, 0 unsettled, 0 GLib warnings or worse
FUZZ replays that ended early: 4 left the fixture, 0 closed the window
FUZZ late changes: 2890 of 24600 observations changed after their step's reply; quiet before a late change: median 60 ms, 99th percentile 170 ms, longest 215 ms
FUZZ fields that differed first, with nothing masked, by sequence: app.matches 1
FUZZ load average: 2.10 at the start, 21.30 at the end
FUZZ artifacts: target/qa-fuzz/calibration/
```

| Line | Is |
| --- | --- |
| `AGREE` | A sequence whose replays agree on every field. |
| `MASKED` | One whose replays differ only on masked fields. It counts as agreeing under a mask, and the "fields that differed first" line counts it with nothing masked, which is how calibration sees what a mask is hiding. |
| `DIFFER` | One whose replays differ: the step, the action, the first field and its two values. |
| `UNSETTLED` | One with a step that did not come to rest within the budget. |
| `FINDING` | A crash or a GLib warning. It comes with a `DIFFER` line that names its step. |
| `ERROR` | One the harness could not replay. |
| `PARKED` | One that is parked, and was not replayed. |
| `FUZZ late changes` | How many observations changed after their step's reply, and how long each such change was in coming after the read before it. A change is seen only while the rest wait still runs, so the default window sees up to 350 ms and `--rest-samples 40 --rest-poll-ms 25` up to a second. |

A sequence that differed, did not come to rest, showed a finding or could not
be replayed (every sequence, with `--keep`) leaves
`target/qa-fuzz/<area>/<sequence>/`:

| File | Holds |
| --- | --- |
| `sequence.jsonl` | The sequence, a scenario file. |
| `<target>-<n>.stream.jsonl` | Replay n's stream. Its first line is `{"stream":…}`: the target, the sequence, the alphabet and its version, the rest rule, and `perturbed`. Then one observation to a line: `step`, `action`, `settled`, `polls`, `waited_ms`, `changed_ms` (when, into the rest wait, a read differed from the one before), the two digests and `observed`. A last `{"gone":…}` line says why the target was lost. |
| `<target>-<n>.transcript.jsonl` | Every command and reply of the replay, the rest wait's reads included. |
| `<target>-<n>.strata.log` | The application's log. |
| `<target>-<n>.input.log` | Every synthetic key, motion and button of the replay, as a scenario run's `input.log` has them. |
| `divergence.json` | The first differing step and fields, with and without the mask, the issue title, and `perturbed`. |
| `shrunk.jsonl` | After `shrink`: the shortest sequence. |

#### Calibration

The projection and the rest rule were fixed by measuring two GTK replays of one
seed against each other, in the pinned image, with the `calibration` alphabet.
This is what F1 found on `strata-next` in November 2026. The counts of seeds
and observations below are from the driver as it merges, version 3 of the
alphabet; the commands are on lgse/strata#2181. The runs made while the
driver was being built are history: they found the first four faults and set
the window. Of their numbers, only what each of those faults did then is
quoted, in the table of faults, and the three delays named below. Measure
again before changing the rest rule, and after any change that moves the
application's own timers.

**How long GTK goes on changing after a reply.** With the quiet window
stretched to a second (`--rest-samples 40 --rest-poll-ms 25`), the summary's
`FUZZ late changes` line counts every late change:

| | 24 at once, seeds 1 to 100 | 96 at once, seeds 1 to 96 |
| --- | --- | --- |
| Observations | 7,986 | 7,678 |
| Changed after their step's reply | 10.7 % | 10.3 % |
| Quiet before a late change: median | 66 ms | 69 ms |
| 99th percentile | 169 ms | 175 ms |
| Longest | 224 ms | 265 ms |

One step in ten is not at rest when its reply is written, which is V3's
finding with a number on it. The late changes with the longest quiet before
them are the application's own delays, as the earlier runs' streams showed
them: a dialog is still named for about 165 ms after the Escape or Return
that closes it, the inline editor of a new folder opens about 200 ms after
its row is listed, and a file written behind the application's back is
listed 150 to 170 ms later.

The runs with the default window see a late change only up to 350 ms, and saw
none after more than 256 ms of quiet at one sequence to a core (115,375
observations) or 273 ms at 96 at once (24,492).

**What the quiet window has to be.** It is the longest quiet before a late
change at one sequence to a core, about 220 to 260 ms, and a margin: eight
equal reads 50 ms apart, 350 ms. Under 96 sequences at once the margin is
thinner: an earlier run of the table's second column, on the same code, saw
304 ms. Shorter windows were tried on seeds 1 to 300, two replays each:

| Quiet window | Seeds whose replays differed |
| --- | --- |
| 150 ms (`--rest-samples 4`) | 3 of 300, each on `app.field`: a new folder's editor, open in one replay and not yet in the other |
| 250 ms (`--rest-samples 6`) | 0 of 300 |
| 350 ms, the rule | 0 of 300 |

So what a short window cuts, here the editor of a new folder, is cured by the
wait. 250 ms was enough for these seeds, and the rule keeps a margin over it.

**What two GTK replays differed on, and what was done about it.**

| Field | Cause | Cure |
| --- | --- | --- |
| `app.field`, `app.focused`, `app.panes[].focused`: a new folder's inline editor, open or not yet | The editor opens about 200 ms after the row, with nothing to see in between | Rest: the 350 ms window |
| `app.dialog`: still named after the key that closes it | The dialog goes about 165 ms after the key | Rest: the 350 ms window |
| `app.matches`: the listing of `/tmp` | The sequence left the fixture, and `/tmp` holds every other run's folders. Left of the fixture's own column does it, and so does Return once the arrow keys have taken focus up to the path bar | The driver: a replay ends at the step that leaves the fixture. The alphabet guards Left besides, so that fewer sequences end early |
| `app.field.caret`, `app.field.selection`: after Shift+click on the entry being renamed | A race in GTK, whatever the rest: the same input events clear the selection in 28 replays of 34 and select the whole name in the other 6 | An `oracle` issue, lgse/strata#2173, and its seed parked. The alphabet does not click an entry while a name is edited: where such a click puts the caret is a matter of pixels in any case |
| `app.preview`: which file the preview shows after Move to Trash from a context menu on a filtered pane | A race in GTK, whatever the rest: a file that is neither focused nor selected in 23 replays of 24, the focused one in the other | An `oracle` issue, lgse/strata#2177, and its seed parked. The alphabet is left as it is, so another seed can meet it |
| `app.focused`, `app.panes[].focused`: after a click that opens a folder's column, with 96 sequences at once | A race in GTK between the column's listing and focus moving into it: focus on the column in one replay, on its first entry in the other. Seen once, and not at one sequence to a core | An `oracle` issue, lgse/strata#2180, and its seed parked |
| `app.matches`, `app.menu`: after Space in a column's sort menu, opened from the keyboard and left open, once a right-click and Down have gone to it | A race in GTK, whatever the rest: from the same input events Space puts the folder back in its default order in 16 replays of 24, and in another order in the other 8; the menu stays open either way. Met by seed `0x1de` in the last run of seeds 1 to 1000; the shrinker took it from 40 steps to 10, which differed in two confirmations of three | An `oracle` issue, lgse/strata#2201, and its seed parked, after that run |

One more `oracle` issue is not a difference: GTK logs a critical, the same one
at the same step in both replays (lgse/strata#2176). A GLib warning is a
divergence in itself, so the comparator reported it, and the shrinker took it
from 40 steps to 7.

**Nothing is masked.** `calibration.mask` is empty, the map has no rule, and
the projection is all ten `app` keys, the tree and the settings. With the rule
above, two GTK replays of a seed agree on every observation but where GTK
itself races or warns:

| Run | Seeds replayed | Agreed | The others |
| --- | --- | --- | --- |
| 40 actions, two replays, 24 at once: seeds 1 to 1000 | 1,000 | 999 | Seed `0x1de`, the sort menu race (lgse/strata#2201), parked since |
| The same at 96 at once: seeds 1001 to 1300 | 299 | 299 | Seed `0x41c` is parked and was not replayed |
| 120 actions, three replays, 24 at once: seeds 2001 to 2100 | 100 | 100 | |

No step was left unsettled and no GLib warning was logged in any of them:
139,867 observations in 2,898 replays. Five GTK faults were found in all, and
all five are parked: seed `0x1de` was parked after the run that met it, so a
run of seeds 1 to 1000 now names it as parked and replays the other 999.

These runs were made with `strata-next` at `682fa0e7f`. They were not repeated
after #2192 and #2195 to #2197 merged application code: the driver is the
same, and the smoke tier agrees at the head.

`replay --parked --repeats 24` says what the four sequences parked then do on
the same code: the critical of lgse/strata#2176 in every replay; the Shift+click of
lgse/strata#2173 one way in 23 replays and the other in 1; the preview of
lgse/strata#2177 on one file in 19 and on another in 5; and the focus of
lgse/strata#2180 the same in all 24, at four sequences at once and not the
load it was met under. The rates move from run to run, which is why a parked
sequence that agrees is not thereby fixed.

**What that does not cover.**

- **One alphabet, one view.** Columns, the canonical fixture, the actions
  `calibration.toml` lists. `app.view_mode` never changes there, and
  `settings.toml` changes on one action only. No file operation ran long
  enough to show progress. An area whose actions wait on longer delays than
  260 ms (a copy with its progress dialog, a per-folder view saved after
  500 ms) has to measure its own, and where the application shows that it is
  busy, wait on that with a `wait` step.
- **Rest is told from outside.** A change that comes after more than 350 ms
  of quiet is taken for the next step's. Two replays then usually agree with
  each other and are both wrong about which step did it, which a comparison of
  two GTK replays cannot see and a comparison with Rune will.
- **What a replay that leaves the fixture does not reach.** A replay ends at
  the step that leaves the fixture, and what the sequence would have done
  after it is not observed: 58 of 2,000 replays and 1.3 % of the observations
  at 40 actions, 30 of 300 replays and 5.2 % of the observations at 120.
- **Rarer races.** Each race above was met in one seed of several hundred or
  more. One that shows in one seed of ten thousand was not met.
- **The clock.** A file made during a replay has the time of the replay, and
  a preview shows its date. Two replays on either side of midnight would
  differ there; none did.
- **A machine starved of CPU.** "96 at once" is 96 sequences on 24 cores, and
  a replay mostly sleeps between its reads: the load average ended between
  12.7 and 30.6 in the runs at 96 at once, on a machine other sessions were
  using too. So the machine was at most a little oversubscribed, and what a
  replay does when it cannot get a core for long was not measured.

#### What the next packages build on

- **The sentence for the checkpoint.** On the final code, 999 of 1,000 seeds
  agreed at 24 at once, 299 of 299 at 96 at once and 100 of 100 at 120
  actions with three replays, with nothing masked and no step unsettled; the
  one that differed is a race in GTK's own sort menu; five GTK faults were
  found in all, all parked. That holds for one alphabet, Columns on
  the canonical fixture, on a machine that is not starved of CPU; with an
  alphabet that keeps every click, Ctrl+click, Shift+click and right-click
  off the entries while a name is edited; and with a replay ending where it
  leaves the fixture, which left 1.3 % of the observations unmade at 40
  actions and 5.2 % at 120. The calibration ran with `strata-next` at
  `682fa0e7f` and was not repeated after #2192 and #2195 to #2197 merged
  application code: the driver is identical, and the smoke tier agrees at the
  head.
- **"Agree" is a rate.** Two replays catch a race that goes the rare way once
  in 24 about 8 % of the time, one that does so once in 6 about 28 %, and one
  that does so once in 3 about 44 %. Two of the races above agreed in a whole
  batch before they were seen. So a
  difference between GTK and Rune is replayed GTK against GTK many times, 24
  and not 2, before Rune is blamed for it.
- **Two targets.** A replay starts the target `STRATA_QA_TARGET` names, which
  is one for the whole process: `Launch::binary` in `tests/qa.rs` is the only
  place the replay path reads the target. Comparing GTK with Rune needs
  `Launch` to take the target, or two processes and `compare` over their kept
  streams, which already applies the map when the targets differ. That, the
  map's first rules and the Columns pilot alphabet are F2's.
- **The rename guard is not for copying.** `calibration` skips every click,
  Ctrl+click, Shift+click and right-click on any entry while a name is being
  edited. That is far wider than the race of lgse/strata#2173, which is a
  Shift+click on the entry being renamed, and it keeps the pointer out of
  rename altogether. Rename is in the Columns pilot's scope, so the pilot
  needs a narrower guard of its own.
- **Outside the fixture.** `left` ends a replay at any visit to the trash,
  Recent, Home or a search across folders, as above.
- **Runner steps other than `fs`.** `restart`, `layout`, the session bus and
  its peers, portals and input methods are refused when a sequence loads.
- **Invariants.** Properties of one target with no GTK peer to compare are
  F3's (lgse/strata#2139). `Stream` and `Observation` in
  `tests/qa/fuzz/replay.rs` and `observe.rs` are what a check of one stream
  reads.
- **`scenario` is for reading.** A sequence with a refused step fails under
  the ordinary runner before it reaches what it was kept for, so a sequence
  file is not something to hand to `qa.sh`.

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

- `rename-and-undo-restores`, and `undo-created-folder-trashes`
  (typed-name) for the name a new folder takes: settles the renamed row
  before Ctrl+Z. The directory monitor can list the new name, and the
  listing select and focus it, before the rename's own completion publishes
  its undo and ends the operation; a Ctrl+Z in between is dropped. The app
  shows nothing a run can read to tell that the undo is ready (Undo has no
  menu item or action state), so the wait is the settle. Under the same
  parallel load, 3 of 60 `rename-and-undo-restores` runs without it lost the
  Ctrl+Z and none of 60 with it did (6 of 116 and 0 of 84 over every probe).
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

## The Rune side

`strata-platform`, `strata-kit`, `strata-components` and `strata-gallery` are tested apart
from the GTK application, in their own cargo runs (`docs/workspace.md`, "Two lanes").
[`docs/rune/testing.md`](rune/testing.md) is the full reference: gallery pages, the script
format, the PNG tolerance, blessing and the compositor tests.

| Command | Runs |
| --- | --- |
| `mise run test-rune` | Every Rune-side test, headless |
| `mise run test-rune c1::` | Page c1's three golden tests. The trailing `::` keeps `c1` from matching `c10` |
| `mise run bless c1` | Rewrites only `tests/golden/*/c1/`. Review every diff before committing it |
| `mise run compile-features` | `strata-platform` with no features, then with each feature alone |
| `mise run test-platform` | The compositor tests, each in a private headless sway session |

- **`strata-app`'s QA backend is tested here too.** `mise run test-rune qa::` runs its key,
  role and layout tables and three sessions of the QA engine against the offscreen host;
  `strata-rune`'s `tests/qa.rs` drives the binary over its own stdin and stdout. Both
  need the `qa` feature, which the task's `--all-features` turns on.
- **Nothing reaches the desktop.** The tasks clear `DISPLAY` and `WAYLAND_SOCKET` and point
  `WAYLAND_DISPLAY` and the session bus at names that do not exist. The tests run on Rune's
  virtual clock.
- **The test data is laid out by gallery page.** `<id>` is a page id such as `c1`, `c14a` or
  `gallery` for the index, so a change adds and blesses only its own files.

  | Path | Holds |
  | --- | --- |
  | `tests/golden/a11y/<id>/<scene>.json` | Each golden scene's accessibility tree, without bounds, scroll offsets, text selection or shortcuts |
  | `tests/golden/png/<id>/<scene>.png` | Each golden scene rendered headless at scale 1 |
  | `tests/golden/scripts/<id>/<script>.json` | The accessibility tree each script ends on |
  | `tests/scripts/<id>/<script>.json` | Rune headless input scripts, one per behaviour group. Each names the feature-map Behavior bullets it checks in `bullets` |
  | `tests/fixtures/code/` | Source files the code view's highlighting tests read |

  `crates/strata-gallery/tests/goldens/` checks all of them.
- **PNG goldens and scripts need a Vulkan adapter.** The harness takes lavapipe where it is
  installed and the default adapter otherwise. The goldens were blessed on an NVIDIA card.
- **They do not run in the pinned test image or in CI.** The image has no Vulkan driver, and
  `./scripts/quality.sh` selects the GTK lane. CI checks the Rune side's formatting,
  dependency policy and spelling only.

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
pinned GUI/font environment, Rust, and the locked Cargo dependencies of both
lanes, fetched, with the QA builds' compiled
([The image's dependencies](#the-images-dependencies)). A stub of every
workspace crate stands in for Strata's sources; what it built is removed, and
no tested application binary is published. The workflow watches
`crates/*/Cargo.toml`: a change to a manifest deeper than that, such as
`strata-test-peer`'s, publishes only when `Cargo.lock` changes with it.

Tags include SHA256 input identifiers, architecture, UID, and GID. Environment
inputs are independent of application source; the build identifier additionally
includes `Cargo.toml`, `Cargo.lock`, and `.dockerignore`, and the publisher uses
those build and BuildKit cache tags to skip or speed up its own rebuilds.
Existing publications are checked before rebuilding. A separate
`base-<environment-inputs>-linux-amd64-1001-1001` tag is the reusable
environment that `scripts/image_base.py` pulls and CI's quality and QA jobs
run. It is published once per environment input set and is not retagged on
ordinary Cargo changes, so the dependencies a run seeds from are those of the
commit that changed the environment; a later `Cargo.lock` fetches its
difference on first use. GHCR storage is independent of Actions cache eviction;
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
