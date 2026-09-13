# End-to-end GUI testing

The Rust suite covers state transitions, filesystem edge cases, and individual
widgets. The end-to-end suite in `tests/e2e` drives the real GTK
application on a headless X display, sends real keyboard and pointer input, and
checks both what the window reports and what happened on disk.

## Running it

```bash
./scripts/e2e.sh                     # every scenario
./scripts/e2e.sh -k drag             # one selection
./scripts/e2e.sh -k "clipboard and columns"
```

### Targeted local validation

Choose checks from the behavior and caller mapping, not from changed-file names
alone. Bounded changes may use a non-empty, relevant selection; broad,
cross-cutting, shared infrastructure, dependency/build/CI/harness changes, or
uncertain coverage require the full pinned quality phases and the full canonical
E2E run. Include relevant views, callers, and preference behavior, and add or
run the regression test that proves the change. Record the scope rationale,
commands, results, and intentional omissions in the handoff. After editing,
rerun affected tests rather than unrelated suites.

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

A filter must collect at least one test; use Cargo's output or a collection
check to verify that it did. `scripts/quality.sh` only accepts `all`, `fmt`,
`clippy`, or `test` and does **not** forward test filters, so it cannot be used
for a targeted test selection.

The E2E runner passes repository-relative paths and normal pytest arguments to
its configured pytest invocation. Use a file or `-k` expression, then confirm
collection is nonzero:

```bash
./scripts/e2e.sh tests/e2e/scenarios/test_inline_renaming.py
./scripts/e2e.sh -k 'rename and not visual' --collect-only
```

`./scripts/e2e.sh` is the canonical pinned-container evidence; native
`scripts/e2e-native.sh` is only for host-toolkit debugging and does not replace
it. Preserve the verified image provenance and `target/e2e-container` and
`target/quality-container` caches. CI still runs its complete unchanged gate.

Every GUI or delegated check must clear inherited display variables and use a
private Xvfb and private D-Bus session. `scripts/test-headless.py` and the E2E
runners enforce this isolation. Rust tests set `GTK_A11Y=none`, `NO_AT_BRIDGE=1`,
and `STRATA_REQUIRE_GTK_TESTS=1`; E2E enables accessibility on its private AT-SPI
bus to drive the application. Other commands must arrange equivalent isolation. A
missing isolated display or bus is a hard failure. Never run against the desktop
or an inherited session bus, silently skip GTK tests, or fall back to the desktop.

The runner prefers Podman when available; select an engine explicitly with
`STRATA_CONTAINER_ENGINE=podman` or `docker`. CI explicitly selects Docker to match
its runtime archive loader; an image in one engine's store is not visible to the
other. Normal runs verify and reuse the
local base image. If missing, they pull the published environment once, verify
its input label and platform, and record it locally. They **never automatically
build images or fetch Ubuntu packages**. A missing publication fails with an
actionable message rather than silently bootstrapping.

When intentionally updating the environment, or before its first publication,
explicitly build the local base once:

```bash
STRATA_CONTAINER_ENGINE=podman python3 scripts/e2e_base.py build
./scripts/e2e.sh
```

The recipe remains `tests/e2e/Dockerfile`: digest-pinned Ubuntu 24.04, a dated
package snapshot (GTK 4.14 and fonts), Rust 1.98.1, and pinned Python dependencies.
Changing those inputs selects a new base; application edits do not. The normal
runner still compiles the checked-out application and retains Cargo's worktree
cache. No host GTK libraries, fonts, desktop sockets, or Rust binaries are mounted.

The checkout is mounted at `/workspace`; pass test paths relative to the
repository. Build and Cargo caches live in `target/e2e-container`, separately
from native builds. Artifacts remain in `target/e2e-artifacts` and are owned by
the invoking user. Minimal generated passwd/group files provide the invoking
UID/GID to D-Bus, so one published environment works across local user IDs without
rebuilding it or mounting the host's account database.
The image includes bubblewrap for sandboxed thumbnail decoding, FFmpeg/ffprobe,
and GTK's GStreamer media backend with the base/good/libav plugins. Rust media
regressions exercise actual normalization, playback, and long-source duration
limits rather than skipping when optional host tools are missing. These packages
come from the existing dated Ubuntu snapshot; the GTK/GLib baseline and Rust
compiler are unchanged. This test-only dependency addition does not apply or
retire the version-specific GTK 4.22.4/GstPlay 1.28.6 patches in
`packaging/media-runtime/`; that opt-in kit remains unchanged and is not shipped
by this image update.

