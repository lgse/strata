# V9b Test image and exit check

| | |
| --- | --- |
| Status | The exit check: done. The image: not landed yet, in commits of its own after these |
| Depends on | V9 |
| Unblocks | V10 |
| Owns | The allowances in `tests/qa/parity/slice.toml` and their checks in `tests/qa/parity.rs`, the `gtk-mutations` selection of `scripts/qa-parity.sh` |
| Commits | `37b63862f` (the allowances and the GTK patches in `slice`), `481392c68` (the closing list names a refused failure), then this record |

An infrastructure package, not in the slice plan. The owner's rulings of 2026-11-11 put it
between V9 and V10 so that V10 runs an exit command it can trust. It has two parts, landed
apart: the exit check here, and the test image's dependency stage (decision 12) after it.

## The exit check

`./scripts/qa-parity.sh slice` is the slice's exit command. After V9 it had two faults,
and neither was about the slice:

- It exited 1 whenever `new-folder-with-selection-groups/columns` failed in its whole-suite
  part. That GTK run was flaky before V3. V7 saw `slice` fail for it twice out of two.
- It did not check the fourth item of the exit check. It ran the whole GTK suite and left
  the GTK mutation patches to `./scripts/qa-mutation.sh`, run beside it, and a bare run of
  that script aborts on `cut.patch`, which has not applied since before V3.

### The flaky run (decision 27)

The owner ruled on 2026-11-11: a named allowance. The ledger names this one run as flaky
before the slice. When it fails in `slice`'s whole-suite part, `slice` runs it again alone,
reports both results, and fails only if the rerun fails. No scenario is edited.

How it is built:

- **The allowance is data.** `tests/qa/parity/slice.toml` has one `[[flaky]]` entry: the
  scenario, the run, the reason, and under `recorded` the document that records the flake,
  `docs/rune/slice/README.md`.
- **Only `slice` reads it.** `./scripts/qa.sh` is unchanged and fails on the run as on any
  other. So does a gate: the allowance is for the whole-suite part, and the ledger refuses
  an allowance for a run of the parity set.
- **What `slice` does.** When the whole suite exits nonzero, `scripts/qa_parity.py slice
  --failed-in <log>` reads the runner's `test … FAILED` lines. If every failed test is a
  run the ledger names as flaky, it prints them and `slice` runs each alone, with
  `--exact`. If any other test failed, a runner test included, or the suite failed without
  naming a test, as a build error does, nothing is run again and `slice` fails.
- **What it prints.** Before the rerun: `== ALLOWANCE: <run> failed in the whole GTK suite;
  the ledger names it flaky, so it runs again alone ==`. After it, one of `ALLOWANCE USED:
  <run> failed in the whole GTK suite and passed alone.` and `ALLOWANCE REFUSED: <run>
  failed in the whole GTK suite and again alone.` A failure it will not run again is
  `NO ALLOWANCE: <tests> failed in the whole GTK suite; nothing is run again.` Each of
  those lines is repeated when the command ends, under `== allowances, from
  tests/qa/parity/slice.toml ==`, with the stale patch's line below. The last line is
  always `== slice exits <status> ==`.
- **What counts as passing alone.** The rerun must exit 0 and print `PASS <run>`. A rerun
  that ran no test is refused.
- **What it keeps.** The failed run's bundle is copied to
  `target/qa-parity/allowance/<scenario>/<run>/` before the rerun replaces it in
  `target/qa-artifacts/`. The lines are also in `target/qa-parity/allowances.txt`. Both are
  emptied when `slice` starts.

### What the allowance is worth

Less than the ruling assumed. It was made on the understanding that the run mostly passes
alone. At these commits it mostly fails alone, so the rerun usually refuses the allowance
it was meant to grant.

