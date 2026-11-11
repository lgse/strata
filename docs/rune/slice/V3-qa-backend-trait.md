# V3 QA engine behind a `Backend` trait

| | |
| --- | --- |
| Status | Not started. The reply baseline is captured |
| Depends on | V2 (done) |
| Unblocks | V7 |
| Owns | `crates/strata-qa`, `src/qa`, `src/ui/accessibility_qa*` |
| Lands as | Several refactor commits on `strata-next`, one per step below |

## Goal

Make the GTK code in `src/qa` one implementation of a `Backend` trait defined in
`crates/strata-qa`, with every toolkit-neutral decision moved into that crate. No reply to
any scenario may change.

## Why

V2 gave the runner and the application one parser. They still do not share one meaning of
`selected`, `visible` or "the entry named X": that logic is GTK code today. A second
backend written beside it would drift. After V3 the Rune backend (V7) implements only what
is specific to a toolkit: reading its tree, sending input, and knowing when it is idle.

## What exists

`crates/strata-qa` (3,359 lines) holds the protocol only: `Command`, `Target`, `Predicate`
and `Check`, their parsers, `Reply`, `Failure` and `changed_keys`. It depends on `regex`,
`serde` and `serde_json`.

`src/qa` (9,479 lines with tests) is the in-process half. Every module but `pixels.rs` and
three test files imports GTK, GDK or GLib.

| Module | Lines | Holds | Toolkit-neutral part |
| --- | --- | --- | --- |
| `qa.rs` | 419 | `Session` (stdin reader thread, stdout kept for replies), `Controller`, the serve loop, reply assembly, `settle`, timing constants | The serve loop, reply assembly, the constants, stream handling |
| `dispatch.rs` | 778 | `Controller::execute`: one match over about 40 commands, with the `wait`, `expect`, `during`, `keys_until` and `note` loops inline | All the loops and the bookkeeping for `remember` and `keep` |
| `check.rs` | 783 | `Controller::check`: about 50 live predicates | Most arms, once they read nodes instead of widgets |
| `state.rs` | 898 | `Observation`, `AppState`, `FieldState`, `Observation::evaluate`, geometry tests; and `scan`, `dump`, the row table | Everything above the semicolon: about 480 lines of plain data and logic |
| `driver.rs` | 277 | `Input`: `find` (locate, settle, locate again), `aim`, `activate`, `resolve`, `focused_widget` | The retry and deadline logic |
| `targets.rs`, `targets/*` | 1,307 | `Rect`, `Located`, `LookupError`, `Aim`, `Memory`, `Boxes`; `locate`; `Wanted`, the role, name and state matcher; `role_name` (the AT-SPI names); `accessible_name` and `accessible_description`; `States`; fields | `Rect`, `Aim`, `LookupError`, `Boxes`, `Wanted`'s matching rules, target-to-node resolution |
| `pointer.rs`, `keyboard.rs` | 636 | Click, double click, drag, wheel, `select` with its retry, chords, typing, `select_with_keyboard` | The sequences and their gaps; the X keycode work is not |
| `onscreen.rs`, `reveal.rs` | 451 | Scrolling a clipped target into view, scrollbar values | `travel`, `wheel_clicks`, the attempt loop |
| `waits.rs` | 171 | `adopt`, `open_menu`, `settle_target`, `wheel_until` | The loops |
| `note.rs` | 130 | `Shape` and the `note` accessors | `Shape` |
| `observe.rs` | 480 | Number expressions, visible rows, scroll values, footer fill, hit paths, theme accent, child processes, saved preferences | `number_json`, expression arithmetic, `child_processes`, the file reads |
| `samples.rs`, `pixels.rs` | 297 | Pixel captures and text measurement | `pixels.rs` whole |
| `clipboard.rs`, `criticals.rs` | 117 | A private display's clipboard; GLib log capture | None; `criticals` needs GLib, not GTK |
| `input.rs`, `input/wayland.rs` | 949 | `Injector`: XTEST or Wayland virtual input, keysyms, the input log | None. This is the GTK backend's input |
| `tests/*` | 1,786 | Unit tests | `predicates.rs`, `snapshot.rs`, `pixels.rs` and `commands.rs` use no toolkit |

The harness does not read AT-SPI. Runs set `GTK_A11Y=none`, and names come from a registry
in `src/ui/accessibility_qa`, where `src/ui/accessibility.rs` records each widget's label,
description and a `Mark`: `Entry`, `Pane(view)`, `EntryContainer`, `PaneSurface` or
`Dialog`. `state::scan` finds panes and entries by mark, not by role.

## The trait

A sketch to start from, not a settled signature. It follows the slice plan's list: the
tree, input, settling, draining, window resize and an optional pixel capture.