Rootless Podman
runs unmask `/proc/*` inside the test container so bubblewrap can mount its own
private `/proc`; the decoder's sandbox and the container's seccomp policy remain
enabled. Docker's outer seccomp/AppArmor profiles and system-path masks must be
disabled for the nested namespace and mount operations. This applies only to the
disposable E2E container; Strata still launches its normal bubblewrap decoder
sandbox. Neither engine uses privileged mode or mounts desktop sockets.
Updating the image inputs is an intentional rendering
environment change and requires reviewing the visual baselines.

For explicit host-toolkit debugging only, `./scripts/e2e-native.sh` accepts
`STRATA_BINARY` and `STRATA_E2E_VENV`. A native pass does not replace canonical
`./scripts/e2e.sh` evidence when targeted or full E2E validation is required.

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
Ubuntu bootstraps. Locally, environment builds still require the explicit command
shown above.

Quality's Cargo home and build directory are under `target/quality-container`,
separate from E2E and native builds. Its Actions cache is keyed by environment,
Cargo manifests, and source revision, with same-environment/manifest restoration.
Only successful quality builds on main pushes save caches; PR consumers cannot
populate main's cache. These are compilation caches, not evidence of passing tests;
the aggregate gate separately requires all test shards to pass. Cold quality
compilation includes test-only and all-feature dependencies, so the E2E application
dependency cache is not advertised as a full quality hit.

The Rust suite runs with `--all-targets --all-features --locked` inside private
Xvfb, with `GTK_A11Y=none`, `NO_AT_BRIDGE=1`, and `STRATA_REQUIRE_GTK_TESTS=1`.
GTK initialization failures cannot silently skip tests. Formatting, compiler,
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
Shard 0 is reserved exclusively for
`ui::search::tests::deferred_scroll_restoration_yields_to_updates_wheel_scrollbar_and_query_reset`.
Every other test is balanced longest-first across shards 1–3; unknown tests
receive a one-second weight and always participate. Timing hints are not an
allowlist and cannot put another test into shard 0. Validation rejects mixed
assignments or a missing, duplicated, or ignored isolated test, so renaming or
removing it requires updating `ISOLATED_TEST` and the reservation policy.
The roughly 160-second deferred-scroll regression remains unchanged and limits
the possible speedup; sharding does not shorten an individual test.

Each **Rust tests shard N** verifies the checkout revision, application/Rust-test source
fingerprint, image inputs, executable checksums, and the entire libtest inventory
before selecting exact names. Tests share neither a display nor a session bus
with another shard. The existing per-process GTK serialization still applies.
The runner checks libtest's actual selection and final passed/ignored counts
before writing a success receipt; nonzero exits, empty runnable shards, or changed
inventories fail. Existing `#[ignore]` entries stay explicitly accounted for;
sharding neither enables them nor silently ignores additional tests. Unsupported
non-libtest harness inventories fail closed rather than disappearing from coverage.

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

### Hardware-aware parallelism

The canonical container and native debugging runner default to isolated
`pytest-xdist` workers. Each worker owns its Xvfb server, private session and
accessibility buses, and input connection; scenarios within a worker run
serially. The application is built once before workers start.

Auto mode chooses the smallest of:

- half the available logical CPUs (rounded down), respecting CPU affinity and
  cgroup v1/v2 quotas, including visible ancestor limits;
- one worker per 2 GiB of available memory after reserving 1 GiB, respecting
  host `MemAvailable` and remaining cgroup memory;
- 16 workers.

At least one worker runs, including when memory availability is unknown. The
budget is detected **inside the container, after compilation**, and printed at
startup. It is a conservative resource budget, not a promise of linear speedup.

```bash
STRATA_E2E_WORKERS=auto ./scripts/e2e.sh  # default, locally and in CI
STRATA_E2E_WORKERS=8 ./scripts/e2e.sh     # explicit budget
STRATA_E2E_WORKERS=1 ./scripts/e2e.sh     # serial scenarios
./scripts/e2e.sh -n 0                   # no worker subprocess, for debugging
```

