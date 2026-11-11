# V9b Test image and exit check

| | |
| --- | --- |
| Status | The exit check: done, with decision 27's condition back with the owner ([why](#what-the-allowance-is-worth)). The image: done and validated here, and not pushed: the push changes the base's key and waits for the owner's word ([the order](#what-a-push-does)) |
| Depends on | V9 |
| Unblocks | V10 |
| Owns | The allowances in `tests/qa/parity/slice.toml` and their checks in `tests/qa/parity.rs`, the `gtk-mutations` selection of `scripts/qa-parity.sh`, the dependency stage of `tests/container/Dockerfile`, `tests/container/stub-workspace.py`, `scripts/dependency_seed.py` |
| Commits | The exit check: `37b63862f` (the allowances and the GTK patches in `slice`), `481392c68` (the closing list names a refused failure), `c7aa6dac2` (its record). The image: `50124f6e2` (the seeding), `7c3d69d3f` (the stage), then this record |

An infrastructure package, not in the slice plan. The owner's rulings of 2026-11-11 put it
between V9 and V10 so that V10 runs an exit command it can trust. It has two parts, landed
apart so that the first can be pushed without the second: the exit check, and the test
image's dependency stage (decision 12).

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
| In the whole GTK suite | 3 of 5 | Passed: `qa-parity-slice-1.log` at `37b63862f`, `qa-full.log` and `qa-parity-slice.log` at `7c3d69d3f`. Failed: `qa-parity-slice.log` at `481392c68`, and `qa-full.log` at this record's commit |
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
third of its runs. It happened in the last run of the exit-check part: 792 passed, the
flaky run failed, the rerun failed, `ALLOWANCE REFUSED`, exit 1, with every other part
passing.

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

## The test image (decision 12)

