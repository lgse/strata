# V10 Slice report

| | |
| --- | --- |
| Status | Done. [The report](slice-report.md) is written, and the exit command exits 0 at `d4f086f25` |
| Depends on | V9, V9b |
| Unblocks | The owner's decision on the slice, and on `strata-next` becoming an integration branch |
| Owns | `docs/rune/slice/slice-report.md` |
| Lands as | One documentation commit, and one more that closes the slice's index |

## Goal

Write the evidence the evaluation asked for: what the slice proved, what it cost, and what
the next slice would take. The exit command passes when this is written.

The slice plan named `docs/rune-port/slice-report.md`. The report goes beside the other
slice files instead.

## Scope

The report has six parts. Each number in it names the command that produced it.

1. **The parity matrix.** Each run of the parity set, its result on each target, and the
   pending runs grouped by reason code. There are seven codes since V8: `scope` is a run
   that needs a part the slice marks Out, and is most of G2's and G3's pending runs. Generate it from `tests/qa/parity/slice.toml` and
   the last three runs' results, not by hand. After V9 the ledger has 36 `required` runs
   (1 in G1, 22 in G2, 13 in G3) and 39 pending: 35 `scope`, 3 `platform`, 1 `rune`. Say
   which required runs pass on Rune for a weaker reason than on GTK: V9 names two.
2. **Harness reach.** The share of step kinds, targets and predicates the Rune backend
   implements, counted against the protocol in `crates/strata-qa`. And how many of the 351
   scenario files it could run in principle: those whose every step, target and predicate
   it implements, whatever the interface can do yet.
3. **Cost.** Lines changed and elapsed time per package, split into three: the refactor of
   Strata (V1 to V4), the Rune interface (V4b to V6), and the harness (V3 and V7). Take
   lines from `git diff --stat` between the package's first and last commit.
4. **Gaps.** Every gap found in strata-components, with its work package id, and every
   Rune draft the slice ran into, with its draft id. Include the ones V0 found: the row
   description hook, the pane group, and the wake of a covered window. From V8: C2d, and
   request 97, since filed as anomalyco/rune#657. From V9: C5a (the editor's focus return
   in a host-keyed collection), C2e (Left in a column with no cursor) and request 98 (the
   Escape after a dead key), drafted from the sources and not run in a window.
5. **Measurements,** on both targets, in the pinned image:
   - launch to first listing;
   - a 10,000-entry listing, from the navigation to the last event. V0 measured 8.9 ms on
     Rune in release in the image; the GTK figure was never taken.
6. **A projection for the next slice:** Icons and List on the same harness. State its
   basis: which parts of V5 to V9 carry over unchanged, and which repeat per view.

Also record:

- which owner decisions were made during the slice, and by what ruling, and which are
  open. Of V9's, 23 to 25 were ruled on 2026-11-11, all for the next slice: undo and the
  naming of a new item stay out, and their seventeen runs stay `scope` pending. 26 is open;
- the known differences between the two interfaces that no ruling makes a `deviation`.
  One so far: a rename the browser fails opens the inline editor again with the message,
  where the GTK view shows a dialog. The dialog is the target; the editor stands in until
  dialogs (C15) are mounted (decision 25);
- where each package's evidence is: the verification worktree's `target/v3-…` to
  `target/v9-61a7d6224/` and V9b's three `target/v9b-…/`, each named in its package's
  record;
