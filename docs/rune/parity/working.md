# Working on a package

How a package of [the parity plan](README.md) goes from assignment to hand-off. `AGENTS.md`
is binding and comes first; this page adds only what the parity tracking asks, and links the
rest.

## Reading a package file

Each file under [`packages/`](packages/) holds the package's row of the plan, its runs, its
done-when list, its record, what its dependents must know, the notes other packages left
for it, and what is open. For an area package, the numbered done-when items are the plan's
[eight deliverables](README.md#what-every-area-package-delivers), and the ledger, not the
plan's count, decides which runs are the package's: H1 lists every run with its owner, a
package edits only its own entries (`docs/testing.md`, "Who owns a run"), and a package can
flip its Columns runs before A1 and A2 merge while its Icons and List runs wait for them.
The checkpoint a package feeds is its wave's, in the plan's
[Checkpoints](README.md#checkpoints).

## Before starting

- Read `AGENTS.md`; the plan's "What parity means", "Conventions", "Coordination" and
  "Validation and gates"; the package's own file; and "What dependents must know" in each
  dependency's file.
- Read what the package stands on: `docs/testing.md` ("The ledger", "Twins", and "Fuzzing"
  once F1 has merged), the `qa-scenarios` skill, the module docs of
  `crates/strata-app/src/slots.rs`, and `docs/rune/shared-state.md`.
- Check that each dependency's file reads `done`. One that does not, or a dependency the
  file names beyond the plan's, is a reason to ask, not to start.

## The claim, and work in flight

- **The claim is the assignment.** The owner, or the session the owner delegates
  coordination to, hands the package to an agent, and that assignment is the claim. The
  package's first pull request sets its file's state to `in progress`.
- **Work in flight is seen in the open pull requests**, `gh pr list -R lgse/strata --base
  strata-next`, and in each package's file once its pull requests merge.
- **[`STATUS.md`](STATUS.md) is not edited by package pull requests.** The package files are
  the source; whoever coordinates brings `STATUS.md` up to date from them, in small
  docs-only pull requests of its own.
- **The archived issues are not reopened, edited or commented on.** That is the
  coordinator's rule: they are history, linked from the files. A new finding's reference
  issue is filed and may be closed at once, as
  [`findings.md`](findings.md#recording-a-finding) says.

## Branch and worktree

- **Branch** `<type>/<issue>-<package>-<slug>` from `strata-next`, for example
  `feat/2134-a7-entry-management`. The number is the package's archived issue's, which its
  file links, since `AGENTS.md`, "Git workflow", asks for an issue number.
- **One worktree per package**, such as `../strata-a7`, with its own `target/`. A second pull
  request of the package branches again from `strata-next` once the first has merged; avoid
  stacked pull requests.
- **Merging, commits and others' pushes** follow `AGENTS.md`, "Git workflow" and "Two
  branches": `strata-next` is taken by merge and never rebased, and `Cargo.lock` is never
  merged by hand. A maintainer may push to the branch or comment on the pull request: fetch
  and merge before every push, never force-push, and answer on the pull request.
- **After each merge of `strata-next`**, read the package's own file again: other packages
  leave notes in it.

## While building

- **Running checks:** `AGENTS.md`, "Pre-push checks", "QA scenarios" and "Test image reuse",
  and the Rune side's rules under "The Rune side". `./scripts/qa-parity.sh <selection>
  --counts` and `--twins` read files on the host and start no container.
- **The Rune side's tests, without the gallery's integration tests.** By the owner's
  instruction of 2026-11-12 ([`owner.md`](owner.md#ruled)), `mise run test-rune [filter]`
  runs every Rune-side crate's tests and only the gallery's unit tests (`--lib --bins
  --examples`); a filter goes to both runs. The gallery's goldens, page tests and scripts are
  not part of any gate: `mise run test-gallery [filter]` runs them on demand, and
  `mise run bless` stays for rewriting goldens when the gallery is cleaned up. The gate for a
  change to a host, the shell or the key routing is `mise run test-platform window::`; a change
  to `strata-platform` still owes its own session tests: `mise run test-platform`, or the
  task with a filter for the shims it touches, such as `p2::`.
  What prompted it, measured that day on the 24-core machine the agents share: each gallery
  PNG golden and each script starts a `strata-gallery` process, and with no lavapipe installed
  each opens a Vulkan device on the real GPU, where the opens serialise in the driver. One
  render alone took 0.63 s; throughput peaked at 8 at once (6.1 renders a second) and fell
  beyond it (5.4 at 24, 4.5 at 48), and at 72 at once 70 to 72 threads sat in uninterruptible
  sleep. A Rune-lane run started about 1,400 of them (514 PNG goldens and 897 scripts; the
  accessibility goldens mount their scenes in process), 24 at a time, and with three package
  agents running lanes at once the load average passed 40.
- **The plain binary compiles.** For any change to a Rune-side or shared crate,
  `cargo check --locked -p strata-rune`, with default features and no `qa`; every other gate
  builds with `--all-features` and so cannot see a missing default-features build, and a
  test-only item in a shared crate breaks it the same way. The coordinator's addition of
  2026-11-12, after `strata-next` at `67578bfbc` failed it unnoticed (the plan's
  [Checks in place of CI](README.md#checks-in-place-of-ci)).
- **The compositor tests on a loaded machine.** `mise run test-platform` starts a sway and
  an Xwayland for each test, as many at once as the machine has cores, and with other
  packages building on the machine its waits ran out in `strata-platform`'s own session
  tests before it reached the rest (finding [#2218](https://github.com/lgse/strata/issues/2218) in [`findings.md`](findings.md)). The task
  now triples every wait unless `STRATA_TEST_TIMEOUT_SCALE` is set, which passed once under
  that load and is no guarantee. To run `strata-rune`'s window tests alone, the only run
  through a real window, which a change to a host, the shell or the key routing owes:
  `mise run test-platform window::`. A failure of a test run alone is the change's until
  shown otherwise.
- **While a gate runs from the worktree, leave the worktree alone, and commit by name.**
  `./scripts/qa-mutation.sh`, and so the mutation phases of `./scripts/qa-parity.sh slice`
  and `mutations`, apply each patch to the source files in place (`git apply`) and take it
  out again with `git restore`. A `git add -A` or `git commit -a` made meanwhile commits a
  mutation, and an uncommitted edit to a file a patch names stops the run or is discarded
  with the patch. So no source file is edited until the run ends; a Markdown file that must
  change is added by its own path; and `git status` is read before anything is committed.
  The mutation runner ignores an interrupt (SIGINT) when it was started detached, so a run
  that has to be stopped is killed, which leaves its patch in the tree: take it out by hand,
  `git restore -- <the patched files>`, and check that `git status --porcelain` is empty.
- **The rules carried over from the slice** are the plan's
  ([Rules carried over from the slice](README.md#rules-carried-over-from-the-slice)). The one
  ruled exception to never editing a GTK mutation patch is a line of `quick-preview.patch`
  ([`owner.md`](owner.md#ruled)).
- **A difference from GTK is not the agent's to decide.** Work down the plan's
  [order of resort](README.md#when-a-run-differs-the-order-of-resort). A `deviation` waits
  for the owner: record it in [`findings.md`](findings.md) and [`owner.md`](owner.md), leave
  the run `pending`, and go on. A run moves from `required` to `parallel` only with the
  owner's agreement.
- **Strata's own faults are `oracle` findings:** search [`findings.md`](findings.md), then
  the issues, by code and by scenario or seed name; record a new one as `findings.md` says
  (a reference issue, which may be closed at once, and an entry there); set it aside, and do
  not chase it.
- **Rune is read-only.** A gap is drafted into `docs/rune/upstream/rune/` as the plan's
  [Rune framework gaps](README.md#rune-framework-gaps) says, and then worked around.
- **Stop rather than widen.** Something outside the package's scope, an owner ruling, or a
  `contract` change another package owns goes under "Open" in the package's file, and in
  [`owner.md`](owner.md) when it is the owner's; the package carries on with what does not
  depend on it. A package that cannot go on at all is set to `held`, with the reason.
- **A finding or an owner question met mid-run** may land as its own small docs-only pull
  request, to `findings.md` or `owner.md`, that merges quickly, so other packages see it
  before the package's own pull request merges.

## Pull requests

- **Small, and targeting `strata-next`.** One per scenario folder is a natural size, each
  flipping the ledger rows it proves. A change to a seam others build on (the protocol in
  `crates/strata-qa`, the runner in `tests/qa`, the shell's slots, a shared component's
  public API) is a `contract` pull request of its own, labelled so.
- **The template, kept whole:** what changed and why, the steps and the expected result,
  `N/A` under "Visual evidence", and the validation run with its counts. Under "Related
  issue", link the package's file and name its archived issue; do not write "Closes", since
  that issue is closed and stays so.
- **The package's file in the same pull request:** one dated entry in its record, with the
  pull request's number, what it delivers and the runs it flips; what its dependents must
  know, if anything; the done-when items it completes, ticked; and what it leaves open. The
  validation and the evidence stay on the pull request, not in the file.
- **Before opening, when anything under `crates/` or `src/` changed:**
  check that every mutation patch still applies, from the repository root:
  `for p in tests/qa/parity/mutations/*.patch tests/qa/mutations/*.patch; do git apply --check "$p" 2>/dev/null || echo "does not apply: $p"; done`.
  It takes seconds and starts no container; it is not a mutation run. Only `cut.patch`
  (finding [#2161](https://github.com/lgse/strata/issues/2161)) is known not to apply. A patch that no longer applies is refreshed and
  proven once, as the plan's
  [When a mutation run is right](README.md#when-a-mutation-run-is-right) says.
- **Before opening:** the checks in the plan's
  ["Before a package's pull request"](README.md#before-a-packages-pull-request) table that the
  change reaches, risk-based as `AGENTS.md` sets out, and the plan's
  [checks in place of CI](README.md#checks-in-place-of-ci): on `strata-next` CI does not run,
  so every result, the Rune lane's included, is written on the pull request, and a check
  skipped is named with its reason.

## Review, the merge gate and the merge

- **A session other than the author's reviews each pull request** before it merges. Fix
  what is right, answer what is not, on the pull request.
- **The merge gate** is the plan's ([The merge gate](README.md#the-merge-gate)), run by the
  package's agent on its branch with `strata-next` merged in: the package's ledger selection
  on both targets (`./scripts/qa-parity.sh <package>`); or, when the runner, `strata-qa`, a
  shared crate or `src/` changed, every required run on both targets
  (`./scripts/qa-parity.sh all`), the whole GTK suite (`./scripts/qa.sh`) and
  `./scripts/quality.sh test`. No mutation set is run: a patch is proven once, alone, as the
  plan's [When a mutation run is right](README.md#when-a-mutation-run-is-right) says.
- **The merge**, once the gate and the checks in place of CI have passed and the review is
  answered; on `strata-next` it does not wait for CI, which does not run. A `CI` run that does
  appear on a pull request into `strata-next` (started before CI was turned off there, or by
  hand) is not waited for, and a red one is not a reason to hold a merge whose own checks
  passed. A merge commit,
  `gh pr merge <number> -R lgse/strata --merge`, never a squash or a rebase
  ([`owner.md`](owner.md#ruled)).
- **A GTK unit test failing on something the change did not touch** (in
  `./scripts/quality.sh test`, now that CI does not run on `strata-next`): read the failure,
  say on the pull request why the change cannot reach it, and rerun that test once alone with
  `./scripts/test-headless.py <name>`. A flaky GTK unit test goes into finding
  [#2175](https://github.com/lgse/strata/issues/2175) in [`findings.md`](findings.md).
- **When the package is done:** its last pull request sets its file's state to `done` and
  ticks its done-when list; its merged branches are deleted.

## What a hand-off records

Each time the work stops, to whoever assigned it, and in the package's file where it is
lasting:

- the state: a pull request opened and awaiting review, merged, or stopped, and why;
- the pull request numbers, the branch and the worktree;
- what was validated, where and with what counts, and what was not run; a skipped check is
  never reported as passed;
- the package's ledger counts before and after, from `./scripts/qa-parity.sh <package>
  --counts`;
- what is blocked or waits for the owner;
- what the packages that depend on this one must know.