| How the run was started | Passed | Where |
| --- | --- | --- |
| In the whole GTK suite, by `slice` | 1 of 2 | `qa-parity-slice-1.log` at `37b63862f`; `qa-parity-slice.log` at `481392c68` |
| Alone, `./scripts/qa.sh new-folder-with-selection-groups/columns` | 1 of 6 | `flaky-alone-{1..6}.log` |
| Alone, as `slice`'s rerun | 0 of 3 | The two rehearsals below, and the run at `481392c68` |
| With its scenario's two other runs, `./scripts/qa.sh new-folder-with-selection-groups` | 1 of 8 | `flaky-with-its-scenario-{1..8}.log` |

Every failure is the same one: the wait for the opened folder's column to take focus times
out after 15 s. The application and the runner are the same at both commits. V3 saw the
same shape at `65eb2da03`, 5 failures in 6 alone and "in a full run it usually passes"; V7
saw 4 passes in 4 alone. The run is not flaky under load so much as broken without it.

**What follows for the exit command.** The whole suite fails for this run in something
like two runs in five, going by these and the earlier packages' runs. With the rerun
passing about one time in nine, `slice` still exits 1 for this run alone in roughly a
third of its runs. It happened in the last run of this package: 792 passed, the flaky run
failed, the rerun failed, `ALLOWANCE REFUSED`, exit 1, with every other part passing.

The allowance is built as ruled and does what the ruling says. Whether it should say
something else is the owner's:

1. **Keep it.** V10 runs `slice` until it exits 0 and names every run. About two runs in
   three exit 0.
2. **Allow the named run outright in the whole-suite part:** print the failure, list it as
   used, never fail on it. `slice` then cannot fail for this run. It also stops telling a
   regression in this one run from the flake, though the rerun alone cannot tell them
   apart either, since it fails nearly every time. The run is outside the parity set:
   New Folder with Selection is no part of the slice.
3. **Fix the run,** on `main`: either the scenario's wait or the focus race under it. It
   is a GTK-side change with its own reason, and the allowance then goes.

Changing the condition is one branch of `alone` in `scripts/qa-parity.sh`. A rerun of the
whole suite instead was not built or measured: at two failures in five it would still
fail twice running about one time in six.

### The GTK mutation patches (the exit check's fourth item)

The item reads: the full GTK suite and its 22 mutation patches behave as before. Before
the slice, 21 patches were caught and `cut.patch` did not apply. V7 and V8 left the patches
out of `slice` and wrote that `./scripts/qa-mutation.sh` is "run beside it".

**What was chosen: `slice` runs them.** `./scripts/qa-parity.sh gtk-mutations` is a new
selection, and `slice` runs it as its last part. It:

1. asks `git apply --check` of every patch a scenario names as its `mutation`;
2. compares the ones that do not apply with the ledger's `[[stale_patch]]` entries, which
   name `cut` alone;
3. refuses to go on, with exit status 2, if a patch fails to apply without being named, or
   a named patch applies again, or a named patch is no scenario's;
4. prints `ALLOWANCE USED: cut.patch does not apply, as before the slice; the other 21 GTK
   mutation patches run.`;
5. runs the 21 through `scripts/qa_mutation.py`, exactly as `./scripts/qa-mutation.sh
   <21 names>` does: the unmodified tree once, then each patch.

**Why.** The other way was to leave the patches as a separately named command in V10's
plan. That keeps `slice` 11 minutes shorter, and it leaves the exit check as two commands,
one of them with 21 names typed from a document, and with `cut.patch`'s status argued in
prose. With the patches in `slice`, one command checks all four items, and "as before" is
tested instead of asserted: `cut.patch` must still be stale, so the day it is rebased the
command says the entry has to go, and a second patch going stale cannot hide behind the
first.

`cut.patch` was not edited, and no GTK patch was. The `[[stale_patch]]` entry is this
package's reading of "as before", not an owner ruling: it is for the owner to confirm.

### What keeps the allowances from growing

