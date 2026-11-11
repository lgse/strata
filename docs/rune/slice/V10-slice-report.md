# V10 Slice report

| | |
| --- | --- |
| Status | Not started |
| Depends on | V9 |
| Unblocks | The owner's decision on the slice, and on `strata-next` becoming an integration branch |
| Owns | `docs/rune/slice/slice-report.md` |
| Lands as | One documentation commit |

## Goal

Write the evidence the evaluation asked for: what the slice proved, what it cost, and what
the next slice would take. The exit command passes when this is written.

The slice plan named `docs/rune-port/slice-report.md`. The report goes beside the other
slice files instead.

## Scope

The report has six parts. Each number in it names the command that produced it.

1. **The parity matrix.** Each run of the parity set, its result on each target, and the
   pending runs grouped by reason code. Generate it from `tests/qa/parity/slice.toml` and
   the last three runs' results, not by hand.
2. **Harness reach.** The share of step kinds, targets and predicates the Rune backend
   implements, counted against the protocol in `crates/strata-qa`. And how many of the 351
   scenario files it could run in principle: those whose every step, target and predicate
   it implements, whatever the interface can do yet.
3. **Cost.** Lines changed and elapsed time per package, split into three: the refactor of
   Strata (V1 to V4), the Rune interface (V4b to V6), and the harness (V3 and V7). Take
   lines from `git diff --stat` between the package's first and last commit.
4. **Gaps.** Every gap found in strata-components, with its work package id, and every
   Rune draft the slice ran into, with its draft id. Include the ones V0 found: the row
   description hook, the pane group, and the wake of a covered window.
5. **Measurements,** on both targets, in the pinned image:
   - launch to first listing;
   - a 10,000-entry listing, from the navigation to the last event. V0 measured 8.9 ms on
     Rune in release in the image; the GTK figure was never taken.
6. **A projection for the next slice:** Icons and List on the same harness. State its
   basis: which parts of V5 to V9 carry over unchanged, and which repeat per view.

Also record:

- which owner decisions were made during the slice, and by what ruling;
- each stop condition and whether it was hit;
- the hand check of the windowed binary on Hyprland (decision 7): what was tried, on which
  compositor, and what was seen. It is the only evidence in the slice from a real
  compositor;
- what the slice does not prove, in the report's first screen: input never crossed winit
  or a compositor.

## The exit check

Run `./scripts/qa-parity.sh slice` in the pinned container. It exits 0 only when:

1. every `required` run passes on both targets;
2. every other run of the set is `pending` with a reason code and a blocker;
3. each Rune mutation patch fails its scenario on the Rune target;
4. the full GTK suite and its 22 mutation patches behave as before.

The fourth means `./scripts/qa.sh` and `./scripts/qa-mutation.sh` in full, after the last
change to the runner or the engine.

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

## After the slice

The owner decides whether the slice is approved. If it is, `strata-next` becomes an
integration branch: work packages land through pull requests from then on, and the
commits made directly on the branch are pushed to `lgse/strata` if they have not been.
