# Vertical slice: Columns on Rune with QA-harness parity

The slice is done when one command runs the same QA scenario files against the GTK binary
and a new Rune binary, and both pass.

- **New binary:** `strata-rune --qa <directory>`, speaking Strata's existing QA protocol
  unchanged.
- **Behaviour covered:** the Columns view on a real directory, driven by Strata's own
  `Browser`: keyboard navigation, selection, and inline rename with filesystem checks.
- **Command:** `./scripts/qa-parity.sh slice`, run in the pinned QA container.

Source: the slice plan of 2026-11-09. Status below is as of 2026-11-11, after V5.

## Exit check

The command exits 0 only when all four hold:

1. Every run marked `required` in `tests/qa/parity/slice.toml` passes on both targets.
2. Every other run in the parity set is marked `pending` with a reason code and a pointer
   to its blocker. No run is unlisted.
3. Each Rune mutation patch makes its named scenario fail on the Rune target.
4. The full GTK suite and its 22 mutation patches still behave as before, which proves the
   harness refactor left the oracle intact.

## Gates

Each gate is a set of existing scenario files, taken as they are. Only Columns runs count.

| Gate | Scenario folders | Files with Columns runs | Columns runs | What it proves |
| --- | --- | --- | --- | --- |
| G1 First light | `arrow-keys-move-focus/columns-arrows` | 1 | 1 | Launch, protocol, real listing, target lookup, key input, snapshot |
| G2 Read path | `keyboard_navigation`, `selection`, `escape_selection`, `background_selection`, `hidden_files`, `startup_arguments` | 35 of 39 | 42 | Navigation, selection and pointer handling match the GTK app |
| G3 Write path | `inline_renaming` | 15 of 16 | 33 | A file operation goes through Strata's services and the fs checks agree |

The counts are read from the scenario headers at `65eb2da03`. Two of G3's Columns runs are
ignored by the runner today; the parity ledger takes its list from the runner, not from
this table.

## Work packages

| WP | Title | Depends on | Status | Done when |
| --- | --- | --- | --- | --- |
| V0 | Assumption checks | None | Done, `f1970421a`. [Findings](V0-findings.md) | Findings written; no stop condition hit |
| V1 | Workspace and toolchain | V0 | Done: the three commits ending at `2a6054141` | Strata builds as a workspace; GTK suite passes |
| V2 | Extract the QA protocol crate | V1 | Done, `60cc86aed` | Runner and binary parse through one crate; GTK suite and mutations pass |
| V3 | [QA engine behind a `Backend` trait](V3-qa-backend-trait.md) | V2 | Done, `60f661490` to `5c2cc9b47` | GTK is one backend; replies unchanged; GTK suite and mutations pass |
| V4 | [Extract the shared crates](V4-shared-crates.md) | V1 | Done, `7f389c536` to `ffb6490eb` | No `gtk4` under the shared crates; GTK suite passes |
| V4b | [Import the component crates](V4b-import-components.md) | V4 | Done: `ad88c18a8` to `7f7a11c26` | The Rune side builds and tests in the workspace; GTK suite passes |
| V5 | [`strata-rune` shell and loop](V5-shell-and-loop.md) | V4b | Done: `651ea8bfb` to `679fb74c9` | A themed window opens on a directory; GLib work reaches the UI thread |
| V5b | [The settings store without GTK](V5b-settings-store.md) | V4 | Done, `ce3b5b83d` | Both interfaces read `settings.toml` through one schema; GTK suite passes |
| V6 | [Columns adapter](V6-columns-adapter.md) | V5 | Not started | Columns shows a real directory with Strata's accessible names |
| V7 | [Rune QA backend](V7-rune-qa-backend.md) | V3, V6 | Not started | Gate G1 passes on both targets |
| V8 | [Read-path parity](V8-read-path-parity.md) | V7 | Not started | Gate G2: ledger complete, required runs pass three times running |
| V9 | [Rename parity](V9-rename-parity.md) | V8 | Not started | Gate G3: same bar, with fs checks |
| V10 | [Slice report](V10-slice-report.md) | V9 | Not started | Report committed; the exit command passes |

V3 can run beside V4b, V5 and V6. V4b is not in the slice plan: it is the import stage of
the [workspace migration](../workspace-migration.md), which the slice needs before V5. V5b
is the part of V5 that moved the settings store, written up on its own because it changes
the GTK application's code.