```rust
/// One toolkit's side of the QA mode. `Node` is a live element: a `gtk::Widget`, or a
/// Rune accessibility node.
pub trait Backend {
    type Node: Clone + PartialEq;
    /// A handle that does not keep its node alive: `remember` holds these.
    type Kept: Clone;

    // The tree.
    fn roots(&self) -> Vec<Self::Node>;                    // the main window first
    fn parent(&self, node: &Self::Node) -> Option<Self::Node>;
    fn children(&self, node: &Self::Node) -> Vec<Self::Node>;
    fn role(&self, node: &Self::Node) -> &'static str;     // AT-SPI names
    fn name(&self, node: &Self::Node) -> String;
    fn description(&self, node: &Self::Node) -> String;
    fn mark(&self, node: &Self::Node) -> Option<Mark>;
    fn states(&self, node: &Self::Node) -> States;
    fn bounds(&self, node: &Self::Node) -> Option<Rect>;   // rendered, in screen points
    fn text(&self, node: &Self::Node) -> Option<String>;
    fn field(&self, node: &Self::Node) -> Option<FieldState>;
    fn focus(&self) -> Option<Self::Node>;
    fn keep(&self, node: &Self::Node) -> Self::Kept;
    fn recall(&self, kept: &Self::Kept) -> Option<Self::Node>;

    // Input.
    async fn chord(&self, chord: &Chord) -> Result<(), String>;
    async fn key(&self, name: &str, pressed: bool) -> Result<(), String>;
    async fn text_input(&self, character: char) -> Result<(), String>;
    async fn pointer_move(&self, point: (f64, f64)) -> Result<(), String>;
    async fn button(&self, number: u8, pressed: bool) -> Result<(), String>;
    async fn wheel(&self, clicks: u32, down: bool) -> Result<(), String>;
    async fn activate(&self, node: &Self::Node) -> Result<(), String>;

    // Time and the window.
    async fn settle(&self);                 // one painted frame, as `settle` is today
    async fn sleep(&self, time: Duration);  // lets the application run meanwhile
    async fn resize(&self, width: u16, height: Option<u16>) -> Result<(), String>;
    fn criticals(&self) -> Vec<String>;
    fn dump(&self) -> String;               // the `tree` query

    /// Everything else: a command or a predicate only this toolkit can answer.
    async fn command(&self, command: &Command) -> Option<Outcome> { None }
    fn check(&self, predicate: &Predicate) -> Option<Result<Result<(), Failure>, String>> {
        None
    }
    fn capture(&self, rect: [i64; 4]) -> Option<Result<Capture, String>> { None }
}
```

Points the design has to settle:

- **Async without `dyn`.** The engine's loops wait, and GTK's waiting is a GLib future. The
  engine is generic over `B: Backend` and the trait uses `async fn`, which needs an
  `#[expect(async_fn_in_trait, reason = …)]` on a public trait. The GTK backend runs the
  engine on the GLib main context, as `Controller::serve` runs today. The Rune backend
  drives it with a small executor whose sleep steps frames.
- **Semantic marks stay.** `Mark` moves to `strata-qa`. The engine finds panes, entry
  containers and entries by mark, as `scan` does, so the Rune backend only has to classify
  its nodes.
- **GTK quirks stay in the GTK backend.** A recycled row whose label still shows the old
  name, the row widget against its `ListItem`, focus on an unmapped row, and `GtkMenuButton`
  describing its inner toggle are all answered inside `name`, `states` and `children`.
- **An unsupported command says so.** When `command` and `check` return `None` for
  something the engine does not handle itself, the reply's `error` is
  `"<step> is not supported on this target"`. The GTK backend supports everything it
  supports today.
- **`strata-qa` stays free of GLib.** Its dependency rule in `deny.toml` does not change.

## Scope

Each step is one commit, a pure refactor, with the GTK suite green before the next.

1. **Baseline.** Done: see Tests.
2. **Move the plain snapshot code.** `Observation`, `PaneObservation`, `EntryObservation`,
   `AppState`, `PaneState`, `FieldState` as data, `field_matches`, the geometry tests
   (`within_viewport`, `within_window`, `inside`, `on_screen`, `contains`,
   `middle_visible`, `collapse_whitespace`), `Observation::evaluate`, `empty_app_state`,
   `Rect`, `Boxes`, `Aim`, `LookupError`, `Outcome`, `Capture` and `pixels.rs`, `Shape`,
   `travel`, `wheel_clicks`, `step_towards`, `coordinate` and `number_json` go to
   `strata-qa`. Their unit tests go with them.
3. **Define `Backend`, `Mark` and `States` in `strata-qa`,** and implement the tree half
   for GTK over `gtk::Widget`, delegating to the functions in `targets/naming.rs`,
   `targets/states.rs` and `state.rs`.
4. **Move snapshot assembly.** `scan` becomes engine code over `Backend`: panes by mark,
   left to right; entries by mark inside the entry container, top to bottom then left to
   right; the focused entry and pane; dialog, menu, preview, peek, search results and the
   focused field.
