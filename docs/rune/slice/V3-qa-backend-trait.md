# V3 QA engine behind a `Backend` trait

| | |
| --- | --- |
| Status | Done: eight commits, `60f661490` to `5c2cc9b47`, and this record |
| Depends on | V2 (done) |
| Unblocks | V7 |
| Owns | `crates/strata-qa`, `src/qa`, `src/ui/accessibility_qa*` |
| Landed as | One refactor commit for each step below on `strata-next`, and one tidy commit |

## Goal

Make the GTK code in `src/qa` one implementation of a `Backend` trait defined in
`crates/strata-qa`, with every toolkit-neutral decision moved into that crate. No reply to
any scenario may change.

## Why

V2 gave the runner and the application one parser. They still did not share one meaning of
`selected`, `visible` or "the entry named X": that logic was GTK code. A second backend
written beside it would have drifted. Now the Rune backend (V7) implements only what is
specific to a toolkit: reading its tree, sending input, and knowing when it is idle.

## What it became

| | Before | After |
| --- | --- | --- |
| `crates/strata-qa` | 3,359 lines: the protocol only | 11,282 lines: the protocol, the engine in `src/engine/`, and 51 unit tests |
| `src/qa` | 9,479 lines with tests | 3,543 lines, 938 of them the X and Wayland input in `input.rs` |
| `strata-qa`'s dependencies | `regex`, `serde`, `serde_json` | those and `toml`, `futures-channel`, `futures-lite`, `rustix`. No toolkit and no GLib |

`strata-qa` holds every decision: which node a target means, what the snapshot lists and in
what order, what each predicate holds, how each step waits and retries, the pointer
sequences with their gaps, the session loop and the reply.

`src/qa` holds what names a GTK type:

| Module | Holds |
| --- | --- |
| `qa.rs` | The launch arguments and the usage text, `settle` and the frame-clock wait, the start of the session on the GLib main context |
| `backend.rs` | `Gtk`, the `Backend` implementation, and the wait for the first entry list |
| `state.rs` | The row table that ties a `GtkListItem` to its row widget, the rules for a recycled row, field reading, the `tree` dump |
| `targets/` | AT-SPI role names, accessible names and descriptions, states, text content, unmapped extents |
| `keyboard.rs` | X keycodes for a chord, the Shift a keysym needs, the borrowed keycode for a character the layout lacks |
| `check.rs`, `observe.rs` | The seven predicates only GTK answers |
| `dispatch.rs`, `reveal.rs`, `samples.rs`, `clipboard.rs` | The eight steps only GTK runs |
| `input.rs`, `criticals.rs` | Unchanged, apart from the release time the engine now keeps |

## The trait

`crates/strata-qa/src/backend.rs` is the reference. The engine is generic over
`B: Backend`, with `async fn` in the trait and no `dyn`; GTK runs it on the GLib main
context. The sketch this package started from had 29 methods. The trait has 50, because
keeping GTK's replies exact needed questions the sketch did not ask.

| Part | Methods | Note |
| --- | --- | --- |
| Session | `ready`, `quit`, `windowless`, `criticals` | `windowless` is the portal host: it takes steps before it has a window, and `app` is then the empty state |
| Windows | `window`, `toplevels`, `title`, `window_width`, `focus`, `adopt` | These replace the sketch's `roots`. `toplevel`, `other_window`, `some_window`, `windows` and `adopt` need titles |
| Tree | `parent`, `children`, `visible`, `attached` | `children` returns hidden nodes too. The engine leaves out a hidden subtree itself, so "presented" means one thing |
| A node | `role`, `name`, `description`, `mark`, `states`, `orientation`, `text`, `field`, `value` | `orientation` is not part of `States`: see below |
| Geometry | `frame`, `origin`, `layout`, `scroll`, `scroll_position` | These replace the sketch's `bounds`. `frame` is the cheap box inside a surface that the snapshot asks of every node; `origin` is the one X round trip that puts it on the screen; `layout` is the box of a node that is not drawn |
| Entries | `entry`, `entries` | Defaulted over `mark`. GTK overrides both |
| The application | `labels`, `settings_file` | The localized names of the preview, the peek and the search results, and where the preferences are saved |
| Memory | `keep`, `recall` | `Memory` holds `Backend::Kept` |
| Time | `settle`, `sleep`, `begin` | `settle` takes the window whose frame it waits for |
| Keys | `chord`, `text_input`, `hold`, `release`, `key` | The first four keep their own gaps: GTK's depend on X keycodes and a release the server times. `key` sends one event |
| Pointer | `pointer_move`, `pointer_position`, `button`, `activate`, `resize` | Each sends one event and the engine pauses. A wheel click is a press and release of button 4 or 5, so there is no `wheel` |
| The rest | `dump`, `command`, `check` | A step or predicate only the toolkit can answer. `None` gives "<step> is not supported on this target" |

What the sketch had and the trait does not: `capture`. Nothing in the engine reads pixels,
so V7 adds it when a step needs it.

How the points the design had to settle came out:

- **Semantic marks stay.** `Mark` lives in `strata-qa`. The engine finds panes and entry
  containers by mark. An entry's mark is recorded on its `GtkListItem`, which is no widget,
  so GTK answers `entry` and `entries` from its row table; a backend whose rows are nodes
  marks them and uses the defaults.
- **GTK quirks stay in the GTK backend.** A recycled row whose label still shows the old
  name is left out of `entries`, and two names sharing one box list nothing. Focus on an
  unmapped row is no focus, in `focus`. `GtkMenuButton` naming its inner toggle is in `name`.
- **An unsupported step or predicate says so,** and a test over an in-memory backend checks
  the text.
- **`strata-qa` stays free of GLib.** `deny.toml` is unchanged and `cargo deny check` passes.

## Steps

| Step | Commit | What moved |
| --- | --- | --- |
| 1. Baseline | none | Captured at `65eb2da03` |
| 2. Plain snapshot code | `60f661490` | The snapshot types and `Observation::evaluate`, geometry, `Outcome`, `Capture` and the pixel measurements, `Shape`, the scroll arithmetic, and the tests that use no toolkit |
| 3. `Backend`, `Mark`, `States` | `bc9f343de` | The trait's tree half and its GTK implementation |
| 4. Snapshot assembly | `2403f2dc9` | `scan` |
| 5. Target lookup | `7a688e26d` | `Wanted`, `locate`, `presented_target`, the field lookups, `find`, `resolve`, and `Scope` |
| 6. Predicates | `9ae9682ea` | `check`, and the reads predicates share |
| 7. Step loops | `d68264458` | `execute`, every loop, the pointer sequences, `aim` |
| 8. Serve loop | `a90d5c082` | The session, the reply, the panic reply, the stream handling |
| Tidy | `5c2cc9b47` | `reveal.rs` and `samples.rs` kept their code-health scores |
| 9. Compare replies | this file | See Results |

Where the steps departed from the plan, and why:

- **`aim` moved in step 7, not 5.** Scrolling a clipped target into view turns the wheel,
  which needs the input half of the trait.
- **More predicates are engine code than planned.** `hit_path`, `scroll_value`,
  `list_sections`, `some_window` and `windows` read only what the trait exposes. GTK keeps
  `pixel`, `accent_pixels`, `repainted`, `contrast_below`, `footer_fill`, and two the plan
  did not list: `elapsed`, which reads an instant only `drop_samples` writes, and
  `diagnostics_mention`, which reads the recorded descriptions and not the accessible ones.
- **`adopt` is engine code.** It runs before a window exists, so the engine waits for the
  window and the backend only takes it. The other steps the plan left with GTK stay behind
  `Backend::command`: `reveal`, `reveal_control`, `set_scrollbar`, `capture`,
  `measure_text`, `drop_samples`, `clipboard_set` and `clipboard`.
- **`input.rs` lost the time of the last button release.** The engine keeps it, because
  the pointer sequences read it. Nothing else in `input.rs` changed.

## Results

Every run named here is in the verify worktree, `~/dev/strata-next-verify/target/`.

| Check | Command | Result |
| --- | --- | --- |
| Engine unit tests | `cargo test -p strata-qa --all-features --locked` | 51 pass. The crate had none before step 2 |
| GTK backend unit tests | `./scripts/test-headless.py qa::` | 5 pass |
| The oracle | `STRATA_QA_KEEP_ARTIFACTS=1 ./scripts/qa.sh` at `5c2cc9b47`, three times | 787 passed twice. Once 786: `new-folder-with-selection-groups/columns`, which is flaky. `qa-final-{1,2,3}.log`, `qa-final-{1,2,3}-artifacts/` |
| Its teeth | `./scripts/qa-mutation.sh` with 21 of the 22 patches | 21 of 21 caught, in 12.2 minutes. `qa-mutation-final-summary.md`. The 22nd, `cut.patch`, does not apply |
| Lint | `./scripts/quality.sh fmt`, `./scripts/quality.sh clippy` | Both clean in the pinned image |
| Rust tests | `./scripts/quality.sh test` | 3,948 passed, 771 ignored by libtest. `quality-final-test.log` |
| Layering | `cargo deny check` | advisories, bans, licenses and sources ok |
| Code health | CodeScene, each file against `65eb2da03` | No file scores lower. See below |

Each step's commit also had a full `qa.sh` run before the next began: 787 passed for steps
2, 3, 6 and 7, and 786 for steps 4, 5 and 8, each time with one of the two flaky runs
below failing.