**What was validated for the done packages.** At `5c2cc9b47`, on 2026-11-10: the full GTK
QA suite three times (787 tests, 758 scenario runs, 2 ignored, in 80 s), `./scripts/quality.sh`
in all three phases, and `./scripts/qa-mutation.sh` with 21 of the 22 patches. All pass,
apart from one flaky run in one of the three QA runs. [V3](V3-qa-backend-trait.md) has the
figures and names the artifacts.

**After the import.** At `7f7a11c26`, on 2026-11-11: `./scripts/quality.sh` in all three
phases (3,948 tests, as before) and the full GTK QA suite once (787 tests, 758 scenario
runs, 2 ignored), both in the pinned image and both of the GTK lane. On the host,
`mise run test-rune`: 2,286 Rune-side tests pass and 249 are ignored, which is the old
repository's result at `e4f2df1a` less the `media-audio` shim. The mutation patches were
not rerun. [V4b](V4b-import-components.md) has the figures.

**After V5.** At `a8f13eef9`, on 2026-11-11, in the pinned image: `./scripts/quality.sh` in
all three phases (3,948 tests, as before) and the full GTK QA suite once (787 tests, 758
scenario runs, 2 ignored). On the host: `mise run test-rune` with 2,298 Rune-side tests
passing and 252 ignored, and the compositor tests `wake::` and `window::` in a private
headless sway. The mutation patches were not rerun. [V5](V5-shell-and-loop.md) and
[V5b](V5b-settings-store.md) have the figures.

Three things are open and were so before V3: `cut.patch` no longer applies,
`new-folder-with-selection-groups/columns` fails more often than not when run alone, and
`distant-archive-scrolled-into-view/icons` failed once under load.

## Scope

The slice is one view, one read path and one write path, end to end through Strata's real
application layer. Width comes later; this slice buys the harness and the seams.

| | Items |
| --- | --- |
| In | The `strata-rune` binary, with a windowed mode and a `--qa` mode. Columns view fed by `Browser` and the local file source. The default key map, selection and pointer intent, marquee. Inline rename through Strata's rename operation and validation. The hidden-files toggle. The directory argument at launch. The `settings.toml` keys the parity set's `prefs` headers name |
| Stubbed | The header shell from strata-components' X1 with inert buttons. Default theme only. No sidebar, tabs or location bar unless a required run cannot pass without one, and then only its static form |
| Out | Icons and List views. Drag and drop, clipboard, preview, search, filter, context menus, dialogs other than the rename error, settings UI, portal chooser, 10xer mode. Pixel predicates and visual baselines |

A run in the parity set that needs something marked Out goes to `pending`. Nobody widens
scope to turn a pending run green.

## Parity strategy

Parity means one runner and one scenario corpus, pointed at two binaries. The scenario files
are the specification; neither target gets its own copy.

- **Select the target, not the tests.** `STRATA_QA_TARGET=gtk|rune` picks the binary the
  runner starts. Fixtures, HOME seeding, `fs` steps and peers stay as they are.
- **The protocol is frozen.** Steps, targets, predicates, reply keys and the snapshot's app
  keys keep their names and meanings. Role names stay AT-SPI's; the Rune backend maps
  AccessKit roles onto them.
- **Accessible names are a porting requirement.** The Rune interface sets the names and
  descriptions `src/ui/accessibility.rs` sets.
- **One engine, two backends.** Parsing and predicate evaluation live in toolkit-neutral
  code shared by both binaries, so the meaning of `selected` or `visible` cannot drift. A
  `Backend` trait supplies the tree, input, settling and window operations.
- **The Rune host is offscreen and runs in real time.** The `--qa` process owns a Rune
  `Runtime`, steps `frame(dt)` with wall-clock time, injects input at Rune's input layer
  and reads `accessibility_json`. It needs no display server.
- **Every run is accounted for.** `tests/qa/parity/slice.toml` lists each run of each file
  in the set as `required` or `pending`. A test fails if a run is missing from it.
- **The Rune harness must be able to fail.** Each gate area gets one mutation patch against
  the Rune tree, checked the way `scripts/qa-mutation.sh` checks GTK's.

Pending runs carry one of six reason codes, which become the slice report's main table:

| Code | Meaning | Points to |
| --- | --- | --- |
| `harness` | A step, target or predicate the Rune backend does not implement yet | The step name |
| `component` | strata-components lacks or differs in the behaviour | Its work package id |
| `rune` | Blocked on Rune itself | The draft id in strata-components |
| `platform` | Needs a real window, compositor, clipboard or drag | The session host, below |
| `toolkit` | The scenario checks something GTK-specific, such as a pixel or a GDK detail | A proposed rewording, as its own GTK-side change |
| `deviation` | The port behaves differently on purpose | An owner ruling |

**What offscreen parity does not prove.** Input never crosses winit or a compositor, so
focus handling, real drag and drop, the clipboard and window decorations are untested here.
Those need a second host: a real window in headless sway, driven by the virtual keyboard and
pointer in `src/qa/input/wayland.rs`. This plan names that host and does not build it.

Known differences belong in the backend, never in a scenario:

- **Key names.** Scenarios use GDK names (`Return`, `Page_Down`, `period`). The Rune
  backend maps them to Rune keys.
- **Layouts.** The `layout` step swaps X keymaps today. The Rune backend emulates a layout
  by sending the physical key with its base character. Non-US runs may start as `harness`
  pending.
- **Reply timing.** GTK replies once the main context drains. Rune replies once the GLib
  context has nothing pending and the runtime has settled. V0 found `Runtime::frame`
  reports "animating", so the backend settles with `Runtime::settle_all`.
- **Criticals.** GLib warnings and criticals still fail a run, since GIO stays. A panic
  keeps exit code 70.

## Architecture

One runner and one shared QA engine drive both binaries. Each binary supplies only a
backend and a view layer.

| Crate | Holds | Must not depend on |
| --- | --- | --- |
| `strata` (root package) | GTK views in `src/ui`, the GTK QA backend in `src/qa` | Rune |
| `crates/strata-qa` | Protocol parsers, the target and predicate engine, the `Backend` trait | Any toolkit, GLib |
| `crates/strata-core`, `strata-preview`, `strata-fs`, `strata-desktop` | Model, services, adapters, `Browser`, the saved settings | `gtk4`, `gdk4`, `sourceview5`, `gstreamer` |
| `crates/strata-platform`, `strata-kit`, `strata-components`, `strata-gallery` | The imported Rune side: platform shims, the look, the components and their gallery | The shared crates, GTK, and the `strata` package |
| `crates/strata-app` | The Rune interface: shell, loop, Columns adapter, and the Rune QA backend behind a `qa` feature | `gtk4` and the `strata` package |
| `crates/strata-rune` | The `strata-rune` binary | Everything but `strata-app`, and the compositor harness its window tests use |

The slice plan put everything Rune-side in one `strata-rune` crate and everything shared in
one `strata-core`. The crates above are the workspace proposal's, which the owner chose.

Three seams carry the design:

- **Loop.** `Browser` is an `Rc` with thread-local GLib sources, so the UI thread must
  iterate its GLib context. Offscreen, the host alternates `MainContext::iteration(false)`
  with `Runtime::frame(dt)`; V0 measured that. In a window, a helper thread waits for what
  the context waits for and sends its dispatch to Rune's loop through
  `AppHandle::update_if`. [V5](V5-shell-and-loop.md) measured why it is not the `Waker`.
- **Events in, intents out.** The adapter subscribes with `Browser::observe` and maps each
  `BrowserEvent` onto strata-components state. Those components take state by accessor and
  own no I/O, so the adapter is the only code that knows both sides.
- **Names.** The adapter sets each row's and pane's accessible name and description. The
  harness finds nothing without them.

## Rules

Strata's `AGENTS.md`, `CONTRIBUTING.md` and `docs/testing.md` come first. Where this plan
conflicts with them, follow them and report the conflict.

- **Never edit the oracle to pass.** Do not change a scenario file, a visual baseline or a
  GTK mutation patch so that either target passes. A scenario that needs rewording is a
  separate GTK-side change with its own reason.
- **Container runs only.** QA evidence comes from the pinned image through the repository's
  scripts. A host run needs the owner's consent and is never evidence.
- **One QA script at a time.** `qa.sh`, `qa-drive.sh`, `qa-mutation.sh` and `qa-parity.sh`
  share `target/qa-container`.
- **One package, one reviewable unit.** The slice plan asked for a branch and a pull
  request per package. Until the slice is approved, the owner has work land as focused
  commits on `strata-next`. A refactor commit contains no behaviour change, and says so.
- **Upstream Rune is read-only.** A gap in Rune becomes a ledger entry and a written
  request. Before V4b the same holds for strata-components; after it, a component change is
  a change on `strata-next` with its own commit.
