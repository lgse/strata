# Vertical slice: Columns on Rune with QA-harness parity

The slice is done when one command runs the same QA scenario files against the GTK binary
and a new Rune binary, and both pass.

- **New binary:** `strata-rune --qa <directory>`, speaking Strata's existing QA protocol
  unchanged.
- **Behaviour covered:** the Columns view on a real directory, driven by Strata's own
  `Browser`: keyboard navigation, selection, and inline rename with filesystem checks.
- **Command:** `./scripts/qa-parity.sh slice`, run in the pinned QA container.

Source: the slice plan of 2026-11-09. Status below is as of 2026-11-11, after V9.

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
this table. G2's 42 runs include G1's one: the ledger lists 75 runs, one under G1, 41 under
G2 and 33 under G3.

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
| V6 | [Columns adapter](V6-columns-adapter.md) | V5 | Done: `90fdcb2f7` to `25d40091f`. The owner's look at the window moved to V10 | Columns shows a real directory with Strata's accessible names |
| V7 | [Rune QA backend](V7-rune-qa-backend.md) | V3, V6 | Done: `4de69a313` to `e97b84e0b`. The image's dependency stage was not changed (decision 12) | Gate G1 passes on both targets |
| V8 | [Read-path parity](V8-read-path-parity.md) | V7 | Done: `dc1f7ed9c` to its record. Gate G2 passes with 23 of 42 runs `required`. One part stopped: the Appearance popover is not mounted | Gate G2: ledger complete, required runs pass three times running |
| V9 | [Rename parity](V9-rename-parity.md) | V8 | Done: `3ba9a1f312ae2b9163c9dde892af88b866bf4bbe` to its record. Gate G3 passes with 13 of 33 runs `required`; 17 are `scope` pending, 2 `platform`, 1 `rune` | Gate G3: same bar, with fs checks |
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

**After V6.** At `24a04ec0b`, on 2026-11-11, on the host: `mise run test-rune` with
2,312 Rune-side tests passing and 253 ignored, Clippy on the Rune lane with
warnings denied, and the compositor tests `window::` in a private headless sway, where the
windowed binary's Columns view lists its folder and a file created in it, and a key sent
through the compositor opens the folder under the cursor. Nothing was run
in the pinned image: V6 changed no shared crate, no file under `src/` and no QA scenario, so
the GTK lane builds from the same sources as after V5. The goldens of the `c2` and `c22`
gallery pages changed on purpose. [V6](V6-columns-adapter.md) has the figures.

**After V7.** At `e97b84e0b`, on 2026-11-11, in the pinned image: `./scripts/qa-parity.sh
g1` passes on both targets, which is gate G1; `./scripts/quality.sh` in all three phases
(3,953 tests: the earlier 3,948, one in `strata-qa` and four in the runner); the full GTK QA
suite three times (793 tests, 758 scenario runs, 2 ignored), whole twice and once with the
flaky `new-folder-with-selection-groups/columns` failing, which also failed in one of two
runs at `b3b8f448e`; and `./scripts/qa-mutation.sh` with 21 of the 22 patches, all caught.
On the host: `mise run test-rune` with 2,337 Rune-side tests passing and 253 ignored, and
the compositor tests `window::`. `./scripts/qa-parity.sh slice` exits 1 for that one flaky
GTK run, twice out of two. The test image did not change. [V7](V7-rune-qa-backend.md) has
the figures and names the artifacts.