The owner ruled twice on 2026-11-11. First: add the Rune side to the image's dependency
stage rather than fetch on first use. V7 found that no run read that stage, and put
[three ways forward](V7-rune-qa-backend.md#the-image-nothing-changed-and-why) back to the
owner, who chose the third, knowing its risks: fix the stage so the publisher passes, and
change the scripts to seed their caches from the image's dependencies.

### What was wrong

- **The stage did not load the workspace.** It stubbed the crates directly under `crates/`
  with an empty `src/lib.rs`. `strata-test-peer` sits deeper and `.dockerignore` kept its
  manifest out; `strata-gallery`'s manifest names example files the stage had deleted. The
  publisher has failed on `strata-next` since the import, in under a minute.
- **Nothing used it.** `scripts/image_base.py build` built the `toolchain` stage, and every
  script kept its Cargo home and target directory under the checkout's `target/`. A new
  worktree fetched 649 crates and Rune's repository on its first run, the GTK lane's
  among them.

### The stage

`tests/container/Dockerfile`'s last stage, `dependencies`, now:

1. copies `Cargo.toml`, `Cargo.lock` and every `Cargo.toml` under `crates/`, at any depth
   (`.dockerignore` lets `crates/**/Cargo.toml` in), and deletes anything else found there;
2. stubs the workspace with `tests/container/stub-workspace.py`: for each package the root
   manifest's `members` reach, an empty `src/lib.rs` and `src/main.rs`, an empty file for
   every `[[bin]]`, `[[example]]`, `[[test]]` and `[[bench]]` the manifest names, at its
   `path` or where Cargo infers it, and a build script when the manifest has
   build-dependencies. No source or build script of Strata's runs in the stage;
3. runs `cargo fetch --locked`, which fetches every crate the lockfile names, for both
   lanes and every feature;
4. builds the two QA builds, each in a cargo run of its own, never `--workspace`:
   `cargo test --locked --features qa,invariant-checks --test qa --no-run`, which is
   `qa.sh`'s build, and `cargo build --locked -p strata-rune --features qa`, which is
   `qa-parity.sh`'s;
5. removes what the stubs built, with `cargo clean -p` for each workspace package, and
   fails if `stub-workspace.py leftovers` finds anything named for a package or one of its
   targets where Cargo puts artifacts, or if no compiled crate is left;
6. moves the Cargo home and the target directory to `/opt/strata-dependencies/cargo` and
   `build`, with `lock.sha256` and `cache-path` beside them, and leaves `/workspace` empty.

Three things about it are not obvious.

- **It builds at the paths a run uses.** The stage's Cargo home is
  `/workspace/target/qa-container/cargo` and its target directory is `build` beside it.
  Cargo's fingerprint of a registry crate holds the absolute path of its source file, so
  compiled crates copied to a cache at any other path are all rebuilt. That is why only
  `target/qa-container` takes the compiled crates and `target/quality-container` takes the
  fetched ones alone.
- **A stub must never pass for the real crate.** Cargo decides whether a workspace crate
  is fresh from file times, and a checkout's sources can be older than the image's stubs,
  so a stub's artifact left in the seed could be taken for the real crate's. Step 5 is
  therefore a hard stop, checked twice, and the cold-cache proof below compiles and runs
  the real crates from the seed.
- **The seeded Cargo home is not cleaned.** It carries `config.toml` with
  `cache.auto-clean-frequency = "never"`. Cargo deletes a crate's sources after a month
  without use and its archive after three, on the first command of a day. Measured here: a
  copy of the seed with its last-use times set 100 days back went from 649 crates to 36
  after one `cargo build -p strata-qa`. With the setting it kept all 649. A base lives
  longer than a month, so without it the first run from an older base would throw most of
  the seed away.

No distribution package was added, and no Vulkan driver (decision 3 stands).

### The seeding

`scripts/dependency_seed.py` runs in the container before Cargo, from `qa.sh`,
`qa-parity.sh`, `qa-drive.sh` and `quality.sh`, and through them from `qa-mutation.sh` and
both mutation selections of `qa-parity.sh`. It prints what it did on one `Dependencies:`
line.

| The cache | What it does |
| --- | --- |
| Warm: `cargo/registry` or `cargo/git` exists | Nothing. "reusing the Cargo cache in …; nothing is copied." It does not look at what is in it |
| Empty, `target/qa-container` | Copies the target directory, then the Cargo home: 4 GB, in 5 s here, where the copy shares blocks. The run compiles Strata's own crates and nothing else |
| Empty, another path, such as `target/quality-container` or CI's `STRATA_TEST_CARGO_CACHE` | Copies the Cargo home alone, 1.2 GB in 3 s here, and says the compiled crates are for another path. The cache compiles everything, from the copied sources |
| Empty, but its target directory already holds builds | Copies the Cargo home and leaves the builds |
| Any, in a base without `/opt/strata-dependencies` | Nothing. Cargo fetches, as before |

- **A lockfile ahead of the image** is said loudly and still works. The seeding compares
  the checkout's `Cargo.lock` with `lock.sha256` and, when they differ, prints
  `Dependencies: WARNING: Cargo.lock has moved past the image.` with both digests, then
  copies what the image has. Cargo fetches and builds what differs, with its own
  `Downloading` lines.
- **It keeps timestamps.** The copy is `cp --archive`. Cargo compares a dependency's
  output time with its dependents' to decide what is fresh.
- **A copy cut short is made again.** Each part is written beside its destination and
  moved in, the target directory first and `cargo/registry` last, and `registry` is what
  makes a cache count as fetched.
- **It cannot fail a run.** If the copy fails, the partial copies are removed and Cargo
  fetches, as before.
- **`quality.sh fmt` and a CI shard skip it.** Neither compiles.
- **`scripts/image_base.py build` builds the `dependencies` stage,** the one the publisher
  pushes as the base, so a base built locally seeds a cache as the published one does. The
  build now needs crates.io and Rune's repository as well as the distribution's archive.

Image-input hashing, the provenance labels and their checks are as they were, with one
more input: `stub-workspace.py` is hashed into the image key. `dependency_key` now hashes
every manifest under `crates/`, not only those one level down. Ordinary runs still build
no image and fetch no distribution package.

### What the build costs

Timed on this machine, an Intel Core Ultra 9 285K, with the build held to four
performance cores (a BuildKit builder with `cpuset-cpus=0-3`), from nothing
(`--no-cache`), for the stage the publisher builds, with the layers compressed into an OCI
archive as a push compresses them.

| Step | Four cores here | The publisher's runner |
| --- | --- | --- |
| The environment: packages, the patched GTK, the toolchain | 1 min 45 s | 1 min 46 s, measured: run 42929411753, which built the same stages |
| `cargo fetch`, the two builds, the cleanup | 1 min 56 s: about 26 s, 50 s and 39 s | About 2 min. The old stage's one build took 48.9 s here and 51.5 s there (run 42962881796) |
| Compressing the layers | 58 s, for 2.1 GB compressed | Not measured. The old image's took 27 s there |
| The whole build | 4 min 46 s | About 5 min to build and compress. With its pushes of the image and the two caches, and its checks, 7 to 9 min of its 30 |

The same build in the build job, when it has to make a candidate: about 4 minutes, since
it compresses and pushes nothing. That job then compiles from an empty cache, because its
Actions cache is keyed by the image. The last such run's steps after the
environment took 8 minutes (run 42931455806). So about 13 minutes of the job's 20.

The image grows from 3.45 GB to 8.95 GB on disk, and from 0.86 GB to 2.1 GB
compressed: 1.2 GB of fetched crates and 2.8 GB of compiled ones.

On this machine with all its cores, `python3 scripts/image_base.py build` took 2 min 5 s
with the environment's layers cached.

### What a push does

Read from the workflows; none was edited. Two workflows run on a push to `strata-next`.

- **`ci.yml`:** `Quality build and lint` and `Dependency and spelling policy`. The test
  shards, the aggregate gate and the scripts' tests run on pull requests, and QA on a
  manual dispatch.
- **`test-image.yml`,** the publisher, when the push changes `Cargo.toml`, `Cargo.lock`,
  `crates/*/Cargo.toml`, `tests/container/**`, `scripts/image_inputs.py`,
  `scripts/image_registry.py`, `.dockerignore` or the workflow.

By commit:

| Commit | The publisher | `Quality build and lint` |
| --- | --- | --- |
| The exit check's three, to `c7aa6dac2` | Does not run: no watched path changes | Resolves the base keyed `9229afdf…`, as now |
| `50124f6e2`, the seeding | Does not run | Resolves `9229afdf…`. The seeding finds no dependencies in that base and says so |
| `7c3d69d3f`, the image | Runs. The key becomes `4fa6c8cf…`. It finds none of its four references, builds the `dependencies` stage, pushes it as the build image and as `base-4fa6c8cf…-linux-amd64-1001-1001`, exports the two caches, then checks both references anonymously | **If it is the head commit of its push:** the registry has no base for the new key, `git diff HEAD^ HEAD -- tests/container` is not empty, so the job builds the candidate itself with `image_base.py build` and goes on, from an empty cache. **If a later commit is the head:** it fails in half a minute with "The unchanged environment is unavailable" |
| Anything after | Runs only for a watched path | Resolves `base-4fa6c8cf…` once the publisher has pushed it. Until then it fails as above |

**The order that keeps the green jobs green.**

1. Push the exit check's commits, to `c7aa6dac2`. Nothing about the image changes.
2. On the owner's word, push to `7c3d69d3f`, so that it is the head of that push. The
   seeding commit goes with it. Watch `Publish the pinned test image`, 7 to 9 minutes, and
   `CI`, about 13.
3. When the publisher is green, push the rest: this record's commit and whatever follows.

Pushing everything at once is safe but shows red: the build job of that push fails at
once, and passes on a re-run after the publisher is green.

**If the publisher fails.** No base exists for the new key, and every later push fails the
build job. `git revert 7c3d69d3f`, pushed as the head commit, puts `tests/container` back:
the key is `9229afdf…` again, which is published, and the build job resolves it. The
publisher then fails as it does today, which is where the branch started. The seeding
commit can stay: without the image's directory it copies nothing.

**If the build job runs out of its 20 minutes** building the candidate, that run is red
and nothing else is wrong. Re-run it once the publisher is green: it then pulls the base.

What could fail the publisher that could not be tried here: its pushes to the registry,
which are larger than before, and the registry's own limits. The image build itself ran
here from nothing, with only the network a runner has: the distribution's archive,
GNOME's sources, crates.io and Rune's public repository.

### Workflow changes this package did not make

Neither is needed for the publisher to pass. Both are the owner's.

1. **`test-image.yml` watches `crates/*/Cargo.toml`.** `strata-test-peer`'s manifest is
   deeper, so a change to it alone does not start the publisher, and the cache key's
   `hashFiles` does not see it either. The change: `crates/**/Cargo.toml` in `on.push.paths`
   and in the `hashFiles` call of "Restore trusted default-branch dependency layers". Until
   then such a change publishes when `Cargo.lock` changes with it, which it nearly always
   does.
2. **The base is tagged once for each image key.** The publisher pushes
   `base-<image key>` only when it does not exist, so the dependencies a run seeds from are
   those of the commit that changed the environment. As `Cargo.lock` moves on, a first run
   fetches what differs and says so. To refresh them with every dependency change, the
   `tags` of "Publish the build environment and its complete dependency cache" would list
   `steps.identity.outputs.environment_image` always, not only when it is missing. That
   makes the base tag move, which `docs/testing.md` says it does not; CI pins the digest
   it resolved, so a run stays consistent either way.

## Rulings recorded

All by the owner on 2026-11-11, and all now in the decision table of the
[README](README.md#owner-decisions):

| # | Ruling |
| --- | --- |
| 12 | The test image: fix the dependency stage so the publisher passes, and have the scripts seed their caches from it. Ruled twice; the second, on V7's three ways forward, chose the largest |
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
- **The image holds compiled crates for one cache only.** The ruling asked for the scripts
  to seed their caches from the image's dependencies. `target/qa-container` takes the
  fetched and the compiled crates. `target/quality-container` takes the fetched crates and
  compiles them: compiled crates are fresh only at the path they were built at, and the
  two caches have two paths. Building everything twice in the image was not done.
- **The seeding commit comes before the image commit.** Alone it changes nothing, and the
  image commit has to be the head of its push for the build job to stay green.
- **`.dockerignore` and `scripts/image_inputs.py` changed too,** beyond `tests/container/`:
  the first to let the deeper manifest in, the second to hash it and the stub script.
- **Nothing under `.github/` changed.** Two changes there are written up and left.

## Tests

In the runner (`tests/qa/parity_tests.rs`, listed in `tests/qa/unit_tests.rs`): two new
tests, 35 in all.

| Test | Shows |
| --- | --- |
| `the_ledger_allows_only_the_flaky_run_and_the_stale_patch_it_was_given` | The committed allowances stand on a real run and a real patch, a reason and a document that names them, and are those two |
| `an_allowance_for_nothing_real_or_with_nothing_behind_it_is_refused` | Eight ways an allowance is refused, and three malformed entries that do not parse |

In `scripts/`, 26 new tests, 269 in all. In `scripts/test_qa_parity.py`: six new tests and
one reworded, 15 in all.

| Test | Shows |
| --- | --- |
| `FlakyAllowanceTests` (3) | The ledger's names; only a flaky run is run again, and any other failed test, a runner test included, stands; the command prints the runs or exits 1, and a failure that names no test is not excused |
| `StalePatchTests` (1) | A stale patch is left out only while the ledger names it and it stays stale: three ways the selection refuses to start |
| `RepositoryTests` (2 new) | The committed ledger's stale patches are the ones that do not apply; the script asks the ledger with the runner's own failure line |

In `scripts/test_dependency_seed.py`, 11 tests, on directories made for the test:

| Test | Shows |
| --- | --- |
| `SeedTests` (7) | An empty cache takes the fetched and the compiled crates with their timestamps; a cache Cargo has used is left exactly as it was, whether it has `registry` or `git`; a cache at another path takes the sources alone; a target directory with builds keeps them; a lockfile ahead of the image is seeded and said loudly, as is an image with no digest; a base without dependencies leaves Cargo to fetch; a copy cut short is made again |
| `CommandTests` (2) | The command needs both of Cargo's directories; a copy that fails leaves an empty cache and exits 0 |
| `ScriptTests` (2) | Each of the four scripts seeds before it compiles; formatting and a shard copy nothing |

In `scripts/test_image_dependencies.py`, 9 tests:

| Test | Shows |
| --- | --- |
| `StubWorkspaceTests` (3) | Every target a manifest names or Cargo looks for gets a file, for the root package and for members at any depth, and a manifest in Cargo's own cache is no member; what the stubs built is found by Cargo's artifact names, and a third party's files are not; the command fails on leftovers and on a tree with no package |
| `RecipeTests` (6) | Every file the recipe copies from its directory is an image input; every image input is under `tests/container/`, where CI looks for a changed environment; the build context and the dependency key hold every workspace manifest, the deep one included; the stage builds what `qa.sh` and `qa-parity.sh` build, at the paths `scripts/lib/qa-container.sh` mounts; it never builds both lanes in one cargo run, and removes its stubs before it moves the result; the Cargo home is told not to clean itself |

`scripts/test_image_base.py` and `scripts/test_image_registry.py` each had one test
extended: the explicit build makes the `dependencies` stage, and the dependency key changes
with a manifest below `crates/*`.

## Done when, and the evidence

### The exit check

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
- **The same command at the last commit met the flake.** At `481392c68` it exits 1 in
  13 min 57 s (`qa-parity-slice.log`): the gates pass as above, the whole suite has 792
  passed and the flaky run failed, the rerun alone fails, and the command prints
  `ALLOWANCE REFUSED: new-folder-with-selection-groups/columns failed in the whole GTK
  suite and again alone.` The Rune patches (3 of 3) and the GTK patches (21 of 21, in 11.3
  minutes) are caught. Nothing else failed. The failed bundle is in
  `qa-parity/allowance/`.
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

### The image

All on 2026-11-11, at `7c3d69d3f`, with Docker, in the base built here by
`python3 scripts/image_base.py build`: key `4fa6c8cf…`, image `sha256:1a3fac7a…`, where the
old key was `9229afdf…`. Logs are in `~/dev/strata-next-verify/target/v9b-7c3d69d3f/`, with
the script that ran them in order, `validation-chain.sh`.

**From an empty cache, with no network.** In a scratch worktree at the same commit, each
command in a cache that did not exist, with every container started with `--network none`
by an engine wrapper (`docker-offline`, named by `STRATA_CONTAINER_ENGINE`):

| Command | What it did | Log |
| --- | --- | --- |
| `./scripts/qa.sh arrow-keys-move-focus/columns-arrows` | Copied the fetched and the compiled crates in 5 s, compiled Strata's seven crates and nothing else in 26 s, and the run passes. 32 s in all | `cold-qa-offline.log` |
| `./scripts/qa-parity.sh g1` | Copied both, compiled Strata's crates of both lanes, 13 s for `strata-rune` and 26 s for the runner, and the run passes on GTK and on Rune. 46 s in all | `cold-parity-g1-offline.log` |
| `./scripts/quality.sh clippy` | Copied the fetched crates in 3 s, said the compiled ones are for another cache, compiled everything from the copied sources, and is clean. 40 s in all, on 24 cores | `cold-quality-clippy-offline.log` |
| `./scripts/qa.sh` with `STRATA_TEST_CARGO_CACHE=target/qa-elsewhere`, as CI's QA job names another cache | Copied the fetched crates, compiled everything, and the run passes. 45 s | `cold-qa-other-cache-offline.log` |

No log has a `Downloading` or `Updating` line. Cargo could not have reached anything: the
containers had no network.

**A warm cache is reused, not reseeded.** The first command again, in the cache it had
just filled: `Dependencies: reusing the Cargo cache in /workspace/target/qa-container;
nothing is copied.`, nothing compiles, 1.4 s (`warm-qa-offline-again.log`). Every run
below in the verification worktree says the same of its caches, which V3 to V9 filled
from the network and which were not touched.

**A lockfile ahead of the image.** In the scratch worktree, `cargo update -p equivalent
--precise 1.0.3` moved one crate (`lock-ahead-update.log`), and the cache was emptied.

- With the network: the warning with both digests, the copy, then Cargo's `Downloaded
  equivalent v1.0.3`; 33 crates compile, the new one, what depends on it and Strata's, and
  the run passes in 33 s (`lock-ahead-online.log`).
- Without it: the same warning and copy, then Cargo fails on that one download, exit 101
  (`lock-ahead-offline.log`). Only the difference needs the network.

**The suites, in the new base, with the verification worktree's warm caches.**

- `./scripts/quality.sh fmt`, `clippy` and `test`: clean, clean, and 3,958 tests passed
  exactly once with 771 ignored (`quality-{fmt,clippy,test}.log`).
- `STRATA_QA_KEEP_ARTIFACTS=1 ./scripts/qa.sh`: 795 tests, 793 passed, none failed, 2
  ignored, in 79 s: 758 scenario runs and the runner's 35 (`qa-full.log`,
  `qa-full-artifacts/`).
- `./scripts/qa-mutation.sh` with the 21 patch names that apply
  (`qa-mutation-names.txt`): 21 of 21 caught, in 11.0 minutes (`qa-mutation.log`,
  `qa-mutation-summary.md`).
- `./scripts/qa-parity.sh slice`: exit 0 in 13 min 53 s. 36 of 36 required runs on GTK
  and on Rune, the whole suite with 793 passed, none failed and 2 ignored, 3 of 3 Rune
  patches and 21 of 21 GTK patches caught, and one allowance used, `cut.patch`'s
  (`qa-parity-slice.log`, `slice-qa-parity/`). This is the last passing run of the exit
  command.
- The scripts' tests: 269 pass.
- At this record's commit, which changes the document the ledger's test reads:
  `./scripts/qa.sh` once more, in `target/v9b-record/`. The runner's 35 tests pass, the
  ledger's among them, and 757 of the 758 scenario runs: the flaky run failed again, and
  `qa.sh`, which reads no allowance, exits 101.
- `./scripts/qa-drive.sh start`, one `send` and `stop`, from an empty cache with no
  network: the session starts in 30 s on the copied crates, and a key moves the focus
  (`cold-qa-drive-offline.log`).

**The build.** Its timings are under [What the build costs](#what-the-build-costs):
`image-build-4-cores.log`, `image-build-local.log` and `image-build-calibration.log`.

**The seed is what it should be.** In the built image: `/opt/strata-dependencies/cargo`
is 1.2 GB with 649 crates and Rune's repository, `build` is 2.8 GB, `lock.sha256` is the
checkout's `Cargo.lock`, `cache-path` is `/workspace/target/qa-container`, and `/workspace`
is empty. The stage's own check found nothing left by the stubs after `cargo clean`
removed 220 files.

**Code health.** CodeScene's local tool: `scripts/dependency_seed.py`,
`tests/container/stub-workspace.py`, `scripts/image_inputs.py` and the two new test files
score 10. `scripts/image_base.py` scores 9.68, and the tool's check of the change finds no
decline in it.

Not run, and why:

- A host QA run: no consent, and none needed.
- `mise run test-rune` and the compositor tests: no crate changed. The Rune lane's QA
  binary was built and run in the image, by `g1` and by `slice`.
- Podman: this host has Docker only. The seeding runs inside the container and does not
  know the engine; the two engines' options in `scripts/lib/qa-container.sh` did not change.
- The publisher and the build job: nothing was pushed, and no workflow can be run from
  here. What they will do is read from the workflows, [above](#what-a-push-does), and the
  image build was timed from nothing on four cores.
- A base older than a month: the Cargo setting that keeps a seeded home from being cleaned
  was tested on a copy of the seed with its last-use times set back, not by waiting.
- `cut.patch`: stale, as before.

## Open

- **The push,** in [the order above](#what-a-push-does), on the owner's word.
- **Two workflow changes** this package did not make,
  [above](#workflow-changes-this-package-did-not-make).
- **The quality cache compiles its own dependencies.** Holding them compiled in the image
  too means building them at `target/quality-container`'s paths as well: about as much
  again in size and build time.
- **Decision 27's condition,** as [above](#what-the-allowance-is-worth): the allowance is
  refused more often than it is used.
- **The `[[stale_patch]]` entry** is this package's reading of "as before": for the owner
  to confirm.
- **`cut.patch` is still stale** and `new-folder-with-selection-groups/columns` still
  fails. Both need a change on `main`.
- **`distant-archive-scrolled-into-view/icons`** has no allowance. It failed once, at V3,
  and not in any run of this package.
