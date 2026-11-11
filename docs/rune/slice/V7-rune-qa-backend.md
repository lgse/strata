# V7 Rune QA backend

| | |
| --- | --- |
| Status | Done: gate G1 passes on both targets. One part is stopped and with the owner: the test image's dependency stage was not changed ([the image](#the-image-nothing-changed-and-why)) |
| Depends on | V3, V6 |
| Unblocks | V8 |
| Owns | `crates/strata-app/src/qa/`, the target switch in `tests/qa.rs`, `scripts/qa-parity.sh`, `tests/qa/parity/` |
| Commits | `4de69a313` (the GLib log capture, a refactor), `66c5ca5f9` (the `keymap` request), `fddf47f74` (the backend), `e97b84e0b` (the runner, the ledger, the command), then this record |

`strata-rune --qa <directory>` speaks the QA protocol, and
`arrow-keys-move-focus/columns-arrows` passes on both targets from one command,
`./scripts/qa-parity.sh g1`. That is gate G1.

## What landed

### The backend

`crates/strata-app/src/qa/`, behind strata-app's `qa` feature, which `strata-rune` forwards.
No release build has it. `strata-qa`'s rule in `deny.toml` names `strata-app` as its second
user.

| Module | Holds |
| --- | --- |
| `qa.rs` | The `--qa` command line and its usage text, the start of the session, and the loop that runs it on the interface thread |
| `backend.rs` | `Rune`, the `Backend` implementation |
| `stage.rs` | The offscreen host as the session drives it: a turn that lands its motions, settle, sleep, and the tree read once for as long as nothing ran |
| `tree.rs` | One snapshot of `Runtime::accessibility_json`: nodes by key path, the keyboard focus, the marks, and the text dump |
| `roles.rs` | Rune's role names to the AT-SPI names the scenarios use |
| `keys.rs` | GDK key names to Rune keys |
| `keyboard.rs`, `keymap.rs` | The held modifiers, and a layout other than the US one |
| `input_log.rs` | `STRATA_INPUT_LOG` |

How each part of `Backend` is answered:

| Part | On Rune |
| --- | --- |
| `window`, `toplevels`, `title`, `window_width` | The tree's root, alone. The title is the one the application last asked for |
| `parent`, `children`, `visible`, `attached` | The snapshot. Rune leaves hidden elements out of the tree, so every node is visible |
| `role` | The role map. A `group` is a `grouping` only when it has a name, and a `panel` otherwise |
| `name`, `description`, `value`, `text` | The node's own. `text` is a label's name or a field's value |
| `states` | `selected`, `checked`, `sensitive`, `selectable`, `editable` and `focusable` from the node; `focused` for the node `focus` returns; `pressed`, `busy` and `marked` are always false |
| `mark` | From the adapter, not from words: the list whose key is a column's (`ColumnsState::column_key`) is the entry container, the options under it are entries, and the named group around it is the pane. The default `entry` and `entries` do the rest |
| `focus` | The focused element's `activeDescendant` when it names a node under it, else the focused element |
| `frame`, `origin`, `layout` | The node's box in window points; the origin is zero |
| `keep`, `recall` | The key path and the role the node had |
| `labels`, `settings_file` | `Preview`, `Folder peek` and `Search results` through the catalogs, and `settings.toml` in `strata_core::storage::config_directory()` |
| `chord`, `text_input`, `hold`, `release`, `key` | `Runtime::physical_input(Modifiers)`, `Runtime::key`, `Runtime::key_up`, then the modifiers that stay held. A chord settles after it |
| `pointer_move`, `pointer_position`, `button` | `Runtime::pointer_move`; buttons 1 to 3 as `pointer_down`/`up`, `button_down`/`up(Middle)` and `secondary_down`/`up`; 4 and 5 turn the wheel 40 points on the press |
| `activate` | AccessKit's click on the node's key, when the node offers one |
| `ready` | Settles until the launch's folder is listed or has failed to list, for 60 s at most |
| `settle` | Turns the host until neither GLib nor Rune has anything ready, for 250 ms at most. Each turn that did something ends with `Runtime::settle_all` |
| `sleep` | Turns the host, waiting on GLib and Rune's next timer between turns, until the time has passed |
| `resize` | `Runtime::set_viewport` |
| `criticals` | GLib's log writer, shared with the GTK application: `strata_core::logging::criticals` |
| `dump` | A line for each node: Rune's role and key, the AT-SPI role, name, `[selected]`, description, `[focused]` and the box |
| `command` | `keymap` only |
| `scroll`, `scroll_position`, `field`, `orientation`, `check` | `None`. The engine replies that the step or predicate is not supported on this target, or finds no scroll pane to turn. Since [V9](V9-rename-parity.md#field) `field` is answered |

The host draws at 1200x760, the GTK window's size, with the look's `animations` off.

### The process

- **Usage.** `strata-rune --qa` with anything but one directory prints
  `usage: strata --qa <directory>` and exits with 2. A build without the feature does not
  hold the text: `strings` finds it once in a `qa` build and not at all in a default one.
- **Streams, the ready reply, panics.** V3's engine: stdout is the replies', application
  output goes to stderr, the first reply is `{"ready":true}`, and a panic in a step replies
  and exits with 70.
- **The loop.** The session is a future polled on the interface thread. A step turns the
  host itself while it waits, so its future never parks. Between steps the loop turns the
  host and then blocks in GLib; the thread that reads stdin wakes it with
  `MainContext::wakeup`. The application therefore keeps running while the runner works on
  files, as a window's does.
- **`STRATA_INPUT_LOG`.** `key <name> down|up`, `motion <x> <y>` and
  `pointer <x> <y> button <n> down|up`, in window points. The key is the scenario's name
  for it, or `ctrl`, `shift`, `alt`, `altgr`: Rune has no keycodes.

### The runner

- **`STRATA_QA_TARGET=gtk|rune`** picks the binary. `gtk` is the default and changes
  nothing. `rune` starts what `STRATA_QA_RUNE_BINARY` names; a launch fails with the reason
  when that is unset or no file. The `launch` step starts the same target's binary.
- **Artifacts** of a Rune run go to `target/qa-artifacts-rune/<scenario>/<run>`, so one
  command leaves both targets' bundles. A Rune run's `screenshot.png` shows the empty X
  screen: the binary draws on none.
- **Layouts.** After it swaps the display's keymap, the runner sends a Rune target the
  layout as a `keymap` request. See the next section.
- **The ledger's tests** are among the runner's unit tests: four new ones, 33 in all.

### The command and the ledger

`./scripts/qa-parity.sh <slice|g1|g2|g3> [--target gtk|rune] [--pending]` runs in the pinned
image through `scripts/lib/qa-container.sh` and shares `target/qa-container` with `qa.sh`.
It builds `strata-rune` with `--features qa` in a cargo run of its own, then the GTK runner,
and runs the selection's `required` runs with `--exact` on each target. It exits 0 only
when every one passes on both. `scripts/qa_parity.py` reads the ledger for it.

- `--pending` is a survey for V8 and V9: a gate's `pending` runs on the Rune target only,
  with a line for each that passes. It fails only when nothing ran.
- `slice` then runs the runner's own tests and the whole GTK suite. It does not run
  `./scripts/qa-mutation.sh`, and says so.
- Artifacts are kept for passing runs too, unless `STRATA_QA_KEEP_ARTIFACTS=0`.
- A selection with no `required` run says so and exits 0. Gates G2 and G3 are in that state.

`tests/qa/parity/slice.toml` lists 75 runs: gate G1's as `required`, and 41 of G2 and 33 of
G3 as `pending`, `harness`, "not yet attempted". The plan counts 42 Columns runs for G2;
one of them is G1's run, which the ledger lists under G1.

## Decision 6: a layout on a target with no display

The owner ruled on 2026-11-11 that non-US layout runs are required, not `harness` pending.
The runs are G2's and G3's; the mechanism is here.

**How it works.**

1. The runner's `layout` step swaps the X display's keymap, as before, on both targets.
2. For a Rune target the runner then sends `{"keymap":{"layouts":"us,ru","variants":"",
   "group":1,"caps_lock":false}}`: the manifest's entry for the layout, which is what
   `setxkbmap` was given. The reply is recorded in the transcript under the scenario's
   line. A layout the header or a variant names is sent the same way once the application
   is ready.
3. `keymap` is the runner's own request, not a scenario step. It is not in
   `strata_qa::COMMANDS`, `parse_command` refuses it, and so the runner refuses a scenario
   that writes it. Only `parse_request`, which the session reads its lines with, takes it.
   The GTK application answers "keymap is not supported on this target"; the runner never
   sends it one.
4. The backend compiles the layout with xkbcommon, from the same xkeyboard-config data the
   X server compiles. A key name is then read as the GTK harness reads it: by its place on
   a US keyboard, Shift included when the US key needs it. The layout says what that key
   types with the group, Caps Lock, Shift and AltGr in effect.
5. The press is sent as Rune's window host would send it (`window/keys.rs` in Rune): the
   character the layout types as text, a named key as itself, and with Ctrl held a command,
   which names the US key when the layout's character is not ASCII. So Ctrl+V under `ru`
   is `Command('v')`, and `j` under `ru` is the text "о".
6. `{"layout":"us"}` sends the plain US layout, which needs no compiling: the backend goes
   back to its built-in table. A unit test holds that table to what the compiled US layout
   types.

The right Alt key is Alt under a US layout and a level shift under a layout that has one,
as in X.

**What is tested.** In `strata-app`: the layouts of the manifest (`ru`, `fr`, `de`,
`caps-lock`) type what they should from US positions; named keys keep their meaning; the
built-in table agrees with the compiled US layout for every name it lists; a layout that
does not compile leaves the keyboard as it was; `keymap` runs through the engine against
the live host. In `strata-qa`: `keymap` parses, a scenario cannot write it, and a backend
that does not take it says so. In the runner: only a Rune target is told, and every layout
of the manifest makes a request the application's parser takes. Gate G1's scenario has a
`{"layout":"us"}` step, so the container run sends one `keymap` and it passes.

**What it cannot do at this Rune rev.** A Rune window gives a key handler the physical key
and the character the key types unmodified (`Tx::physical_key`, `Tx::key_base`), and
`Chord::matches_tx` falls back on them for a layout whose characters are not ASCII: that is
how a plain `j` binding still matches under `ru`. `Runtime::key` carries neither, and the
entry that does, `Runtime::physical_key_input`, is `pub(crate)` (`runtime/input/physical.rs`
at `2a44156`); only Rune's own script player reaches it. So a binding on a plain letter does
not match under a non-Latin layout through this backend, where it would in a window.
`arrow-keys-move-focus/columns-hjkl-tenxer-ru` needs exactly that, and 10xer mode besides,
which the slice leaves out. When V8 sorts it, the Rune half is a `rune` blocker and needs a
request upstream: a public entry for a key press with its physical code and base.

## The image: nothing changed, and why

The owner ruled on 2026-11-11 to add the Rune side to the test image's dependency stage
rather than fetch on first use. The work stopped on that part, as instructed, because of
what the scripts and the workflows turned out to do. Nothing under `tests/container/`
changed, and no script changed how it obtains the image.

**What was found.**

- **No run reads the dependency stage.** `scripts/image_base.py build` builds the
  Dockerfile's `toolchain` target, not `dependencies`. `qa.sh`, `quality.sh` and now
  `qa-parity.sh` set `CARGO_HOME` and `CARGO_TARGET_DIR` under the checkout's `target/`,
  so Cargo's registry and every compiled dependency live in the worktree, not in the
  image. Crates are fetched there on first use already, the GTK lane's too. The stage's
  `/build/target` is in the image the publisher pushes, and nothing mounts or copies it.
- **So growing the stage would not do what the ruling wants.** For a run to use
  dependencies cached in the image, `qa.sh` and `quality.sh` would have to seed their
  cache from it, which is a change to shared scripts beyond the dependency stage.
- **The Rune side builds in the pinned base as it is.** `cargo build --locked -p strata-rune
  --features qa` succeeds in the image keyed `9229afdf…`, with no package added: 30 s the
  first time in the verification worktree's cache, 2 s after.
- **Any edit to the Dockerfile changes the base's key.** `scripts/image_inputs.py` hashes
  the whole file, so a change to the dependency stage alone gives a new `image_key`, and
  the base every run uses is looked up by that key.
- **What CI does on `strata-next` then.** `Quality build and lint` asks the registry for
  the base of the new key. It is not there until the publisher has pushed it. The job then
  compares `tests/container` between `HEAD^` and `HEAD`: if the pushed head commit is not
  itself the one that changed it, the job fails with "The unchanged environment is
  unavailable"; if it is, the job builds the candidate itself, inside its 20 minutes.
  Every later push fails the same way until the publisher succeeds.
- **The publisher has not succeeded on this branch since the import.** It fails in the
  dependency stage in under a minute: `failed to load manifest for workspace member
  /build/crates/strata-app`, from `strata-test-peer`'s manifest, which `.dockerignore`
  leaves out (run 42991113077). With the stage fixed it would have to build the image
  under a new key, whose registry cache is keyed the same and so empty, and then compile
  both lanes' dependencies, in its 30 minutes on four cores. Nothing here can run it.

**What that means.** Landing the change would turn the currently green build job red at
least until a publisher run nobody has seen pass on this branch succeeds, for no gain in
any local or CI run. Decision 12 is still open, and is now a question with three answers:

1. Leave the image alone. The Rune lane builds in it today. This is the state after V7.
2. Fix the dependency stage so the publisher passes again (copy `strata-test-peer`'s
   manifest, stub the targets `strata-gallery`'s manifest names, build both lanes in
   separate cargo runs). It changes the base's key, so push it as the head commit of its
   own push, watch the publisher, and revert if it fails.
3. Make runs use the image's dependencies. That is 2 plus a change to how `qa.sh` and
   `quality.sh` set up their caches.

The G1 evidence below is from the pinned base with the worktree's cache, which is the
first-use fetch the ruling chose against. The binaries and the tests are the same either
way; where the crates were cached is the only difference.

## Where the work departs from this plan

- **The tree is read from `Runtime::accessibility_json`,** not from the `TreeUpdate`. The
  JSON is what Rune's tests and goldens read, it carries Rune's role names as the plan's
  role table spells them, and its bounds are already in window points.
- **`Kept` is the key path and the role,** not AccessKit's node id, which the JSON does not
  carry. A row's key holds its entry's id, so a row a list gives to another entry is
  another path.
- **Marks come from the adapter's column keys,** as the plan says, with the pane found as
  the named group around a column's list.
- **`settle` gives up after 250 ms,** the GTK harness's own limit for a frame, so a host
  that never comes to rest cannot hang a step.
- **The protocol gained one request,** `keymap`, for decision 6. The plan froze the
  protocol; no scenario step, target, predicate or reply key changed, and no scenario can
  write the request.
- **`strata-core` gained a `qa` feature** for the GLib log capture, which the plan moved to
  "a shared crate". Both applications' `qa` features turn it on.
- **Artifacts for the Rune target have their own folder.** The plan said artifacts stay as
  they are; with one folder a parity run would keep only the second target's bundle.
- **`qa-parity.sh slice` does not run the GTK mutation patches,** the fourth item of the
  exit check. It runs the whole GTK suite and names what it left out. `cut.patch` does not
  apply, so a bare `qa-mutation.sh` cannot pass today.
- **`qa-parity.sh mutations` does not exist yet.** The first Rune mutation patch is V8's.
- **The usage text is `usage: strata --qa <directory>`.** It keeps the prefix, as the plan
  asks, and names `strata` though the binary is `strata-rune`. `release.yml` and `ci.yml`
  were not pointed at this binary: that is a CI change for the owner.
- **The labels are spelled here again.** The plan said both come from the shared crates.
  The paths do; `PREVIEW_LABEL`, `PEEK_LABEL` and `SEARCH_RESULTS_LABEL` are under `src/ui`
  and `src/qa.rs`, which this branch does not move. The backend holds the same three
  English keys, and the slice draws none of the three.

## What V7 found

- **G1 reads the view, not `Browser`.** Breaking what the adapter tells `Browser`
  (`Column::position_of` one position off) leaves G1 passing on Rune: the view moves its own
  cursor, and the echo of a selection the view published is not applied back. The harness
  cannot see a `Browser` that disagrees with the view until something acts on `Browser`'s
  selection, such as a rename. V6's in-process tests assert the two agree after every key,
  and that is the guard until G3. Breaking what the adapter shows (hidden entries listed)
  fails G1 at its first `expect`, as it should.
- **A list's scroll pane is inside the list in Rune's tree, and around it in GTK's.** The
  engine finds a pane's viewport as the `scroll pane` above the entry container. On Rune
  that is the strip's sideways viewport, not the list's own. The `viewport` target and the
  `visible` notes will read the wrong box until the backend presents the list's own scroll
  view there. No gate run of V7 reads it.
- **Rune has no public entry for a key with its physical code,** as decision 6's section
  says.
- **Every keyed layout box is a `group` in Rune's tree.** The role map reports the unnamed
  ones as `panel`, as GTK reports a plain container.
- **A panic outside a step is not answered.** The engine catches a panic in the session's
  future. One in an idle turn of the host, between two steps, unwinds out of `main`: the
  runner sees the process exit, without a last reply and without exit code 70.
- **An empty folder is ready** when its listing ends, though it shows no list
  ([V6](V6-columns-adapter.md#open)).
- **The plan's G2 count includes G1's run.**

## Tests

In `strata-app` (`mise run test-rune qa::`, 23 tests):

| Test | Shows |
| --- | --- |
| `qa::roles::tests` (3), `qa::keys::tests` (7) | The two maps: the roles V0 saw and the ones the parity set's targets name, the named keys and characters of the parity set, Ctrl as a command, the right Alt key |
| `qa::keymap::tests` (6), `qa::keyboard::tests` (4) | Decision 6's mechanism |
| `qa::tests::the_first_light_scenario_passes_step_for_step` | Gate G1's steps through the engine against the canonical tree: `expect`, `select`, `wait`, `keymap`, `key`; the ten `app` keys; the input log |
| `qa::tests::a_wait_for_what_never_comes_fails_and_shows_the_tree` | A wrong focus fails, and the reply carries the tree in its text shape |
| `qa::tests::held_modifiers_reach_the_view_and_what_only_gtk_runs_says_so` | Shift+Down grows a range through the modifiers Rune sees held; `clipboard_set` and `pixel` are not supported on this target |

In `strata-rune` (`tests/qa.rs`, compiled with the `qa` feature):

| Test | Shows |
| --- | --- |
| `the_binary_answers_ready_then_one_reply_for_each_step_and_ends_with_its_input` | The binary over its own stdin and stdout, with no display: the ready reply and its ten keys, a step that arrives while the host sleeps, `select`, an unknown step refused, exit 0 when stdin closes |
| `a_wrong_command_line_prints_the_usage_the_release_check_looks_for` | Exit 2, the usage on stderr, nothing on stdout |

In `strata-qa`: one new test and one extended, 52 in all. In the runner: four new unit
tests and one extended. In `scripts/`: `test_qa_parity.py`, five tests.

## Done when, and the evidence

All on 2026-11-11, at `e97b84e0b`. The pinned-image runs are from the worktree at
`~/dev/strata-next-verify`, in the base keyed `9229afdf…`, with their logs and kept
artifacts in its `target/v7-e97b84e0b/`.

- **`arrow-keys-move-focus/columns-arrows` passes on both targets from one command.**
  `./scripts/qa-parity.sh g1` exits 0: the run passes on GTK in 1.3 s and on Rune in 0.7 s
  (`qa-parity-g1.log`). The bundles are `g1-gtk-artifacts/columns-arrows/` and
  `g1-rune-artifacts/columns-arrows/`, copies of what the run left in
  `target/qa-artifacts/arrow-keys-move-focus/columns-arrows/` and
  `target/qa-artifacts-rune/arrow-keys-move-focus/columns-arrows/`. The two transcripts
  agree in every verdict field of every line, and Rune's has the one `keymap` line more.
- **The harness can fail.** With the adapter listing hidden entries, `qa-parity.sh g1
  --target rune` exits 1 at the scenario's first `expect`, which reads `.hidden.txt` among
  the entries (`g1-broken-listing.log`, `g1-broken-listing-artifacts/`). With the adapter
  telling `Browser` positions one off, G1 still passes (`g1-broken-positions.log`): see
  "What V7 found". Both edits were made by hand in the verification worktree at this
  commit and reverted, and `qa-parity.sh g1` passes again after (`qa-parity-g1-after.log`).
- **The maps and the backend in process.** `mise run test-rune qa::` on the host: 23
  tests pass. `strata-rune`'s two binary tests pass
  (`mise run test-rune the_binary_answers`, `mise run test-rune a_wrong_command_line`).
- **The ledger.** The runner's four new unit tests pass in every run of the runner below:
  no Columns run of the set is unlisted.
- **GTK unchanged.** `STRATA_QA_KEEP_ARTIFACTS=1 ./scripts/qa.sh`: 793 tests, 790 passed,
  1 failed, 2 ignored, in 79 s (`qa-full.log`, `qa-full-artifacts/`). That is the earlier
  789 plus the four runner tests. Of the 758 scenario runs, 757 pass and one fails:
  `new-folder-with-selection-groups/columns`, at its wait for the opened folder's column
  to take focus. It is the run [V3](V3-qa-backend-trait.md) records as flaky, with the same
  failure. Two more full runs at the same commit pass whole: 791 passed, none failed, 2
  ignored, in 66 s each (`qa-full-2.log`, `qa-full-3.log`). Alone, the run passed 4 of 4
  times (`flaky-new-{1..4}.log`). To tell the flake from a regression, the full suite also
  ran twice at `b3b8f448e`, before this package: 787 passed once, and once 786 with the
  same run failing (`baseline-b3b8f448e-full-{1,2}.log`).
- **The engine changed, so the mutation patches ran.** `./scripts/qa-mutation.sh` with the
  21 patches that apply: 21 of 21 caught, in 11.1 minutes (`qa-mutation.log`,
  `qa-mutation/summary.md`). `keyboard-navigation`, G1's own, is caught by
  `arrow-keys-move-focus/columns-arrows`.
- **The Rust suite and lint.** `./scripts/quality.sh fmt` and `clippy` are clean, and
  `./scripts/quality.sh test` verifies 3,953 tests passed exactly once with 771 ignored:
  the earlier 3,948, one more in `strata-qa` and the runner's four.
- **The whole selection.** `./scripts/qa-parity.sh slice` ran twice. Both times G1 passes on
  both targets and the runner's tests pass, and both times the command exits 1 for the one
  flaky GTK run above, in its whole-suite part: 790 passed, 1 failed, 2 ignored
  (`qa-parity-slice.log`, `qa-parity-slice-2.log`). The selection inherits that flake until
  it is fixed. `./scripts/qa-parity.sh g2` says the ledger has no required run for it and
  exits 0.
- **No GTK.** `cargo tree -p strata-rune -i gtk4`, with and without `--features qa`:
  "package ID specification `gtk4` did not match any packages". The same for `gdk4` and
  `sourceview5`. `cargo deny --workspace check` passes all four sections.
- **Not in a release build.** `strata-rune` has no default feature. `strings` finds
  `usage: strata --qa` once in a build with `--features qa` and not in one without.
- **The Rune side.** On the host, `mise run test-rune`: 72 test binaries, 2,337 tests
  passed, none failed, 253 ignored. That is V6's 2,312 and 253, plus the 23 tests of
  `strata-app`'s `qa` module and the two of `strata-rune`'s `tests/qa.rs`. Clippy on the
  Rune lane passes with warnings denied, with every feature and with none.
  `mise run test-platform window::` passes its three tests: the windowed binary takes its
  arguments through the same `run` the QA mode now branches from.
- **Code health.** CodeScene, through its local tool since `CS_ACCESS_TOKEN` is not set
  here: every new file scores 10, and no changed file scores lower than at `b3b8f448e`
  (`tests/qa.rs` 7.63, `tests/qa/drive.rs` 6.98, `command/parse.rs` 5.38,
  `engine/dispatch.rs` 6.21, each as before).
- **Scripts.** `python3 -m unittest discover -s scripts -p 'test_*.py'`: 239 tests pass.
  `typos` and `git diff --check` are clean.

Not run, and why:

- A host QA run: no consent, and none needed.
- `./scripts/qa-mutation.sh` with `cut.patch`: it does not apply, as before V3.
- `mise run test-platform` beyond `window::`: `strata-platform` did not change.
- CI: the commits were not pushed when this was written. On a push the image publisher
  will fail as it has since the import, since the manifests it watches changed and the
  dependency stage did not; `Quality build and lint` resolves the same base as before.

## Open

- **Decision 12,** as [the image](#the-image-nothing-changed-and-why) section puts it.
- **Point `release.yml` and `ci.yml`'s usage checks at `strata-rune`.** Until then nothing
  in CI proves a release build of it lacks the QA mode. No release builds it yet.
- **A Rune request for a public physical-key entry,** when V8 marks the run that needs it.
- **Decision 13** is unchanged: no CI job builds, lints or tests the Rune lane, so the
  backend and its tests are checked locally.

## What it left for V8

- **Run a gate:** `./scripts/qa-parity.sh g2` for the required runs on both targets;
  `./scripts/qa-parity.sh g2 --pending` to see what each pending run does on Rune. A run's
  bundle is `target/qa-artifacts-rune/<scenario>/<run>/`: `failures.txt` and `transcript.jsonl`
  first, then `tree.txt` (Rune's role and key before the AT-SPI role) and `strata.log`.
  G1 on both targets takes about 5 s once built.
- **Compare verdicts, not `app`.** V3 found two runs of one binary differ in `app`. On G1
  the two targets' transcripts agree line for line in `ok`, `error`, `failures`, `result`,
  `criticals` and `changed`; Rune's has the `keymap` line more.
- **Promote a run** by changing its ledger entry to `required` and dropping `reason` and
  `blocker`. The runner's tests refuse an entry that is half changed.
- **What returns "not supported":** every step behind `Backend::command` but `keymap`
  (`reveal`, `reveal_control`, `set_scrollbar`, `capture`, `measure_text`, `drop_samples`,
  `clipboard_set`, `clipboard`) and every predicate behind `Backend::check` (`pixel`,
  `accent_pixels`, `repainted`, `contrast_below`, `footer_fill`, `elapsed`,
  `diagnostics_mention`).
- **What silently does nothing yet:** `scroll` is `None`, so the engine never wheels a
  clipped target into view and `scroll_value` finds nothing; the wheel turns only up and
  down; `field` is `None`, so `field` targets and predicates find no field (V9);
  `orientation` is `None`.
- **Reveal before aiming** is not built. A row a virtual list has not mounted is not in
  the tree, and `select` will time out on it.
- **The role map's gaps:** roles the slice has not drawn were mapped by name and not
  seen: buttons that toggle, menus, text fields, dialogs. Check each against GTK's tree
  when a run first targets one. An unlisted role is `filler`.
- **The key map's gaps:** keypad keys are the main block's keys, as in Rune. The built-in
  table lists the names the parity set uses and GDK's names for ASCII punctuation; any
  other name is an error that says so.
- **Settling.** A step returns once neither GLib nor Rune has anything ready. A GLib
  timer still running, such as the 75 ms before a child column opens, is not waited for,
  as on GTK: scenarios wait for what it brings.
- **Flaky runs:** none seen on the Rune target. The GTK suite's two are unchanged.
