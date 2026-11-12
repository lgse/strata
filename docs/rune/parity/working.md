# Working on a package

How a package of [the parity plan](README.md) is taken from claim to hand-off, for an agent
or a person. `AGENTS.md` is binding and comes first; this restates what the plan's
"Coordination" and "Validation and gates" ask, in the order a package meets it.

## Before starting

- Read `AGENTS.md`; the plan's "What parity means", "Conventions", "What every area package
  delivers", "Coordination" and "Validation and gates"; the package's own file under
  [`packages/`](packages/); and, in each dependency's file, "What dependents must know".
- Read what the package stands on: the ledger (`docs/testing.md`, "The ledger"), twins
  (`docs/testing.md`, "Twins", and the `qa-scenarios` skill), the shell's slots (the module
  docs of `crates/strata-app/src/slots.rs`), shared state (`docs/rune/shared-state.md`),
  and, once F1 has merged, fuzzing (`docs/testing.md`, "Fuzzing").
- Check that every dependency's row in [`STATUS.md`](STATUS.md) reads `done`. A dependency
  that is not done, or that the package's file names beyond the plan's, is a reason to stop
  and ask, not to start.

## Claiming, branch and worktree

- **The claim** is the package's first pull request: it sets the package's row in
  [`STATUS.md`](STATUS.md) to `in progress` and names the branch in the package's file.
- **Branch** `<type>/<issue>-<package>-<slug>` from `strata-next`, for example
  `feat/2134-a7-entry-management`. The number is the package's archived issue's, which its
  file links, since `AGENTS.md` asks for an issue number in a branch name.
- **One worktree per package**, such as `../strata-a7`, with its own `target/`. A second pull
  request of the package branches again from `strata-next` once the first has merged; avoid
  stacked pull requests.
- **Merge, never rebase.** A branch takes `strata-next` by merge, and a pushed commit is
  never rewritten. `Cargo.lock` is never merged by hand: take `strata-next`'s copy and
  regenerate it.
- **Others may push to the branch.** A maintainer may review a pull request, comment on it or
  push commits to its branch. Fetch before every push and merge the remote branch if it
  moved; never force-push, and answer their comments on the pull request.
- **Commits** are Conventional Commits, focused, with no `Co-Authored-By` or other
  attribution trailer.

## While building

- **One pinned-container run at a time on a machine.** `./scripts/qa.sh`, `quality.sh`,
  `qa-mutation.sh`, `qa-parity.sh` runs, `qa-fuzz.sh` (once F1 has merged) and a
  `qa-drive.sh` session from start to stop share the container's build cache. Keep each run scoped to the package's selection, not `all`.
  `./scripts/qa-parity.sh <selection> --counts` and `--twins` read files on the host and
  start no container.
- **Two lanes.** Never run cargo with `--workspace` alone; use `scripts/lanes.py`. Rune-side
  tests run through `mise run test-rune <filter>`, with a filter that matches at least one
  test; GTK-lane tests through `./scripts/test-headless.py <filter>`. Nothing uses the
  desktop display or the session bus.
- **The oracle is never edited to pass.** No change to a GTK scenario's steps, a visual
  baseline or a GTK mutation patch; the `parity` header label is the one edit allowed to a
  GTK scenario. The one ruled exception is a line of `quick-preview.patch`
  ([`owner.md`](owner.md#ruled)).
- **A difference from GTK is not the agent's to decide.** Work down the plan's order of
  resort: fix the port, absorb it in the backend, reword the scenario on `main`, a twin under
  an exception the reviewer may approve (`timing`, `a11y`, `toolkit`, a clear-cut
  `oracle`), or `pending` with its blocker named. A `deviation` waits for the owner: record
  it in [`findings.md`](findings.md) and [`owner.md`](owner.md), leave the run `pending`,
  and go on. A run moves from `required` to `parallel` only with the owner's agreement.
- **Strata's own faults are `oracle` findings:** search [`findings.md`](findings.md) by code
  and by scenario or seed name, record a new one in the pull request that meets it, set it
  aside, and do not chase it.
- **Rune is read-only.** A gap is drafted into `docs/rune/upstream/rune/` as the plan's
  "Rune framework gaps" says, taking the next free number, and then worked around; nothing
  is filed upstream.
- **Stop rather than widen.** Something outside the package's scope, an owner ruling, or a
  `contract` change another package owns is written under "Open" in the package's file (and
  in [`owner.md`](owner.md) when it is the owner's), and the package carries on with what
  does not depend on it. A package that cannot go on at all is set to `held`, with the
  reason in its file.

## Pull requests

- **Small, and targeting `strata-next`.** One per scenario folder is a natural size, each
  flipping the ledger rows it proves. A change to a seam others build on (the protocol in
  `crates/strata-qa`, the runner in `tests/qa`, the shell's slots, a shared component's
  public API) is a `contract` pull request of its own, labelled so.
- **The template, kept whole:** what changed and why, the steps and the expected result, and
  `N/A` under "Visual evidence". Under "Related issue", link the package's file and name its
  archived issue; do not write "Closes", since that issue is closed and stays so.
- **The package's file in the same pull request:** a dated entry in its record (what the
  pull request delivers, the runs it flips, the commands and their counts, the evidence
  directory), the done-when items it completes ticked, what its dependents must know, and
  what it leaves open. The merge commit is added to the entry by the package's next change
  to its file.
- **Before opening:** the checks in the plan's "Before a package's pull request" table that
  the change reaches, risk-based as `AGENTS.md` sets out, with `./scripts/quality.sh fmt` and
  `clippy` once at the end. CI builds the GTK lane only, so the Rune-lane evidence
  (`mise run test-rune`, the Rune lane's fmt and clippy, the parity runs) is written on the
  pull request.

## Review, the merge gate and the merge

- **A session other than the author's reviews each pull request** before it merges. Fix
  what is right, answer what is not, on the pull request.
- **The merge gate**, run by the package's agent on its branch with `strata-next` merged in:
  the package's ledger selection on both targets (`./scripts/qa-parity.sh <package>`) and its
  Rune mutation patches (`./scripts/qa-parity.sh mutations`); and, when the runner,
  `strata-qa`, a shared crate or `src/` changed, the full gate too:
  `./scripts/qa-parity.sh slice` (the parity set, both mutation sets and the whole GTK suite
  with its two named allowances) and `./scripts/quality.sh test`.
- **The merge**, once the gate and the pull request's CI checks are green: a merge commit,
  `gh pr merge <number> -R lgse/strata --merge`, never a squash or a rebase. The owner's
  ruling is in [`owner.md`](owner.md#ruled).
- **CI failing on something the change did not touch:** read the failure, say on the pull
  request why the change cannot reach it, and rerun the whole workflow once (a failed shard
  cannot be rerun alone). A flaky GTK unit test goes into finding [#2175](https://github.com/lgse/strata/issues/2175) in
  [`findings.md`](findings.md).
- **When the package is done:** its last pull request sets its row to `done` and ticks its
  done-when list; its merged branches are deleted.

## What a hand-off records

Each time the work stops, in the package's file and to whoever assigned it:

- the state: a pull request opened and awaiting review, merged, or stopped, and why;
- the pull request numbers, the branch, the worktree and the commits;
- what was validated, where and with what counts, and what was not run; a skipped check is
  never reported as passed;
- the package's ledger counts before and after, from `./scripts/qa-parity.sh <package>
  --counts`;
- what is blocked or waits for the owner;
- what the packages that depend on this one must know.
