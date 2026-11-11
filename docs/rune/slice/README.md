# Vertical slice: Columns on Rune with QA-harness parity

The slice is done when one command runs the same QA scenario files against the GTK binary
and a new Rune binary, and both pass.

- **New binary:** `strata-rune --qa <directory>`, speaking Strata's existing QA protocol
  unchanged.
- **Behaviour covered:** the Columns view on a real directory, driven by Strata's own
  `Browser`: keyboard navigation, selection, and inline rename with filesystem checks.
- **Command:** `./scripts/qa-parity.sh slice`, run in the pinned QA container.

Source: the slice plan of 2026-11-09. Status below is as of 2026-11-10.

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
| V3 | [QA engine behind a `Backend` trait](V3-qa-backend-trait.md) | V2 | Not started; baseline captured | GTK is one backend; replies unchanged; GTK suite and mutations pass |
| V4 | [Extract the shared crates](V4-shared-crates.md) | V1 | Done, `7f389c536` to `ffb6490eb` | No `gtk4` under the shared crates; GTK suite passes |
| V4b | [Import the component crates](V4b-import-components.md) | V4 | Not started | The Rune side builds and tests in the workspace; GTK suite passes |
| V5 | [`strata-rune` shell and loop](V5-shell-and-loop.md) | V4b | Not started | A themed window opens on a directory; GLib work reaches the UI thread |
| V6 | [Columns adapter](V6-columns-adapter.md) | V5 | Not started | Columns shows a real directory with Strata's accessible names |
| V7 | [Rune QA backend](V7-rune-qa-backend.md) | V3, V6 | Not started | Gate G1 passes on both targets |
| V8 | [Read-path parity](V8-read-path-parity.md) | V7 | Not started | Gate G2: ledger complete, required runs pass three times running |
| V9 | [Rename parity](V9-rename-parity.md) | V8 | Not started | Gate G3: same bar, with fs checks |
| V10 | [Slice report](V10-slice-report.md) | V9 | Not started | Report committed; the exit command passes |

V3 can run beside V4b, V5 and V6. V4b is not in the slice plan: it is the import stage of
the [workspace migration](../workspace-migration.md), which the slice needs before V5.

**What was validated for the done packages.** The full GTK QA suite passed at `65eb2da03`
on 2026-11-10: 787 tests, 758 scenario runs, 2 ignored, in 80 s. `./scripts/quality.sh` and
`./scripts/qa-mutation.sh` were not rerun at that commit in the session that wrote these
files. `strata-next` is ahead of `lgse/strata` and has not been pushed.

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
| `crates/strata-core`, `strata-preview`, `strata-fs`, `strata-desktop` | Model, services, adapters, `Browser` | `gtk4`, `gdk4`, `sourceview5`, `gstreamer` |
| `crates/strata-app` | The Rune interface: shell, loop, Columns adapter, and the Rune QA backend behind a `qa` feature | `gtk4` and the `strata` package |
| `crates/strata-rune` | The `strata-rune` binary | Everything but `strata-app` |

The slice plan put everything Rune-side in one `strata-rune` crate and everything shared in
one `strata-core`. The crates above are the workspace proposal's, which the owner chose.

Three seams carry the design:

- **Loop.** `Browser` is an `Rc` with thread-local GLib sources, so the UI thread must
  iterate its GLib context. Offscreen, the host alternates `MainContext::iteration(false)`
  with `Runtime::frame(dt)`; V0 measured that. In a window, something must wake Rune when
  GLib has work. This is the least certain part of the slice.
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
| V0 loop probe shows a stall | Not hit offscreen. The windowed wake is untested; V5 settles it |
| V3 changes a reply: the refactored GTK harness cannot reproduce the earlier reply logs | Open |
| G2 is mostly blocked: more than a third of its runs land as `component` or `rune` pending | Open |

## Owner decisions

| # | Decision | Needed by | Status |
| --- | --- | --- | --- |
| 1 | Where `strata-rune` lives | V5 | Settled: on `strata-next`, in the workspace |
| 2 | Raise the toolchain to the one Rune needs | V1 | Done: Rust 1.99.1 |
| 3 | A software Vulkan driver in the pinned image | V7 | Not needed for the offscreen host |
| 4 | Which strata-components revision | V4b | Open. V0 used `integration` at `e4f2df1a` |
| 5 | GTK's accessible names and AT-SPI role names as the contract | V6 | Open on two points: row descriptions and the pane group |
| 6 | Non-US layout runs: required or pending | V8 | Open. The plan assumes `harness` pending |
| 7 | Hand check of the windowed binary on Hyprland | V10 | Open. Do it once V6 lands |
| 8 | The windowed wake: a tick, or a GLib embedding shim | V5 | Open. New from V0 |
| 9 | `gstreamer` in `strata-platform` against the `AGENTS.md` rule | V4b | Open. New from V0 |
| 10 | Import history: squash or keep | V4b | Open. From the workspace proposal |
| 11 | Who owns cursor and selection: `Browser` or the component | V6 | Open. New from V0 |

## Risks

- **Passing parity gets read as more than it is.** The slice proves application behaviour
  through Rune's input layer, and says nothing about a real desktop.
- **The refactor could weaken the oracle.** V3 touches the code that decides pass and fail.
  The mutation suite is the guard.
- **Geometry differs between toolkits.** Row heights and fonts change what `visible`,
  `moved` and `at` offsets see. Expect some `toolkit` pending runs, and resist tuning the
  look to match GTK pixel for pixel.
- **Two halves of one loop.** If Rune cannot be woken reliably, the windowed app stalls
  even though the offscreen harness passes.
- **Two models of selection.** `Browser` and the Columns component each keep a cursor and a
  selection. V0 saw them disagree at launch.