- **They are checked against the suite.** `Ledger::allowance_problems` in
  `tests/qa/parity.rs` refuses a `[[flaky]]` entry that is no run the runner plans, or is a
  run of the parity set, and a `[[stale_patch]]` entry that no scenario names as its
  mutation. It refuses either without a reason, listed twice, or citing a document that
  is missing or does not name the run or the patch file.
- **The list is pinned.** The runner test
  `the_ledger_allows_only_the_flaky_run_and_the_stale_patch_it_was_given` holds the
  committed ledger to those two entries. Another one needs that test changed, in a diff a
  reviewer sees, beside the document that records the ruling.
- **A stale patch has to be stale.** `sort_gtk_patches` in `scripts/qa_parity_mutation.py`
  is checked by `scripts/test_qa_parity.py` against the committed tree: the patches the
  ledger calls stale are exactly the ones `git apply --check` refuses.
- **The ledger's format refuses the rest.** An entry with an unknown field, or with
  `reason` or `recorded` missing, does not parse.

## Rulings recorded

All by the owner on 2026-11-11, and all now in the decision table of the
[README](README.md#owner-decisions):

| # | Ruling |
| --- | --- |
| 13, 18 | The `rune` CI job and the usage checks of `release.yml` and `ci.yml` wait until after the slice |
| 21 | The Appearance popover is left for the next slice. Its five runs stay `scope` pending |
| 22 | The Menu key and Shift+F10 wait until after the slice |
| 27 | New: the flaky run gets a named allowance, as above |

## Where the work departs from the plan

- **The package is not in the slice plan.** It follows two rulings made after V9.
- **`slice` runs the GTK mutation patches.** The exit check always named them; V7's and
  V8's `slice` did not run them.
- **The ledger holds more than runs.** It was the list of the parity set. It now also
  holds the two allowances, because that is the file `slice` already reads and the runner
  already checks.
- **`scripts/qa_parity_mutation.py` takes a target.** `rune`, the default, is what it did;
  `gtk` is the new part. `./scripts/qa-parity.sh mutations` is unchanged.

## Tests

In the runner (`tests/qa/parity_tests.rs`, listed in `tests/qa/unit_tests.rs`): two new
tests, 35 in all.

| Test | Shows |
| --- | --- |
| `the_ledger_allows_only_the_flaky_run_and_the_stale_patch_it_was_given` | The committed allowances stand on a real run and a real patch, a reason and a document that names them, and are those two |
| `an_allowance_for_nothing_real_or_with_nothing_behind_it_is_refused` | Eight ways an allowance is refused, and three malformed entries that do not parse |

In `scripts/test_qa_parity.py`: six new tests and one reworded, 15 in all.

| Test | Shows |
| --- | --- |
| `FlakyAllowanceTests` (3) | The ledger's names; only a flaky run is run again, and any other failed test, a runner test included, stands; the command prints the runs or exits 1, and a failure that names no test is not excused |
| `StalePatchTests` (1) | A stale patch is left out only while the ledger names it and it stays stale: three ways the selection refuses to start |
| `RepositoryTests` (2 new) | The committed ledger's stale patches are the ones that do not apply; the script asks the ledger with the runner's own failure line |

## Done when, and the evidence

All on 2026-11-11, in the pinned image keyed `9229afdf…`, which this part does not change,
from the worktree at `~/dev/strata-next-verify`. Logs and kept files are in its
`target/v9b-37b63862f/` and `target/v9b-481392c68/`.

- **The exit command checks all four items and exits 0 when nothing fails.** At
  `37b63862f`, `./scripts/qa-parity.sh slice` exits 0 in 13 min 42 s
  (`qa-parity-slice-1.log`): 36 of 36 required runs on GTK and on Rune; the whole suite
  with 793 passed, none failed and 2 ignored, which is 758 scenario runs and the runner's
  35 tests; 3 of 3 Rune patches caught; `ALLOWANCE USED: cut.patch does not apply, as
  before the slice; the other 21 GTK mutation patches run.` and 21 of 21 caught, in 11.3
  minutes. The summaries are in `slice-1/`.
- **The same command at the last commit met the flake.** At `481392c68` it exits 1 in 13 min 57 s
  (`qa-parity-slice.log`): the gates pass as above, the whole suite has 792 passed and
  the flaky run failed, the rerun alone fails, and the command prints `ALLOWANCE REFUSED:
  new-folder-with-selection-groups/columns failed in the whole GTK suite and again
  alone.` The Rune patches (3 of 3) and the GTK patches (21 of 21, in 11.3 minutes) are caught. Nothing
  else failed. The failed bundle is in `qa-parity/allowance/`.
- **Each branch of the allowance, rehearsed.** A stand-in for `cargo` in the container
  (`rehearsal-cargo`) cuts the whole suite, and only it, down to one real run and then
  reports chosen tests as the runner reports a failure. Everything else is real: the
  ledger, `scripts/qa_parity.py`, the rerun. At `481392c68`, with the mutation parts not
  started (`rehearsal-docker-short`), so each exits 1 at the end whatever came before:
  - the flaky run and another test fail: `NO ALLOWANCE: escape-clears-selection/list
    failed in the whole GTK suite; nothing is run again.`, and no rerun
    (`rehearsal-flaky-and-another.log`);
  - the flaky run fails alone: the real run is run again, fails, `ALLOWANCE REFUSED`
    (`rehearsal-flaky-alone.log`);
  - a run that passes alone: with the ledger's entry pointed at
    `new-folder-with-selection-groups/list` in the worktree only
    (`rehearsal-stand-in-ledger.diff`), that run is run again, passes, `ALLOWANCE USED:
    new-folder-with-selection-groups/list failed in the whole GTK suite and passed alone.`
    (`rehearsal-stand-in-passes-alone.log`);
  - a name that is no run: the rerun runs no test and is refused
    (`target/v9b-481392c68-superseded-a2ee7c3bc/rehearsal-stand-in-names-no-run.log`, at
    the commit `481392c68` amends).

  At `37b63862f` the first two were also run with every part started, 13 minutes each:
  exit 1 both times, 3 of 3 and 21 of 21 patches caught (`rehearsal-flaky-only.log`,
  `rehearsal-flaky-and-another.log`).
- **The runner's tests.** The two new ones pass in every whole-suite run above, among 35.
- **The Rust suite and lint,** at `37b63862f`; `481392c68` changes two scripts and their
  test. `./scripts/quality.sh fmt` and `clippy` are clean, and `./scripts/quality.sh test`
  verifies 3,958 passed exactly once with 771 ignored: V9's 3,956 and the two runner tests
  (`quality-{fmt,clippy,test}.log`).
- **Scripts.** `python3 -m unittest discover -s scripts -p 'test_*.py'`: 249 tests pass,
  V9's 243 and six new. `git diff --check` is clean.
- **Code health.** CodeScene, through its local tool since `CS_ACCESS_TOKEN` is not set:
  `tests/qa/parity.rs`, `tests/qa/parity_tests.rs`, `scripts/qa_parity.py`,
  `scripts/qa_parity_mutation.py` and `scripts/test_qa_parity.py` score 10, and its
  check of the changed files finds no decline.

Not run, and why:

- A host QA run: no consent, and none needed.
- `mise run test-rune` and the compositor tests: no file of the Rune side changed.
- `./scripts/qa-mutation.sh` on its own: `slice` runs the same 21 patches through the
  same function.
- CI: nothing was pushed. This part changes no file a workflow watches for the image, so
  a push resolves the same base as before.

## Open

- **Decision 27's condition,** as [above](#what-the-allowance-is-worth): the allowance is
  refused more often than it is used.
- **The `[[stale_patch]]` entry** is this package's reading of "as before": for the owner
  to confirm.
- **`cut.patch` is still stale** and `new-folder-with-selection-groups/columns` still
  fails. Both need a change on `main`.
- **`distant-archive-scrolled-into-view/icons`** has no allowance. It failed once, at V3,
  and not in any run of this package.
