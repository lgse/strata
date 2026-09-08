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

Docker is required by default. For rootless Podman, set
`STRATA_CONTAINER_ENGINE=podman`. The script builds and runs the same image
locally and in CI, using `tests/e2e/Dockerfile`: a digest-pinned Ubuntu 24.04
base, a dated Ubuntu package snapshot (GTK 4.14 and fonts), Rust 1.98.1, and
pinned Python dependencies. The Docker build context is the repository root;
`.dockerignore` excludes host artifacts and configuration. The local runner
builds the `toolchain` stage; CI shards need only the `runtime` stage. No host GTK libraries, fonts, desktop sockets, or
Rust binaries are mounted into the container.

The checkout is mounted at `/workspace`; pass test paths relative to the
repository. Build and Cargo caches live in `target/e2e-container`, separately
from native builds. Artifacts remain in `target/e2e-artifacts` and are owned by
the invoking user. The image adds an account for the invoking UID/GID because
D-Bus requires an account entry; this does not change the rendering packages.
Updating the image inputs is an intentional rendering
environment change and requires reviewing the visual baselines.

For explicit host-toolkit debugging only, `./scripts/e2e-native.sh` accepts
`STRATA_BINARY` and `STRATA_E2E_VENV`. It is not the pre-push E2E gate; a native
pass does not replace `./scripts/e2e.sh`.

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
complete Rust test suite on the same private Xvfb/D-Bus infrastructure with:

```bash
./scripts/test-headless.py
./scripts/test-headless.py -- --nocapture
```

This requires Xvfb and AT-SPI but not the Python E2E packages. It isolates
application preferences while retaining access to the installed Cargo/Rust
toolchains, and disables accessibility bridging for Rust tests. A startup failure
aborts; it never falls back to the real display.
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
in each view, a selection with focus, an open context menu, and a confirmation
dialog. Local and CI runs use this one rendering profile. Other host GTK
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
   Only the binary, checksummed provenance, and complete shard plan are uploaded.
2. **E2E shard N** jobs start on independent 4-vCPU runners, restore only the
   runtime layers (no Rust toolchain), download that run's bundle, and invoke
   `./scripts/e2e.sh` with two isolated xdist workers. There is no host build,
   Cargo cache restore, or pip installation on the warm path. Every runner uses
   the same pinned rendering packages and baselines as a local canonical run.
3. **End-to-end GUI suite** retains the existing required-check name. It requires
   every dependency to succeed, verifies that all planned node IDs passed setup,
   call, and teardown exactly once, and enforces **less than 180 seconds** from
   the build job's start through aggregation. Setup, dependency installation or
   cache retrieval, compilation, transfers, downstream runner queues, and test
   execution are included—not just pytest time. Job durations and total elapsed
   time appear in the Actions summary; five seconds are reserved for final
   teardown rather than spending the entire budget before the job can finish.
   The initial queue before any E2E runner
   starts and final GitHub job teardown are not measurable from inside this gate;
   use the Actions job timestamps when evaluating the final observed runtime.

The matrix is generated from `harness/sharding.py`, not a fixed runner count or
file list. Tests are scheduled longest-first using committed setup+call+teardown
measurements from `tests/e2e/durations.json`, with 25% headroom and a 40-second
estimated worker budget. New tests automatically receive a conservative five-second
weight. More tests or longer measured durations add runners. Baselines stay in
one serial scheduling group. An indivisible group over budget or a plan requiring
more than GitHub's 256 matrix jobs fails explicitly instead of silently extending
the gate. There is no `max-parallel` throttle; the runner provider must have enough
concurrent capacity. A busy runner pool can therefore fail the wall-clock budget.

Shards validate their entire collection against the plan before selecting tests.
A missing, extra, skipped, failed, or stale result fails the aggregate gate.
`fail-fast: false` preserves other shards' diagnostics. Worker crashes and failed
assertions are not retried; only display infrastructure startup retains its one
retry. CI caps each test at 60 seconds and each shard job at three minutes.
Reports and JUnit files are uploaded on both success and failure, with distinct
artifact names per runner and run attempt; screenshots/trees/logs are uploaded on
failures. The gate never mixes previous attempts into a fresh measurement.

### Cache lifecycle and cold starts

BuildKit's content-addressed cache invalidates on the actual Dockerfile,
requirements, Rust manifest/lockfile, source, and resource inputs. A stub application
warms dependencies only; its executable and all Strata fingerprints are removed
before the real source is copied and compiled. The bundle is tied to the checked-out
commit, source/resource contents (including local edits), rendering inputs, binary
checksum, and plan checksum. The runtime image's input label must match too; it cannot be replaced
by an arbitrary host binary or an artifact from another run.

`Warm E2E dependencies` seeds a separate default-branch cache daily, when image or
Rust dependency inputs change, and on manual dispatch. The regular build can read
that cache and the previous application cache. Only the build job writes the
application cache; shards are read-only consumers. GitHub's cache branch scoping
allows fork PRs to read the main cache without credentials or registry access and
prevents PR caches from replacing main's cache. No `pull_request_target` execution
or package-write permission is needed.

**A completely cold/evicted cache or a newly changed image is not guaranteed to
install and compile within three minutes.** The build is allowed ten minutes to
finish and save usable layers, but the aggregate still fails the 180-second budget:
a cold run is not silently exempted or advertised as a fast pass. Seed the dependency
cache before enabling the new required gate, and rerun the **entire workflow** after
cold recovery. External package outages, cache eviction, and runner queues cannot
be solved by adding test shards. Inspect the build logs' `CACHED` entries and the
critical-path summary rather than raising the time limit.

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
  --build-arg E2E_IMAGE_KEY="$(python3 scripts/e2e_bundle.py image-key)" \
  --file tests/e2e/Dockerfile .
chmod +x target/e2e-bundle/strata
STRATA_CONTAINER_ENGINE=podman STRATA_E2E_IMAGE=strata-e2e:ci-runtime \
  STRATA_E2E_BUNDLE=target/e2e-bundle ./scripts/e2e.sh --e2e-plan=target/e2e-bundle/plan.json --e2e-shard=0
```

The bundle must be inside the checkout. Without these explicit bundle/image
variables the local canonical runner still builds the source normally.

After a complete successful CI attempt, download its `e2e-report-<attempt>-*`
artifacts into an empty `target/e2e-reports` directory and its bundle into
`target/e2e-bundle`, then refresh timings:

```bash
python3 scripts/e2e_ci.py durations target/e2e-bundle/plan.json target/e2e-reports \
  > tests/e2e/durations.json
```

Review and commit the changes. Reports must cover every test and shard; partial or
failed runs cannot overwrite scheduling measurements. Durations are scheduling
hints, never an allowlist: new tests always participate without editing this file.

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