- **One Rune revision.** The workspace pins one. Move every Rune crate together.
- **No GTK in the Rune binary.** Every change that touches `strata-app` or `strata-rune`
  shows `cargo tree -p strata-rune -i gtk4` finding nothing.
- **Two lanes, never `--workspace`.** The GTK application and the Rune side build in
  separate cargo runs, selected through `scripts/lanes.py` (`docs/workspace.md`). A new
  Rune-side crate is added to its list.
- **No new CI jobs or image changes without approval.** Run parity locally. Propose CI and
  image changes as owner items.
- **Cite evidence.** Each "done" names the command, the container run and the artifact
  directory.
- **Stop rather than widen.** When a stop condition is met, or a package needs something
  out of scope, write up what was found and wait.

## Stop conditions

Work halts and is reported when any of these holds.

| Condition | Status |
| --- | --- |
| V0 host probe fails: an offscreen `Runtime` cannot host the app or give the tree | Not hit |
| V0 core probe finds widget types under `Browser` or rename | Not hit |
| V0 loop probe shows a stall | Not hit, offscreen or in a window. V5 measured a covered Wayland window: a folder change reaches the interface in 102 ms shown and 102 ms covered |
| V3 changes a reply: the refactored GTK harness cannot reproduce the earlier reply logs | Not hit. Every verdict matches; `app` differs between two runs of the baseline itself, so V7's ledger compares verdicts |
| G2 is mostly blocked: more than a third of its runs land as `component` or `rune` pending | Open |

## Owner decisions

| # | Decision | Needed by | Status |
| --- | --- | --- | --- |
| 1 | Where `strata-rune` lives | V5 | Settled: on `strata-next`, in the workspace |
| 2 | Raise the toolchain to the one Rune needs | V1 | Done: Rust 1.99.1 |
| 3 | A software Vulkan driver in the pinned image | V7 | Not needed for the offscreen host |
| 4 | Which strata-components revision | V4b | Settled: `integration` at `e4f2df1a`. The old repository is frozen there |
| 5 | GTK's accessible names and AT-SPI role names as the contract | V6 | Open on two points: row descriptions and the pane group |
| 6 | Non-US layout runs: required or pending | V8 | Open. The plan assumes `harness` pending |
| 7 | Hand check of the windowed binary on Hyprland | V10 | Open. Do it once V6 lands |
| 8 | The windowed wake: a tick, or a GLib embedding shim | V5 | Settled on 2026-11-11: the embedding, notified through `AppHandle::update_if`. A `Waker` call never runs `poll` under a covered Wayland window at Rune `2a44156` |
| 9 | `gstreamer` in `strata-platform` against the `AGENTS.md` rule | V4b | Settled: the rule stands, and the `media-audio` shim was not imported |
| 10 | Import history: squash or keep | V4b | Settled: squashed |
| 11 | Who owns cursor and selection: `Browser` or the component | V6 | Open. New from V0 |
| 12 | The test image's dependency stage, which no longer loads the workspace | Before the publisher is needed again | Open. New from V4b; an image change |
| 13 | A `rune` job in CI, with a Vulkan adapter for the goldens | V7 | Open. New from V4b. Until then the Rune side is checked locally |
| 14 | `[profile.dev.package."*"] debug = false` for the workspace | Any time | Open. New from V4b |
| 15 | A home for the `media-audio` shim | The component plan's phase 5 | Open. New from V4b |
| 16 | Move the settings store, against the `AGENTS.md` rule on `src/ui` | V5 | Settled on 2026-11-11: moved. [V5b](V5b-settings-store.md) |

## Risks

- **Passing parity gets read as more than it is.** The slice proves application behaviour
  through Rune's input layer, and says nothing about a real desktop.
- **The refactor could weaken the oracle.** V3 touches the code that decides pass and fail.
  The mutation suite is the guard.
- **Geometry differs between toolkits.** Row heights and fonts change what `visible`,
  `moved` and `at` offsets see. Expect some `toolkit` pending runs, and resist tuning the
  look to match GTK pixel for pixel.
- **Two halves of one loop.** The windowed host depends on `AppHandle::update_if` running
  under a covered window, which Rune does not promise. The offscreen harness cannot see a
  regression there; the compositor tests `wake::` and `window::` can, and they are local.
- **Two models of selection.** `Browser` and the Columns component each keep a cursor and a
  selection. V0 saw them disagree at launch.