**After V8.** At `24fb0b9c6`, on 2026-11-11, in the pinned image: `./scripts/qa-parity.sh
g2` three times running, 22 of 22 required runs on GTK and on Rune each time, which with
gate G1's run is 23 of the 42 runs the plan counts for G2; `./scripts/qa-parity.sh g1`;
and `./scripts/qa-parity.sh mutations`, both Rune patches caught. At `5118ca9b7`, whose
GTK lane and runner are the same: the full GTK QA suite once (793 tests, 758 scenario
runs, 2 ignored), whole but for the flaky `new-folder-with-selection-groups/columns`;
`./scripts/qa-mutation.sh` with 21 of the 22 patches, all caught; and `./scripts/quality.sh
fmt` and `clippy`. On the host: `mise run test-rune` with 2,365 Rune-side tests passing and
253 ignored, at `78ceea9e2`, and the compositor tests `window::`. `./scripts/quality.sh
test` was not run. The other 19 runs of G2 are pending: 18 `scope` and one `platform`.
[V8](V8-read-path-parity.md#done-when-and-the-evidence) has the figures and names the
artifacts.

**After V9.** At `61a7d6224`, on 2026-11-11, in the pinned image: `./scripts/qa-parity.sh
g3` three times running, 13 of 13 required runs on GTK and on Rune each time, with every
`fs` check holding on both; `./scripts/qa-parity.sh g1` and `g2`, 1 and 22 on each target;
`./scripts/qa-parity.sh mutations`, three Rune patches caught, the rename one at an `fs`
check; `./scripts/qa.sh` with the 15 `inline_renaming` scenarios the runner starts, 42
runs; `./scripts/qa-mutation.sh` with that folder's four patches, all caught; and
`./scripts/quality.sh` in all three phases, 3,956 tests: V7's 3,953 and three new ones for
the rename rules that moved to `strata-fs`. On the host: `mise run test-rune` with 2,385
Rune-side tests passing and 253 ignored, and the compositor tests `window::`. The full GTK
suite and its other mutation patches were not run: the runner and the engine did not
change. The other 20 runs of G3 are pending: 17 `scope`, two `platform` and one `rune`.
[V9](V9-rename-parity.md#done-when-and-the-evidence) has the figures and names the
artifacts.

Three things are open and were so before V3: `cut.patch` no longer applies,
`new-folder-with-selection-groups/columns` fails more often than not when run alone, and
`distant-archive-scrolled-into-view/icons` failed once under load.

## Scope

The slice is one view, one read path and one write path, end to end through Strata's real
application layer. Width comes later; this slice buys the harness and the seams.

| | Items |
| --- | --- |
| In | The `strata-rune` binary, with a windowed mode and a `--qa` mode. Columns view fed by `Browser` and the local file source. The default key map, selection and pointer intent, marquee. Inline rename through Strata's rename operation and validation. The hidden-files toggle. The directory argument at launch. The `settings.toml` keys the parity set's `prefs` headers name. Since decision 19 (2026-11-11): the context menus of an entry, of several entries and of a pane, opened by the pointer, with the GTK menus' rows for a local folder. Open, Rename, Select All, Refresh and the hidden-files row act; every other row is shown and inert until its part is in. Of the window's keys, F2 and Ctrl+R rename and F5 reads the columns again |
| Stubbed | The header shell from strata-components' X1 with inert buttons. Default theme only. No sidebar, tabs or location bar unless a required run cannot pass without one, and then only its static form |
| Out | Icons and List views. Drag and drop, clipboard, preview, search, filter, dialogs other than the rename error, settings UI, portal chooser, 10xer mode. Undo, and naming a new folder or file. Pixel predicates and visual baselines. Of menus: the Appearance popover, which decision 19 asked for and V8 stopped on ([why](V8-read-path-parity.md#the-appearance-popover-stopped)); the Menu key; the Trash and Recent menus, custom actions and "Send to…" |

A run in the parity set that needs something marked Out goes to `pending`, coded `scope`.
Nobody widens scope to turn a pending run green; the owner widened it once, to menus.

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

Pending runs carry one of seven reason codes, which become the slice report's main table.
`scope` was added on 2026-11-11 by an owner ruling (decision 19), so that `component` keeps
meaning a component that lacks or differs:

| Code | Meaning | Points to |
| --- | --- | --- |
| `harness` | A step, target or predicate the Rune backend does not implement yet | The step name |
| `component` | strata-components lacks or differs in the behaviour | Its work package id |
| `rune` | Blocked on Rune itself | The draft id in strata-components |
| `platform` | Needs a real window, compositor, clipboard or drag | The session host, below |
| `toolkit` | The scenario checks something GTK-specific, such as a pixel or a GDK detail | A proposed rewording, as its own GTK-side change |
| `deviation` | The port behaves differently on purpose | An owner ruling |
| `scope` | The run needs a part of the interface the slice marks Out | That part's work package id, and the first step that needs it |

**What offscreen parity does not prove.** Input never crosses winit or a compositor, so
focus handling, real drag and drop, the clipboard and window decorations are untested here.
Those need a second host: a real window in headless sway, driven by the virtual keyboard and
pointer in `src/qa/input/wayland.rs`. This plan names that host and does not build it.

Known differences belong in the backend, never in a scenario:

- **Key names.** Scenarios use GDK names (`Return`, `Page_Down`, `period`). The Rune
  backend maps them to Rune keys.
- **Layouts.** The `layout` step swaps X keymaps. The runner also sends a Rune target the
  layout, as its own `keymap` request, and the backend types under it: what the key at a
  US position types there, as a Rune window would hear it. Non-US runs are required
  (decision 6). Rune's public input API carries no physical key, so a binding on a plain
  letter does not match under a non-Latin layout yet ([V7](V7-rune-qa-backend.md#decision-6-a-layout-on-a-target-with-no-display)).
  Dead keys are composed as winit composes them for a Rune window, which delivers the
  Escape that breaks a sequence off, where GTK's input method takes it
  ([V9](V9-rename-parity.md#the-layout-step)).
- **Field offsets.** The protocol counts a text field in characters. Rune's tree counts
  shaped clusters, which the backend turns into characters over the field's text.
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
| G3 has no stop condition. Counted the same way after its first sort | Not hit: 1 of 33, the `rune` run. Two component gaps were found and closed in the components ([V9](V9-rename-parity.md#the-two-component-changes)) |
| G2 is mostly blocked: more than a third of its runs land as `component` or `rune` pending | Hit on 2026-11-11 as first counted, by V8's table of likely codes: 20 of 42, all of them runs that need a part the slice marks Out. V8 stopped (`dc1f7ed9c`). The owner then ruled such runs `scope` and V8 went on: under that coding the count is 0, with one `rune` blocker named behind a `scope` one. [V8](V8-read-path-parity.md#the-stop-condition) |

## Owner decisions

| # | Decision | Needed by | Status |
| --- | --- | --- | --- |
| 1 | Where `strata-rune` lives | V5 | Settled: on `strata-next`, in the workspace |
| 2 | Raise the toolchain to the one Rune needs | V1 | Done: Rust 1.99.1 |
| 3 | A software Vulkan driver in the pinned image | V7 | Not needed for the offscreen host |
| 4 | Which strata-components revision | V4b | Settled: `integration` at `e4f2df1a`. The old repository is frozen there |
| 5 | GTK's accessible names and AT-SPI role names as the contract | V6 | Settled on 2026-11-11: the component changed. A host describes each row and the view names each column's group. [V6](V6-columns-adapter.md#decision-5-row-descriptions-and-the-pane-group) |
| 6 | Non-US layout runs: required or pending | V8 | Settled on 2026-11-11: required, against the plan's assumption. V7 built the mechanism: the runner tells a Rune target the layout and the backend types under it. [V7](V7-rune-qa-backend.md#decision-6-a-layout-on-a-target-with-no-display). G2 has one such run, `arrow-keys-move-focus/columns-hjkl-tenxer-ru`: it is `scope` pending for 10xer mode, with the physical key Rune's input lacks named behind it and [the request](../upstream/rune/97-key-press-with-physical-key.md) filed as [anomalyco/rune#657](https://github.com/anomalyco/rune/issues/657). G3 has one step under `cn-altgr-pinyin`, whose dead keys V9 taught the backend to compose: the step passes, and the run is `rune` pending for the Escape after it ([the request](../upstream/rune/98-escape-after-a-dead-key.md), drafted) |
| 7 | Hand check of the windowed binary on Hyprland | V10 | Open. V6 has landed; the owner moved the check to V10. The compositor tests see the window list its folder, and nothing looks at the picture |
| 8 | The windowed wake: a tick, or a GLib embedding shim | V5 | Settled on 2026-11-11: the embedding, notified through `AppHandle::update_if`. A `Waker` call never runs `poll` under a covered Wayland window at Rune `2a44156` |
| 9 | `gstreamer` in `strata-platform` against the `AGENTS.md` rule | V4b | Settled: the rule stands, and the `media-audio` shim was not imported |
| 10 | Import history: squash or keep | V4b | Settled: squashed |
| 11 | Who owns cursor and selection: `Browser` or the component | V6 | Settled on 2026-11-11 by the probe, as the owner ruled: the view interprets input and `Browser` holds the result. The key expectations of the ten G2 Columns runs V6 could drive in process are all met that way. [V6](V6-columns-adapter.md#decision-11-who-owns-the-cursor-and-the-selection) |
| 12 | The test image's dependency stage, which no longer loads the workspace | Before the publisher is needed again | Open, and wider. The owner ruled on 2026-11-11 to add the Rune side to the stage. V7 found that no run reads the stage, that the Rune lane builds in the pinned base as it is, and that any edit to the Dockerfile changes the base's key and breaks the green build job until the publisher passes. Nothing was changed. [V7](V7-rune-qa-backend.md#the-image-nothing-changed-and-why) has the three ways forward |
| 13 | A `rune` job in CI, with a Vulkan adapter for the goldens | V7 | Open. New from V4b. V7 did without one: parity runs locally through `./scripts/qa-parity.sh`, and the Rune side is still checked locally |
| 14 | `[profile.dev.package."*"] debug = false` for the workspace | Any time | Open. New from V4b |
| 15 | A home for the `media-audio` shim | The component plan's phase 5 | Open. New from V4b |
| 16 | Move the settings store, against the `AGENTS.md` rule on `src/ui` | V5 | Settled on 2026-11-11: moved. [V5b](V5b-settings-store.md) |
| 17 | Six key behaviours where the Columns component differs from the GTK view: Return on a folder whose column is open, an arrow at the list's end, a key that leaves the cursor in place, Ctrl+A's cursor, a key that only changes the active column, and Right when the open child column is another folder's. Change the component, or accept each as a `deviation` | V8 | Settled on 2026-11-11 and done in V8 (C2d): the owner ruled that the component changes so all six behave as the GTK view does, whether or not a scenario checks them. Each was read in the GTK code first. [V8](V8-read-path-parity.md#decision-17-the-six-keys) has what GTK does for each and where the change is; two details of the GTK view are left out and named there. V9 found a seventh, Left in a column with no cursor, and changed it under the same ruling (C2e): for the owner to confirm |
| 18 | Point the usage checks of `release.yml` and `ci.yml` at `strata-rune` | Before a release builds it | Open. New from V7. The binary keeps the `usage: strata --qa` prefix in a `qa` build only; nothing in CI checks that yet |
| 19 | G2's stop condition was hit by scope: 20 of its 42 runs need a part the slice marks Out. Go on, widen or stop; and how such a run is coded | V8 | Settled on 2026-11-11: go on, and widen the slice to menus, the context menu and the Appearance menu; code such a run `scope`. V8 mounted the context menus, which made the two right-click runs `required`. It stopped on the Appearance popover: that is no menu, its controls are other Out parts' and no work package composes it. [V8](V8-read-path-parity.md#the-appearance-popover-stopped) |
| 20 | `leftover-click-honors-modifiers/columns` aims at a row's name label, which Rune's tree does not have | V8 | Settled on 2026-11-11: backend-only first. Found: the backend reads the box of the name the collection keys for its hosts from the drawn elements, with no change to the accessibility tree, and the run is `required`. [V8](V8-read-path-parity.md#the-rows-name-label-decision-20) |
| 21 | The Appearance popover: plan it as a host composition (the view switch, the preview toggle, C20a's sliders, a radio row rune-kit's menu lacks), or leave its five runs `scope` pending until the location bar is in, which they need next anyway | The slice after this one | Open. New from V8 |
| 22 | The Menu key and Shift+F10 open no menu yet: C2's columns install their own keyboard controller, and C21's `MenuKeys` has to wrap it there | With decision 21 | Open. New from V8; V9 did not need it |
| 23 | Undo: seven of G3's runs pass every step before Ctrl+Z and stop there. `Browser` has the undo engine; `strata-app` would bind one key. Wire it in the slice, or leave the seven `scope` pending | V10, or the slice after | Open. New from V9 |
| 24 | Naming a new folder or file: ten of G3's runs start from New File or Ctrl+Shift+N (two of them after the filter bar). It is `Browser::create_new_file` and `name_new_item` behind the editor V9 wired, with `Naming::NewItem` | The slice after this one | Open. New from V9 |
| 25 | A rename the browser fails: the plan has the editor open again with the message, which V9 built. The GTK view shows a dialog. Keep the editor, or wait for the dialog (C15) | The slice after this one | Open. New from V9 |
| 26 | Request 98, the Escape after a dead key, was read from the sources and not run in a window, and has no headless repro. Check it in a window, then file it or drop it | Before it is filed | Open. New from V9 |

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
- **The harness reads the view.** A QA run on the Rune target sees what the Columns view
  shows. It does not see `Browser` holding another cursor or selection, until something
  acts on `Browser`'s: V7 broke what the adapter tells `Browser` and G1 still passed. V6's
  in-process tests hold the two together. G3's runs are the first that can: a rename acts
  on the entry `Browser` holds, and the runner reads the folder. The rename mutation patch
  is caught by an `fs` check.
- **Nearly half of the read-path gate is outside the slice.** 18 of G2's 42 runs are
  `scope` pending after V8. For five it is only a preamble that closes the preview panel;
  for the rest it is what the run is about. In G3 it is 17 of 33: seven stop at undo and
  ten at a new item.
- **The harness presents what Rune's tree leaves out.** Since V8 the backend adds two
  things from the drawn elements: a row's name as its `label`, and the panes behind an
  open menu. Both are boxes and names Rune drew, never guesses, but they are the
  backend's reading and not Rune's accessibility tree: a screen reader hears neither.
- **A field's offsets are a conversion.** Rune's tree places a caret by shaped cluster and
  the protocol by character. The backend converts by Unicode's grapheme clusters, which is
  what a shaper clusters by and is not read from Rune. It was checked against a live field
  on precomposed names; a combining sequence was checked in the conversion alone.
- **Two of G3's required runs are weak on Rune.** `window-accelerators-leave-rename` with
  Ctrl+\ and with Ctrl+K passes because `strata-app` binds neither key yet. The guard they
  are about is held in process with keys it does bind.
- **Inert rows look live.** The context menus show every row the GTK menus show, and most
  do nothing yet. In the windowed binary that is a menu full of dead rows until their
  parts land.
- **Two models of selection.** `Browser` and the Columns component each keep a cursor and a
  selection. V6 keeps them in step, with the view interpreting input; its tests check that
  they agree after every key. The component's key rules are its own port of Strata's, and
  differed from `Browser`'s in six places, which V8 closed (decision 17), and in a seventh,
  which V9 closed. A rename changes an entry's id under a selection the view holds; V9
  moves what is held to the new id when the listing shows it, and found that without it a
  range lost its anchor.