5. **Move target lookup.** `Wanted`'s matching, `locate`, `presented_target`, `field`,
   `find` with its settle-and-compare, `aim` and `resolve` become engine code. `Memory`
   holds `B::Kept`.
6. **Move the predicates.** Each arm of `Controller::check` that reads only names, roles,
   states, bounds, text and kept boxes moves. What stays GTK's goes through
   `Backend::check`: `Pixel`, `AccentPixels`, `Repainted`, `ContrastBelow`, `HitPath`,
   `FooterFill`, `ScrollValue`, `ListSections`, `SomeWindow` and `Windows`.
7. **Move the command loops.** `wait`, `expect`, `during`, `holds`, `keys_until`, `note`,
   `remember`, `keep`, `settle`, `query`, `open_menu`, `wheel_until`, `select` and
   `select_with_keyboard`, plus the pointer and keyboard sequences with their gaps, become
   engine code over the input half of the trait. `clipboard`, `capture`, `measure_text`,
   `drop_samples`, `set_scrollbar`, `adopt` and the reveal steps stay behind
   `Backend::command`.
8. **Move the serve loop.** Reading commands, the ready reply, reply assembly with
   `changed_keys`, the panic reply and exit code 70, and the stream handling of
   `Session::prepare` become engine code a backend calls. The usage text keeps its
   `usage: strata --qa` prefix, which the release workflow greps for.
9. **Compare replies** against the baseline, and record the result.

If a step cannot be made reply-identical, it stops there and is reported: that is the slice
plan's stop condition for V3.

## Out of scope

- Any Rune code, and any change to the runner in `tests/qa.rs`.
- Any scenario, baseline image or mutation patch.
- Moving `input.rs` or the Wayland input client. The slice plan names them for a later
  session host; they stay where they are.
- New steps, targets or predicates.

## Tests

The harness decides pass and fail for everything else, so this package escalates to full
runs, as `AGENTS.md` asks for harness code. Run them from the verify worktree, one script at
a time.

| Check | Command | Expect |
| --- | --- | --- |
| Engine unit tests | `cargo test -p strata-qa --all-features --locked` (no toolkit, so no display) | Pass, and more tests than before step 2 |
| GTK backend unit tests | `./scripts/test-headless.py qa::` | Pass |
| The oracle | `STRATA_QA_KEEP_ARTIFACTS=1 ./scripts/qa.sh` | 758 runs pass, as at the baseline |
| Its teeth | `./scripts/qa-mutation.sh` | All 22 patches still fail their scenarios |
| Replies | A diff of every run's `transcript.jsonl` against the baseline | No difference but timing |
| Lint, before the push | `./scripts/quality.sh fmt`, `./scripts/quality.sh clippy` | Clean |
| Layering | `cargo deny check` | `strata-qa` still has no toolkit and no GLib |
| Code health | `scripts/code-health.sh --changed` | No file scores lower |

**The baseline.** `STRATA_QA_KEEP_ARTIFACTS=1 ./scripts/qa.sh` at `65eb2da03` wrote one
folder per run with `transcript.jsonl`, `tree.txt`, `state.json`, `input.log` and
`result.json`: 787 tests passed, 2 ignored, in 80 s. A copy is kept outside the tree, in
the verify worktree's `target/qa-baseline-65eb2da03/`. Capture it again if `strata-next`
has moved before step 2 starts.

**Comparing replies.** A transcript line is `{"command":…,"reply":…}`. Before comparing:

- replace each run's fixture and home names, which are random
  (`strata-qa-fixture-XXXXXX`);
- drop values that are measured time: an `elapsed` result, "held for N of M polls", and
  timeout texts;
- keep everything else, including `ok`, `app`, `changed`, `failures`, `error` and the
  order of lines.

The comparison script is a throwaway and stays out of the tree. The slice plan asks for the
G1 to G3 runs to match; compare all 758 and report any run outside those gates that
differs.

## Done when

- `src/qa` implements `Backend` and holds no predicate, target or loop logic that does not
  name a GTK type.
- The GTK suite and all 22 mutation patches behave as before.
- The reply logs of the G1 to G3 Columns runs match the baseline apart from timing.
- The commit messages say no behaviour changed, and the handoff names the runs and the
  artifact directories.

## Risks

- **A subtle change in timing.** Several sequences depend on when `settle` runs between
  events, such as the 100 ms button hold and the 90 ms double-click gap. Moving a loop must
  keep every await where it is.
- **Generic code hides a GTK assumption.** `scan` returns no entries when two names share
  one box, because a recycled `GtkListView` row can show that for a frame. Rules like this
  must stay with GTK, or the engine would hide a real defect on Rune.
- **Size.** About 4,000 lines move or change shape. Steps 5 to 7 are the bulk; split them
  further along module lines if a commit grows past what can be reviewed.
- **`main` merges.** Fixes to `src/qa` on `main` will conflict with every step. Merge
  `main` just before starting, and again before step 5.
