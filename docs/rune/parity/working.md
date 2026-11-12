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
  coordinator's rule: they are history, linked from the files.

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
- **The rules carried over from the slice** are the plan's
  ([Rules carried over from the slice](README.md#rules-carried-over-from-the-slice)). The one
  ruled exception to never editing a GTK mutation patch is a line of `quick-preview.patch`
  ([`owner.md`](owner.md#ruled)).
- **A difference from GTK is not the agent's to decide.** Work down the plan's
  [order of resort](README.md#when-a-run-differs-the-order-of-resort). A `deviation` waits
  for the owner: record it in [`findings.md`](findings.md) and [`owner.md`](owner.md), leave
  the run `pending`, and go on. A run moves from `required` to `parallel` only with the
  owner's agreement.
- **Strata's own faults are `oracle` findings:** search [`findings.md`](findings.md) by code
  and by scenario or seed name, record a new one, set it aside, and do not chase it.
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
- **Before opening:** the checks in the plan's
  ["Before a package's pull request"](README.md#before-a-packages-pull-request) table that the
  change reaches, risk-based as `AGENTS.md` sets out. CI builds the GTK lane only, so the
  Rune-lane evidence is written on the pull request.

## Review, the merge gate and the merge

- **A session other than the author's reviews each pull request** before it merges. Fix
  what is right, answer what is not, on the pull request.
- **The merge gate** is the plan's ([The merge gate](README.md#the-merge-gate)), run by the
  package's agent on its branch with `strata-next` merged in: the package's ledger selection
  on both targets (`./scripts/qa-parity.sh <package>`) and its Rune mutation patches
  (`./scripts/qa-parity.sh mutations`); and, when the runner, `strata-qa`, a shared crate or
  `src/` changed, `./scripts/qa-parity.sh slice` and `./scripts/quality.sh test` too.
- **The merge**, once the gate and CI are green: a merge commit,
  `gh pr merge <number> -R lgse/strata --merge`, never a squash or a rebase
  ([`owner.md`](owner.md#ruled)).
- **CI failing on something the change did not touch:** read the failure, say on the pull
  request why the change cannot reach it, and rerun the whole workflow once. A flaky GTK unit
  test goes into finding [#2175](https://github.com/lgse/strata/issues/2175) in [`findings.md`](findings.md).
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