- each stop condition and whether it was hit;
- the hand check of the windowed binary on Hyprland (decision 7). The owner does the
  check, not the package, and did it on 2026-11-11: a real directory in the Columns view,
  keyboard navigation and selection, and inline rename, all behaving correctly.
  [The report](slice-report.md#the-hand-check) records it as the owner stated it, with the
  command that builds and opens the window on a directory,
  `cargo run --locked -p strata-rune -- <directory>`. It is the only evidence in the slice
  from a real compositor, and no other compositor was checked;
- what the slice does not prove, in the report's first screen: input never crossed winit
  or a compositor.

## The exit check

Run `./scripts/qa-parity.sh slice` in the pinned container, alone: it shares
`target/qa-container` with every other QA script. It exits 0 only when:

1. every `required` run passes on both targets;
2. every other run of the set is `pending` with a reason code and a blocker;
3. each Rune mutation patch fails its scenario on the Rune target;
4. the full GTK suite and its 22 mutation patches behave as before.

Since [V9b](V9b-test-image-and-exit-check.md) the one command checks all four, in this
order, and runs every part whatever the earlier ones did:

| Part | What runs | Item |
| --- | --- | --- |
| The gates | The ledger's 36 `required` runs, on GTK and then on Rune | 1 |
| The runner and the whole GTK suite | The runner's 35 unit tests, the ledger's among them, and the 758 scenario runs, as `./scripts/qa.sh` runs them | 2 and 4 |
| The Rune mutation patches | The three under `tests/qa/parity/mutations/`, as `./scripts/qa-parity.sh mutations` | 3 |
| The GTK mutation patches | The 21 that apply, as `./scripts/qa-parity.sh gtk-mutations` | 4 |

"As before" has two named exceptions, the ledger's allowances
([README](README.md#what-the-exit-command-lets-stand)):

- `new-folder-with-selection-groups/columns` may fail in the whole-suite part. The command
  then prints `ALLOWANCE USED: new-folder-with-selection-groups/columns failed in the whole
  GTK suite; the ledger names it flaky before the slice.` and goes on. It does not run the
  run again and does not fail for it (decision 27, as the owner ruled it on 2026-11-11).
- `cut.patch` does not apply, as before the slice, and must not: the command prints
  `ALLOWANCE USED: cut.patch does not apply, as before the slice; the other 21 GTK mutation
  patches run.` every time.

The command ends with the allowances it used or refused, under `== allowances, from
tests/qa/parity/slice.toml ==`, and then `== slice exits <status> ==`. A failure of the
whole suite that no allowance names is in that list too, as `NO ALLOWANCE: …`, and the
command exits 1. The report quotes those lines from the run it names, and says whether the
flaky run failed in it. A flaky run's failed bundle is in
`target/qa-parity/allowance/<scenario>/<run>/`.

A run takes about 14 minutes with a warm cache: under two for the gates and the whole
suite, one for the Rune patches and 11 for the GTK ones. Its first line is the seeding's,
`Dependencies: …`: with the verification worktree's warm cache it says the cache is reused.
The base it runs in depends on whether V9b's image commit is checked out: the key is
`4fa6c8cf…` with it and `9229afdf…` without, and the environment is the same in both.

The command exits 0 whether or not the flaky run fails. It fails in about two whole-suite
runs in five, so expect `ALLOWANCE USED` for it in some runs and not in others.
[V9b](V9b-test-image-and-exit-check.md#what-the-allowance-is-worth) has the figures the
ruling rests on.

## Out of scope

- Fixing anything. A defect found while writing the report is recorded and goes back to
  the package that owns it.
- A recommendation on whether to continue. The report gives the owner what the decision
  needs.

## Tests

Documentation only, so no build is needed for the file itself: review the complete diff,
check every link and every command against the scripts, and run `git diff --check`. The
numbers come from runs that are named in the report with their artifact directories.

## Done when

- The report is committed.
- `./scripts/qa-parity.sh slice` exits 0, and the report names that run.
- The slice index marks every package done or says what is left.

## Where the work departs from this plan

- **The exit command ran before the report was written,** at `d4f086f25`. Every commit
  after it changes documents only, so the report names that run.
- **The matrix reads a survey as well.** The exit command does not start a pending run on
  Rune, so `./scripts/qa-parity.sh slice --pending` was run at the same commit: the matrix
  has each pending run's result on Rune and the line it stops at.
- **Harness reach is counted against the engine.** Most of the protocol is the shared
  engine's, not a backend's, so an item counts as implemented when the engine can run it
  from what the Rune backend answers. The report says what the count of scenario files
  cannot see.
- **Cost has a fourth group,** for V8, V9 and V9b, and no elapsed time: no record states
  any.
- **The measurements are three, not two.** A 10,000-entry listing is timed at launch, as
  V0 timed it, and again in a running process. All three are read at `Browser`'s own log
  lines, which both binaries write. Release binaries were built for it, in a directory of
  their own. The GTK figures include a window on Xvfb and the Rune figures do not, which
  the report says beside them.
- **The hand check was done,** so the report records it in place of the slot.
- **The generators are outside the tree,** in `target/v10-d4f086f25/` of the verification
  worktree, with their output.

## Done when, and the evidence

All on 2026-11-11, from the worktree at `~/dev/strata-next-verify`, in the pinned image,
with Docker. Logs and kept files are in its `target/v10-d4f086f25/`.

- **The report is committed:** [slice-report.md](slice-report.md).
- **The exit command exits 0, and the report names the run.** `./scripts/qa-parity.sh slice`
  at `d4f086f25`, in 13 min 43 s: 36 of 36 required runs on GTK and on Rune, the whole suite
  with 793 passed, none failed and 2 ignored, 3 of 3 Rune patches and 21 of 21 GTK patches
  caught, and one allowance used, `cut.patch`'s. The flaky run passed in this run
  (`qa-parity-slice.log`).
- **The slice index marks every package done** in the commit after the report's.

Also run: `./scripts/qa-parity.sh slice --pending`, where none of the 39 pending runs
passes on Rune (`qa-parity-pending-survey.log`); the measurements, in a
`./scripts/qa-drive.sh` session (`measure.sh`); `./scripts/qa.sh` once with the index as
its commit leaves it, since the runner's ledger test reads that file: 793 passed, none
failed and 2 ignored (`target/v10-index/qa-full.log`); and on the host, with no display
variable set, `cargo run --locked -p strata-rune -- --help`
(`hand-check-command-help.log`).

Checked for the documents: the complete diff was read, every link and every command in the
report was held against the files and the scripts, and `typos` and `git diff --check` are
clean.

Not run, and why: `./scripts/quality.sh`, `mise run test-rune` and the compositor tests,
because no file outside `docs/` and `AGENTS.md` changed; a host QA run, for which there is
no consent and no need.

## After the slice

The owner decides whether the slice is approved. If it is, `strata-next` becomes an
integration branch: work packages land through pull requests from then on, and the
commits made directly on the branch are pushed to `lgse/strata` if they have not been.