**`cut.patch` does not apply,** at `65eb2da03` either: `src/ui/browser/clipboard.rs` moved
under it on `main` (#2092). It is out of this package's scope, so the mutation run leaves
it out and "all 22 patches" is not shown. It needs a rebased patch.

**Two runs are flaky, and were before V3.**

- `new-folder-with-selection-groups/columns` waits for the opened folder's column to take
  focus, and focus stays on the row. Run alone it failed 5 of 6 times at `65eb2da03`, 5 of
  8 at step 3 and 3 of 6 at step 4. In a full run it usually passes.
- `distant-archive-scrolled-into-view/icons` failed once in a full run, at step 8: the new
  archive was never selected. Its input log is byte for byte the baseline's, and it passed
  8 of 8 alone at the same commit.

### Replies

The comparison reads every run's `transcript.jsonl`. It masks the random fixture and home
names and a fixture server's port, and drops measured time: an `elapsed` actual, "held for
N of M polls", "held for N ms" and timeout texts.

Two runs of one binary do not write the same transcripts. A reply reads `app` at the
moment it is written, and the application keeps working after a step returns: a pane is
listed before or after its entries arrive, a preview before or after its details. So the
comparison is in two parts.

- **The verdict of every line**: the command, `ok`, `error`, `failures`, `result` and
  `criticals`. These do not depend on when the reply was written.
- **`app` and `changed`**, held against several baseline runs: a line passes when some
  baseline run wrote the same reply at that line.

Three full runs at `65eb2da03` and three at `5c2cc9b47`, 758 scenario runs each:

| A run held against | Runs whose verdict differs | Runs with a reply no reference run wrote | Gate runs among them |
| --- | --- | --- | --- |
| Baseline against the other two baseline runs | 0, 0, 0 | 34, 36, 38 | 1, 4, 2 |
| Final against two baseline runs | 0, 0, 1 | 32 to 41 | 2 to 5 |

The one verdict that differs is the flaky `new-folder-with-selection-groups/columns`. No
verdict differs in the 73 Columns runs of gates G1 to G3 in any final run.

Whole transcripts differ in 54 to 59 runs between two baseline runs, in 52 to 64 between a
baseline and a final run, and in 58 to 59 between two final runs. The refactored harness
differs from the baseline as much as the baseline differs from itself, and only in when
`app` was read. The stop condition "V3 changes a reply" is not hit.

Artifacts: `qa-baseline-65eb2da03/`, `qa-baseline-{2,3}-artifacts/` and
`qa-final-{1,2,3}-artifacts/`, with each step's own run in `qa-step{2..8}-artifacts/`. The
two scripts that made the comparison, and the one that ran the final checks, are kept
beside them in `v3-tools/`, outside the tree.

### Code health

| File at `65eb2da03` | Score | Where the code is now | Score |
| --- | --- | --- | --- |
| `src/qa/check.rs` | 5.91 | `engine/check.rs`; `src/qa/check.rs` | 6.39; 9.4 |
| `src/qa/dispatch.rs` | 5.59 | `engine/dispatch.rs`; `src/qa/dispatch.rs` | 6.21; 9.22 |
| `src/qa/state.rs` | 7.13 | `snapshot.rs`, `engine/scan.rs`; `src/qa/state.rs` | 9, 10; 9.68 |
| `src/qa/observe.rs` | 7.04 | `engine/observe.rs`; `src/qa/observe.rs` | 7.04; 10 |
| `src/qa/targets/locate.rs` | 7.94 | `engine/locate.rs` | 8.43 |
| `src/qa/pointer.rs` | 8.67 | `engine/pointer.rs` | 8.81 |
| `src/qa/driver.rs` | 8.85 | `engine/scope.rs`, `engine/activate.rs` | 9.38, 9.42 |
| `src/qa/waits.rs` | 8.81 | `engine/waits.rs` | 8.81 |
| `src/qa/keyboard.rs` | 9.15 | `engine/keyboard.rs`; `src/qa/keyboard.rs` | 9.59; 9.53 |
| `src/qa/reveal.rs` | 8.38 | `src/qa/reveal.rs` | 8.39 |
| `src/qa/samples.rs` | 8.64 | `src/qa/samples.rs` | 9.38 |
| `src/qa.rs` | 10 | `engine/serve.rs`; `src/qa.rs`, `src/qa/backend.rs` | 10; 10, 10 |

`engine/check.rs` and `engine/dispatch.rs` are still one long match each. They were moved
arm for arm so the replies could be compared; splitting them is a change of its own.

## What V3 found

- **`has_state` reads a widget's orientation for every state it tests,** and GTK logs
  `gtk_orientable_get_orientation: assertion 'GTK_IS_ORIENTABLE (orientable)' failed` for
  some widget when asked. Reading it for every widget, as a first version of step 3 did,
  failed 13 runs. The engine keeps the old reads exactly, so no reply changes; which widget
  it is, and whether a step that tests a state on it should fail, is open.
- **The transcripts are not reproducible line for line,** as described under Replies. A
  parity ledger that compares GTK with Rune has to compare verdicts, not `app`.
- **`cut.patch` is stale** and the two runs above are flaky.

## Out of scope

- Any Rune code, and any change to the runner in `tests/qa.rs`.
- Any scenario, baseline image or mutation patch.
- Moving `input.rs` or the Wayland input client. The slice plan names them for a later
  session host; they stay where they are.
- New steps, targets or predicates.

## Done when

- `src/qa` implements `Backend` and holds no predicate, target or loop logic that does not
  name a GTK type. Done.
- The GTK suite and all 22 mutation patches behave as before. The suite does; 21 patches
  do, and `cut.patch` did not apply before either.
- The reply logs of the G1 to G3 Columns runs match the baseline apart from timing. See
  Replies.
- The commit messages say no behaviour changed, and the handoff names the runs and the
  artifact directories. Done.