A positive override bypasses the automatic resource caps. Explicit numeric
pytest `-n` options take precedence over the environment variable. Parallel runs require
`--dist=loadgroup`: all visual-baseline scenarios share one scheduling group so
their fixed fixture directory is never claimed concurrently. Other scenarios
are distributed individually. Worker crashes fail the run without automatic
replacement or assertion retries. Baseline updates use the same grouping.
Compare worker budgets with a warm build cache; xdist does not speed up image
setup or Rust compilation.

### Keep Rust test windows off the local desktop

Some Rust tests also create GTK windows when a display is available. Run the
complete suite, or a justified targeted selection, on the same private
Xvfb/private D-Bus infrastructure with:

```bash
./scripts/test-headless.py
./scripts/test-headless.py -- --nocapture
./scripts/test-headless.py relevant_module_or_test_name
```

This requires Xvfb and AT-SPI but not the Python E2E packages. It isolates
application preferences while retaining access to the installed Cargo/Rust
toolchains, and disables accessibility bridging for Rust tests. A startup failure
aborts; it never falls back to the real display or an inherited session bus.
The E2E runner likewise clears inherited display variables before startup.

### Native debugging dependencies

These are installed inside the canonical image; only native debugging needs
them on the host.

| Purpose | Arch | Debian / Ubuntu |
| --- | --- | --- |
| Headless X server | `xorg-server-xvfb` | `xvfb` |
| Accessibility bus and registry | `at-spi2-core` | `at-spi2-core` |
| Python AT-SPI bindings | `python-gobject` | `python3-gi`, `gir1.2-atspi-2.0` |
| Private session bus | `dbus` | `dbus-daemon`, `dbus-bin` |
| Screen capture | `imagemagick` | `imagemagick` |
| Font | `cantarell-fonts` | `fonts-cantarell` |

`pytest`, `pytest-xdist`, and Pillow are installed into the virtual environment from
`tests/e2e/requirements.txt`. The environment is created with
`--system-site-packages` because PyGObject is a system package.

## How a scenario runs

Each scenario gets:

- a fresh fixture tree in its own temporary directory, with fixed modification
  times, generated by `harness/fixtures.py`;
- a throwaway `HOME` and XDG directories, seeded with a complete
  `settings.toml` so no preference is inherited from a default change
  (`harness/environment.py`);
- a private D-Bus session with no service directories, so nothing is
  D-Bus-activated behind the suite's back — no desktop portal, no document
  portal FUSE mount;
- an accessibility bus and registry started by the harness rather than by
  systemd activation.

Rendering is pinned: Xvfb at 1440x900x24 and 96 DPI, `GSK_RENDERER=cairo`,
software GL, `GDK_SCALE=1`, the Adwaita theme and icon theme, Cantarell 11,
`C.UTF-8`, `UTC`, animations off, and `reduce_motion` on.

Application preferences and file operations use only the scenario's temporary
HOME/XDG directories and fixtures. Both are created under `/tmp` so trash
capabilities do not vary when the caller's `TMPDIR` is on another filesystem. System libraries, fonts, and icon assets are
provided by the container; desktop endpoints and user configuration overrides
are not inherited.

## Writing a scenario

Scenarios talk to `harness.browser.Strata`, which locates controls by
accessible role, name, and state:

```python
def test_cut_moves_only_after_paste(strata):
    fixture = strata.fixture

    strata.select_entry("todo.txt")
    strata.keyboard.press("ctrl+x")
    assert fixture.path("todo.txt").exists()

    strata.open_directory("archive")
    strata.paste_into("archive")

    strata.wait(
        lambda: fixture.path("archive/todo.txt").exists(),
        "the cut file to arrive in archive",
    )
```

Rules the suite holds itself to:

- **Locate by accessibility, never by pixels.** Pointer targets are derived
  from a located node's accessible bounds. A literal screen coordinate in a
  scenario is a defect.
- **Synchronize on conditions, never on sleeps.** `strata.wait(...)` polls a
  predicate and, on timeout, prints the accessibility tree. There are no
  `time.sleep` calls in the scenarios; the small gaps inside
  `harness/interaction.py` are transport settling between synthetic X events,
  not waits on application state.
- **Assert on the filesystem as well as the window.** Every file operation
  checks the resulting tree, not only the listing.
- **Run one scenario per presentation where it matters.** `harness.modes`
  supplies the `ALL_MODES` parameterization.

To exercise a preference, mark the scenario:

```python
@pytest.mark.preferences(browser_mode="icons", type_to_search=False)
def test_something(strata):
    ...
```

### Inline new-entry focus regressions

`test_inline_renaming.py` checks immediate default-file/folder creation, collision
numbering, selected default names, valid-name commits on click-away, and retaining
the original name on Escape or representative invalid input. It exercises existing and newly
created items in all three views, verifies file contents, and covers repeated
renames with folder-wide or file-stem selection. `test_entry_management.py` also
covers reopening invalid edits, inside-field clicks, name conflicts, and empty
directories.

```bash
./scripts/e2e.sh tests/e2e/scenarios/test_inline_renaming.py tests/e2e/scenarios/test_entry_management.py
```

The long-name scenario sends real F2, End, Left/Right, typing, Backspace, and
inside-field clicks. It uses a 420×300 Columns window with the sidebar open,
so the column is wider than the browser viewport; List/Icons use 640×300 to
leave room for their minimum card/table widths. The adjacent Rust caret fixture
requires mapped, non-zero-width editors and overflowing text, then checks the
scroll-adjusted caret against both `GtkText` and browser bounds, including
viewport resizes. `./scripts/e2e-mutation-check.sh rename-caret` proves that
removing the viewport constraint is detected.

Keep these as real XTEST pointer interactions: emitting a focus controller's
`leave` signal in a Rust test checks the handler, not GTK's in-flight focus walk
(#566). Rename dispatch must wait until that walk returns because an operation
can refresh the row model. Creation now uses real entries rather than temporary
placeholder rows; the same rename path handles new and existing items. Rust
tests cover atomic naming collisions, Unicode validation, editor lifetimes,
cancellation/navigation before the created entry becomes visible, and scrolling
to new entries beyond the initial viewport in large directories.

### Accessible names are product surface

The harness finds an entry because Strata names it. Those names live in
`src/ui/accessibility.rs` and exist for screen readers first: an entry row is
labeled with its name and described as `Folder` or `File`, a pane is labeled
with its directory and described with its presentation, menu items carry the
menu role with the accelerator in the description, and modal dialogs carry the
dialog role and a name. When the harness cannot identify a control, the fix is
to name it in the application, not to reach around the accessibility layer.

### Input

Keyboard and pointer events go through XTEST (`harness/xtest.py`). AT-SPI's own
`GenerateMouseEvent` never replies on a headless server, so the harness talks to
the same X extension `at-spi2-registryd` would have used. Discovery and state
inspection still go through AT-SPI. GTK 4.14 reports popup-relative rather than
application-relative bounds; the harness resolves that native surface's origin
through X11 using its accessible dimensions. Controls are still located only
by accessibility semantics. AT-SPI's older `push button` spelling is normalized
to `button`, and selection tests assert actual selected-state transitions
rather than relying on GTK 4.14 to export `SELECTABLE` for unselected rows.

## Failure artifacts

A failing scenario writes a directory under `target/e2e-artifacts/<worker>/<test name>`
(the worker component is omitted with `-n 0`)
containing a screenshot, the accessibility tree, the application log, the
fixture tree listing, and the Xvfb, D-Bus, and AT-SPI logs. The path is printed
in the test output, and CI uploads the whole directory. Pass
`--keep-artifacts` to collect them for passing scenarios too.

## Visual baselines

`tests/e2e/scenarios/test_visual_baselines.py` compares a small set of stable
states with the images in `tests/e2e/baselines/gtk-4.14`: one canonical fixture
in each view, a hovered Icons tile, a selection with focus, an open context menu,
and a confirmation dialog. Local and CI runs use this one rendering profile. Other host GTK
versions do not have separate baselines; native baseline runs fail rather than
silently accepting a different renderer.
These scenarios exclusively claim `/tmp/strata-e2e-baseline`, because the
breadcrumb and context menu render the full path. An existing directory or
symlink is a setup error, never deleted or reused. Within a run, baseline
scenarios stay on one worker; independent concurrent runs must use separate
containers.

A capture matches when no more than 0.5% of pixels differ by more than 24 in
any channel, which absorbs the subpixel antialiasing that software rendering
varies between runs.

Baselines are never accepted automatically. When a change is intended:

```bash
STRATA_E2E_UPDATE_BASELINES=1 ./scripts/e2e.sh -k baseline
```

Then review the new images and commit them with the change, so the difference
is visible in the pull request. On a mismatch the suite writes the expected,
actual, and diff images into the artifact directory.

## Proving the suite would catch a regression

`scripts/e2e-mutation-check.sh` applies one deliberate defect at a time from
`tests/e2e/mutations`, rebuilds, and asserts that the scenarios for that
workflow fail:

```bash
./scripts/e2e-mutation-check.sh              # every mutation
./scripts/e2e-mutation-check.sh clipboard    # one of them
```

Each patch breaks a single critical workflow — drag and drop, clipboard,
keyboard navigation, click modes, view switching, filtered quick preview. The unmodified scenarios
must pass first; only a failed scenario assertion in the mutated run counts as
detection, not a startup/collection error or killed process. Logs and JUnit
reports are saved in `target/e2e-mutations`. The script restores source changes
afterwards. Run it after changing the harness, and when adding a scenario for
a workflow that does not have a mutation yet.

## In CI

### Three-minute critical path

`.github/workflows/ci.yml` has three E2E stages:

1. **E2E build and plan** restores BuildKit layers for the pinned native/Python
   dependencies, Rust toolchain, and compiled `Cargo.lock` dependencies. It
   compiles the tested revision **once**, without debug information or incremental
   artifacts, and collects the real pytest inventory (including parameter IDs).
   It caches a Zstandard-compressed runtime archive by rendering inputs and UID/GID,
   independently of source revisions, and uploads the binary, checksummed provenance,
   and complete shard plan. A separate small plan artifact keeps aggregation small.
2. **E2E shard N** jobs start on independent 4-vCPU runners, restore that exact runtime
   archive, download the attempt's binary bundle, and invoke `./scripts/e2e.sh` with two isolated
   xdist workers. Shards do not start BuildKit, contact a registry, restore Cargo
   caches, compile, or install packages. Every runner uses
   the same pinned rendering packages and baselines as a local canonical run.
3. **End-to-end GUI suite** retains the existing required-check name. It requires
   every dependency to succeed, verifies that all planned node IDs passed setup,
   call, and teardown exactly once. Timing is **informational**, with a three-minute
   performance target—not a pass/fail condition. From build-job creation through
   aggregation, the initial E2E queue, setup,
   dependency installation or cache retrieval, compilation, transfers, downstream
   runner queues, and test execution are included—not just pytest time. Queue and
   execution durations appear in the Actions summary. Slow runs and unavailable
   timing telemetry do not fail passing tests. Final teardown cannot be measured
   from inside its own job;
   use the Actions completion timestamp to verify the final observed runtime.

The matrix is generated from `harness/sharding.py`, not a fixed runner count or
file list. Tests are scheduled longest-first using committed setup+call+teardown
CI measurements from `tests/e2e/durations.json`, with 25% headroom and a 90-second
soft target per worker (two workers per runner). New tests automatically receive a
conservative five-second weight. More tests or longer measured durations add runners
up to a maximum of eight shards, reducing duplicated runtime setup and bounding
fan-out. At the cap, shards run longer rather than failing planning or dropping tests.
Baselines stay in one serial scheduling group, even when that group exceeds the soft
target. Estimated time alone never fails the gate; hang-protection timeouts still apply.
Tune `TARGET_SECONDS` and `MAX_SHARDS` in `tests/e2e/harness/sharding.py` manually as
runtime and cost needs change. The shard cap is not a spending cap: longer runs still
consume more runner-minutes. There is no `max-parallel` throttle; the runner provider
must have enough concurrent capacity for up to eight shards. Runner queues affect
reported timing, not test correctness.

Shards validate their entire collection against the plan before selecting tests.
A missing, extra, skipped, failed, or stale result fails the aggregate gate.
`fail-fast: false` preserves other shards' diagnostics. Worker crashes and failed
assertions are not retried; only display infrastructure startup retains its one
retry. CI caps each test at 60 seconds and each shard job at ten minutes to stop
hung execution, independently of the informational three-minute overall target.
Reports and JUnit files are uploaded on both success and failure, with distinct
artifact names per runner and run attempt; screenshots/trees/logs are uploaded on
failures. The gate never mixes previous attempts into a fresh measurement.

### Understanding a failed gate

The prerequisite check reports whether bootstrap or shard execution failed, links
unsuccessful jobs and steps, and inspects a bounded amount of their logs. Confirmed
Ubuntu Snapshot HTTP errors, compiler diagnostics, and provenance failures are
identified separately. When bootstrap fails, it explicitly says that no scenarios
ran. Timing is reported separately and never changes the test result. If logs
cannot be retrieved or classified, it says so rather than guessing the cause.

### Cache lifecycle and cold starts

BuildKit's content-addressed cache invalidates on the actual Dockerfile, package
installer, requirements, Rust manifest/lockfile, source, and resource inputs.
`install-packages.sh` downloads from the official archive using byte-identical,
signed snapshot indexes, then installs against the original snapshot sources.
It never refreshes indexes from the moving archive: versions and APT checksum
verification stay pinned. For superseded packages missing from the archive,
Launchpad's primary archive is tried using the exact filename and mandatory SHA256
from the signed snapshot metadata. Downloads use APT's sandboxed partial directory;
APT verifies them again when installing against the original snapshot sources.
The snapshot remains the final fallback; failed maintainer scripts are not retried. This avoids making every
package download wait on the slower snapshot service during cold recovery.

The `package-indexes` stage fetches and authenticates the dated APT indexes once.
Runtime and toolchain installation reuse them through read-only build mounts;
neither downloads a second copy, and the lists are removed before each install
layer is committed. The warmer publishes the index-stage cache **before** package
and compiler bootstrap, so a later failure does not discard a successful index
fetch. Source builds attach the public input digest as an output image label, not
an environment variable or secret-looking build argument.

A stub application
warms dependencies only; its executable and all Strata fingerprints are removed
before the real source is copied and compiled. The bundle is tied to the checked-out
commit, source/resource contents (including local edits), rendering inputs, binary
checksum, and plan checksum. The runtime image's input label must match too; it cannot be replaced
by an arbitrary host binary or an artifact from another run.

`Warm E2E dependencies` publishes the full dependency cache in a separate workflow,
with a 30-minute bootstrap allowance: daily, on image/dependency changes on main
and PRs, and on manual dispatch. PRs warm only their own branch-scoped cache.
The timed build restores a BuildKit local-cache directory through the cache action's
segmented transfer path, keyed by image inputs, manifests, ignore rules, and UID/GID.
BuildKit still validates content-addressed dependency records before reusing them.
A missing local cache first looks for published environments, then falls back to
the shared GHA dependency cache; the old
`strata-e2e-v1` cache remains a read-only migration source. The warmer publishes both
formats and replaces its local export directory so obsolete blobs do not accumulate.

The timed build does not publish an application BuildKit cache. It compiles the
tested revision against cached dependencies and immediately hands off the binary
and plan instead of waiting for cache export. Repeated same-revision runs therefore
exercise the same compilation path as new revisions, not a misleading binary-cache
shortcut.

The runtime archive uses a separate exact-key cache; shards restore it without
starting BuildKit. Only the producer exports or saves a missing archive. It confirms
that publication succeeded before telling shards to use the cache. If publication
is unavailable (including read-only fork caches), it instead uploads an explicit
attempt-scoped `e2e-runtime-<attempt>` artifact; transfer time remains in the timing report.
This avoids repeatedly uploading and unzipping a large unchanged runtime on ordinary
source revisions without making cache-write permission a prerequisite for testing.

Node-24-native actions verify artifact download digests; the runner also checks
bundle provenance and the loaded image's input label. GitHub's cache branch scoping
allows fork PRs to read the main cache without credentials or registry access and
prevents PR caches from replacing main's cache. No `pull_request_target` execution
or package-write permission is needed. Main cannot restore a PR-scoped cache:
a successful PR run does not seed main's first bootstrap after merging a new
workflow. Run the trusted default-branch warmer to populate main's own caches;
do not promote untrusted PR build caches into main. These caches are evictable,
not permanent package storage. Published environments provide the durable fallback
below; upstream outages can still block the first publication of new inputs.

### Persistent published environments

`Publish pinned E2E environments` publishes two GHCR packages from trusted main:

- `ghcr.io/lgse/strata-e2e-runtime`: the pinned GUI/Python/font environment.
- `ghcr.io/lgse/strata-e2e-build`: Rust plus the runtime and compiled locked Cargo
  dependencies. The stub application and its fingerprints are removed; no tested
  application binary is published here.

Tags include SHA256 input identifiers, architecture, UID, and GID. Runtime inputs
are independent of application source; the build identifier additionally includes
`Cargo.toml`, `Cargo.lock`, and `.dockerignore`. Publication runs on input changes
or manual dispatch, not ordinary application commits. Existing publications are
checked before rebuilding. A separate `base-<environment-inputs>-linux-amd64-1001-1001`
tag in the build package provides a reusable local environment independent of
application manifests. It is published once per environment input set and is not
retagged on ordinary Cargo changes. GHCR storage is independent of Actions cache eviction;
retain tags needed by supported revisions rather than treating them as disposable
per-run artifacts.

On a fast dependency-cache miss, CI resolves matching image tags to **OCI digests**,
checks their platform and input labels, and uses them directly as application and
runtime bases. Published labels are preserved, not overwritten to match a checkout. Those builds do not
execute Ubuntu/Python/Rust bootstrap stages, even with an empty BuildKit cache.
The application still compiles for the tested revision, and the base's Cargo
manifests must match the checkout. A fresh runner still transfers image layers;
this avoids package-server requests and dependency compilation, not all network I/O.

The build package also stores complete BuildKit caches. An environment-only cache
identifier lets new Cargo inputs reuse unchanged GTK/Rust dependencies. The warmer
can import these persistent caches to refill the faster segmented GitHub caches.
If an image is absent or inaccessible, CI reports that fact and falls back to
verified cache/source building; it never substitutes `latest`, another dependency
version, or an unverified bundle.

Only the main-only publisher has `packages: write` and registry login credentials.
PR CI and warmers pull anonymously and cannot publish shared images. After the
first publication, **make both packages public in GHCR package settings** and rerun
the publisher. Its anonymous-access check fails descriptively until they are
readable without credentials. Do not give fork PRs package-write credentials.

To exercise the registry path, manually dispatch **CI** with
`require_published_environments` enabled. This bypasses the fast dependency cache
and fails if matching public bases are unavailable; it cannot silently benchmark a
source-build fallback. All scenarios and the same informational timing report still apply. The
runtime archive cache remains enabled, so this is not a completely cold transport
benchmark.

New environment inputs still require an initial trusted publication; a PR cannot
seed main by publishing its own build. The three-minute target is informational
for both warm and cold runs.

**New inputs without published images or cached bootstrap are not guaranteed to
install and compile within three minutes.** The timed build retains a ten-minute
limit to stop hung bootstrap jobs; elapsed time alone never fails the aggregate.
Cold and warm durations are reported honestly rather than conflated. Large
cache publication no longer blocks binary handoff or races that ten-minute limit;
the separate warmer owns it. Seed the dependency cache before enabling the new
required gate, and rerun the **entire workflow** after the warmer completes. External package outages, cache eviction, and runner queues cannot
be solved by adding test shards. Inspect the build logs' `CACHED` entries and the
critical-path summary rather than raising the time limit.

### Measured fresh-revision run

[CI run 34235908938](https://github.com/lgse/strata/actions/runs/34235908938)
(`84d8e82`, 2026-09-08) compiled Strata again and passed all 581 cases exactly once
on 30 runners in **172 seconds**, measured from the initial E2E queue/attempt start
through the aggregate job's completed timestamp. The internal measurement was
168.7 seconds before teardown. The runtime archive was cached; the new segmented
dependency cache missed, so this run exercised the GHA dependency-cache fallback
while the separate warmer published the new format. This was not an identical
binary-cache rerun or a completely cold package bootstrap.

Attempt 2 of the same run restored both segmented caches and **recompiled Strata
in 7.51 seconds**. All 581 cases passed exactly once: **139 seconds** from initial
E2E queue to completion, **141 seconds** from attempt start, and 137.4 seconds at
the internal measurement. This is same-revision recompilation, not a second new
revision or a binary-cache shortcut. These measurements predate persistent GHCR
base images; they are not a benchmark of the registry-only cold-runner path.

### Reproducing and maintaining shards

To collect the current inventory inside the canonical container:

```bash
./scripts/e2e.sh -n 0 --collect-only --e2e-write-plan=target/e2e-plan.json
./scripts/e2e.sh --e2e-plan=target/e2e-plan.json --e2e-shard=0 \
  --e2e-report=target/e2e-reports/shard-0.json
```

For the exact CI binary, use a disposable checkout at the commit in its metadata
(a PR normally tests the merge commit). Download `e2e-bundle-<attempt>` into
`target/e2e-bundle`, build the `runtime` target with the invoking UID/GID, and run
(use `docker` instead of `podman` if appropriate):

```bash
podman build --target runtime --tag strata-e2e:ci-runtime \
  --build-arg E2E_UID="$(id -u)" --build-arg E2E_GID="$(id -g)" \
  --label org.strata.e2e.inputs="$(python3 scripts/e2e_bundle.py image-key)" \
  --file tests/e2e/Dockerfile .
chmod +x target/e2e-bundle/strata
STRATA_CONTAINER_ENGINE=podman STRATA_E2E_IMAGE=strata-e2e:ci-runtime \
  STRATA_E2E_BUNDLE=target/e2e-bundle ./scripts/e2e.sh --e2e-plan=target/e2e-bundle/plan.json --e2e-shard=0
```

The bundle must be inside the checkout. Without these explicit bundle/image
variables the local canonical runner reuses its verified base and compiles the
checked-out application normally; it does not rebuild the base.

After a complete CI attempt with every test passing, download its `e2e-report-<attempt>-*` artifacts into an empty
`target/e2e-reports` directory and its `e2e-plan-<attempt>` into
`target/e2e-bundle`, then refresh timings:

```bash
python3 scripts/e2e_ci.py durations target/e2e-bundle/plan.json target/e2e-reports \
  > tests/e2e/durations.json
```

Review and commit the changes. Reports must cover every test and shard; partial or
failed runs cannot overwrite scheduling measurements. Durations are scheduling
hints, never an allowlist: new tests always participate without editing this file.

### Maintaining behavioral coverage

Before consolidating tests, identify the retained behavioral owner for every
assertion. Fewer test functions or collected cases do not establish a runtime
improvement. Preserve functional layout, input-routing, filesystem-safety,
live-preference and lifecycle regressions, even when they share fixtures.
Record task-specific inventories and consolidation decisions in the issue or PR,
not in a committed audit report.

### Coverage audit (#607)

154 GUI cases were removed from the 729-test inventory (six new harness tests
exercise collection/sharding/reporting). No workflow was moved into an optional,
nightly-only, or changed-files-only suite.

| Removed/reduced coverage | Retained owner |
| --- | --- |
| Six invalid strings × mode × kind × new/existing × completion, reduced to `bad/name` (120 cases removed) | `src/services/operations/tests.rs::basenames_reject_empty_reserved_nested_absolute_and_nul_names` (no GTK/display requirement); GUI retains every mode/kind/lifecycle and both Enter and real click-away |
| Four invalid names in correction/reopen workflow, reduced to one (9) | Same validation tests; correction/reopen still runs in all three views |
| Four accepted-name variants in inside-field click workflow, reduced to ` padded ` (18) | `basenames_accept_single_native_and_unicode_components`; GUI still checks exact untrimmed names for files/folders in every view |
| Standalone new-folder Escape and existing-file cancellation tests (4) | `test_inline_renaming.py::test_escape_preserves_the_original_name`, covering both lifecycles, kinds, and every view |
| Standalone invalid rename in dialogs suite (1) | Stronger synchronized `test_invalid_names_retain_the_original`, including filesystem contents and GTK-critical checks |
| Separate preview metadata/list-preservation launches (2) | Assertions consolidated into `test_space_opens_and_closes_the_quick_preview` in all three views |

All 72 valid rename focus-exit combinations remain: GTK's real in-flight focus walk
is not covered by emitting a controller signal in Rust. Real drag/XTEST routing,
caret visibility, clipboard selection/undo, multi-window preferences, accessibility
semantics, and all six visual baselines also remain. The removed validation vectors
are covered by unconditional, display-independent Rust tests—not by tests that
silently return when GTK cannot initialize.

The existing quality job is unchanged. Enabling all its GTK fixtures on Ubuntu
revealed a pre-existing offscreen new-entry failure tracked in #613; this CI overhaul
does not delete that fixture, change application scrolling, or hide it behind a new
skip. Continue running Rust GTK tests on a private display locally as described above.

The existing copy-conflict/undo scenario also waits for dialog dismissal and the
copied entry's keyboard focus before sending Ctrl+Z. A selected-state notification
alone was too early for this follow-up keyboard action; the filesystem/undo assertions
remain unchanged. There are no assertion retries or fixed sleeps.
