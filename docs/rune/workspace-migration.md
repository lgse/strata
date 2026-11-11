# One workspace for Strata and its Rune components

The migration runs on `strata-next`, a long-lived branch of `lgse/strata`. The branch is one
Cargo workspace holding the GTK app, the crates both interfaces share, and the Rune side.
`main` keeps shipping 1.x on GTK until the branch merges.

Source: the workspace proposal, revision 7 of 2026-11-09. Status below is as of 2026-11-10.

## Decided by the owner

- **One workspace.** One lockfile and one Rune pin.
- **GTK is not kept.** It is removed once the Rune interface reaches parity, so no effort
  goes into organizing GTK code.
- **The migration happens on `strata-next`.** `main` stays on GTK and keeps releasing 1.x,
  mostly bug fixes.
- **Tooling may break on the branch.** Packaging is repaired before the first Rune release.
- **The test image publisher may run for `strata-next`,** so the GTK QA scenarios keep
  running there.
- **The branch lands on `main` as a merge commit,** not a squash.
- **1.x ends at that merge,** with a last 1.x release just before.

## Where the work departs from the proposal

| Proposal | What happened | Ruled |
| --- | --- | --- |
| Finish strata-components first, and verify drag and drop and the file chooser by hand, before anything changes in Strata | The branch was started while strata-components was still in progress | Owner, 2026-11-10 |
| Import the component crates, then cut cycles and extract | The cycles were cut and the shared crates extracted first. The import follows | Owner, 2026-11-10 |
| Every migration change is a pull request to `strata-next` | Work lands as focused commits directly on `strata-next` until a vertical slice is approved. The branch then becomes an integration branch for pull requests | Owner, 2026-11-10 |
| Four shared crates and `strata-test-support` | The same, plus `strata-qa` from the slice plan | Owner, 2026-11-10 |
| `strata-core` has no GIO | `strata-core` depends on `gio` and `glib`. It has no interface toolkit | Not ruled; see `docs/workspace.md` |
| The browser's state sits in `strata-core` | `app` with `Browser` is in `strata-fs`, because it uses GLib sources and the services | Not ruled |
| Import the component crates with their history | The history was squashed into one commit, from `integration` at `e4f2df1a` | Owner, 2026-11-10 |
| All of `strata-platform` is imported | Its `media-audio` shim, the gallery's `media_playback` example and their tests were left out, because they use GStreamer | Owner, 2026-11-10 |
| One workspace, built as one | One workspace and one lockfile, built in two cargo runs. A run over the whole workspace does not compile; see `docs/workspace.md`, "Two lanes" | Not ruled |

## Two branches

| | `main` | `strata-next` |
| --- | --- | --- |
| Holds | The GTK app, one crate | The workspace: GTK app, shared crates, Rune side |
| Takes | Bug fixes, the occasional feature | All migration work, and `main` by merge |
| Releases | 1.x, through the existing workflow | None until it merges |
| Tooling | All of it working | May break |

Rules that keep them in step:

- **Fixes land on `main` first.** They reach `strata-next` by merge, never by a second
  hand-written copy. A fix that exists only on the branch is lost to 1.x users.
- **`main` merges into `strata-next` after every 1.x release and at least weekly.** Always a
  merge commit. The branch is never rebased.
- **All migration work stays on the branch,** including refactors that move no files.
- **Fix the parity target.** Record the commit where `main` stops taking features and move
  the feature map's watermark to it.
- **A feature that still lands on `main` opens a matching Rune issue in the same pull
  request.** Otherwise the parity bar moves without anyone deciding it.
- **A fix under `src/ui` says in its pull request whether the Rune app needs the same
  change.**

Merges should stay cheap because `src/ui`, `src/assets` and `src/qa` never move on the
branch. What needs a hand merge is a fix that touches code an extraction has rewritten.
One part of `src/ui` did move, by an owner ruling: the settings schema.
[slice/V5b](slice/V5b-settings-store.md) says where a change from `main` goes.

## Target layout on the branch

```text
strata/                   branch: strata-next
  Cargo.toml              [package] strata + [workspace]: members, shared deps, lints
  src/                    the GTK app: main.rs, ui/, assets, qa/. Deleted at parity.
  crates/
    strata-core/          shared; exists
    strata-preview/       shared; exists
    strata-fs/            shared; exists
    strata-desktop/       shared; exists
    strata-qa/            shared by both QA backends; exists
    strata-test-support/  dev-dependency only; exists
    strata-platform/      Rune side; imported, exists
    strata-kit/           Rune side; imported, exists
    strata-components/    Rune side; imported, exists
    strata-gallery/       Rune side; imported, exists
    strata-app/           Rune side; exists: the Rune interface
    strata-rune/          Rune side; exists: the second binary
  tests/
    qa/ qa.rs             GTK scenarios, unchanged until parity
    golden/ scripts/      Rune goldens and headless scripts, paths unchanged
  docs/
    rune/plan.md          the component plan, imported
    rune/work/**          the component work packages, imported
    rune/decisions/**     the component spikes and rulings, imported
    rune/upstream/**      what was asked of Rune upstream, imported
    rune/testing.md       how the Rune side is tested, imported
    rune/slice/**         the vertical slice
```

`docs/workspace.md` describes the crates that exist today and the conventions the split
produced.

## Dependency rules

`cargo deny check` enforces these through `deny.toml`. The rows for crates that do not exist
yet are added with them.

| Crate | May depend on | Never |
| --- | --- | --- |
| `strata-core` | Plain Rust crates, `gio`, `glib` | GTK, GDK, Rune |
| `strata-preview` | core | Either interface |
| `strata-fs` | core, preview, `gio`, `glib`, `rustix` | `gtk4`, Rune |
| `strata-desktop` | core, preview, fs, `zbus`, `ashpd` | Either interface |
| `strata-qa` | Plain Rust crates | Either interface, GLib |
| `strata-platform` | `rune` only | rune-kit, strata-kit, any Strata crate, Strata names in its API |
| `strata-kit` | `rune`, `rune-kit` | Shared crates |
| `strata-components` | `rune`, `rune-kit`, strata-kit | strata-platform, core, fs, preview, desktop |
| `strata-app` | Rune side and every shared crate | GTK |
| `strata` (root) | GTK and every shared crate | Rune and any Rune-side crate, until parity |
| `strata-rune` | `strata-app` | GTK |

Components never depend on the shared crates, on purpose. It lets the gallery run on
generated data. `strata-app` adapts `FileEntry` and `Location` to the component contract
types.

Two things the V0 checks found here, both settled at the import:

- `cargo deny check` follows only what the root package reaches. The check now runs with
  `--workspace`, in `mise.toml`, `scripts/check.sh` and CI.
- `strata-platform` depended on `gstreamer` behind its `media-audio` feature, which the
  `AGENTS.md` rule for crates under `crates/` forbids. The owner kept the rule, and the shim
  was not imported.

The import found a third. Cargo gives a dependency the features of every package in one
run, and the two sides disagree on `yeslogic-fontconfig-sys`: Rune turns on `dlopen`, under
which the functions `src/assets.rs` calls are not exported. So `cargo check --workspace`
fails, and every script selects one side's packages through `scripts/lanes.py`.

## What must keep working, and what may break

Each extraction claims to change no behaviour, and the GTK app's tests are the only evidence
for that claim. The GTK app is also the parity reference for the port.

Keep working at every step:

- **The workspace builds and its tests run on a developer machine.**
- **The GTK QA scenarios,** in the pinned test image.
- **The Rune lane:** goldens and headless scripts for the imported crates.
- **The dependency rules check.**

Allowed to break, and repaired before the first Rune release:

| Area | Why it breaks on the branch | Repaired by |
| --- | --- | --- |
| Nix package | The lockfile gains a git source for Rune, and members live in `./crates`, outside the package's fileset | Fileset and an output hash, or Rune from crates.io if it is published by then |
| Release workflow | It pins its own compiler and its signing environment is restricted to the default branch. It bundles notices and data for the GTK binary | New pin, notices and bundle contents for the Rune binary |
| AUR packages | They repackage the release binaries. Their dependency list names the GTK runtime | New dependency list |
| CI test shards | The shard plan and recorded durations assume one crate | A new plan over the final crate list |
| Test image publisher | Its dependency stage stubbed only the crates directly under `crates/`, and after the import it could not load the workspace. Fixed in the slice's [V9b](slice/V9b-test-image-and-exit-check.md): the stage stubs every member and holds both lanes' dependencies | Done, on the owner's ruling on decision 12 |
| Rune side in CI | CI and the pinned image build and test the GTK lane only. The image has no Vulkan driver, which the Rune goldens need | A `rune` job, and a software Vulkan driver in the image or a runner with one |
| Dependency guard | `ci.yml` forbids `x11rb`, `wayland-client` and `xkbcommon` in a default build's `cargo tree`. It reads the root package's tree only, so it still passes with the Rune side in the workspace | A rule written for the Rune binary |
| Licence policy | Done in the import: Rune's git source, BSL-1.0 for two Windows-only crates, and the `bincode` advisory are in `deny.toml` | |
| Third-party notices | About 200 new packages in the Rune binary | Regenerated notices |
| Docs and rule examples | Test filters change with each extraction | One pass over `AGENTS.md` and `docs/testing.md` |

**CI has to be repaired before the merge, not after.** `main`'s required checks gate the
pull request that lands the branch.

## How it lands

| Stage | Status |
| --- | --- |
| 1. Finish strata-components and verify drag and drop and the chooser by hand | Set aside as a gate. Still open in `wmfeht/strata-components` |
| 2. Create `strata-next` | Done. CI and the test image publisher run for it |
| 3. Turn the root manifest into a workspace and move to Rust 1.99.1 | Done |
| 4. Cut the cycles in place | Done |
| 5. Extract the shared crates | Done: core, preview, fs, desktop, test-support, qa |
| 6. Import the component crates | Done. [slice/V4b](slice/V4b-import-components.md) |
| 7. Close the old repository | `wmfeht/strata-components` is frozen at `e4f2df1a`: every component change is now a commit on `strata-next`. The owner archives it |
| 8. Start the Rune app | Started: `strata-app` and `strata-rune` exist ([slice/V5](slice/V5-shell-and-loop.md)). The vertical slice, [slice/README.md](slice/README.md) |
| 9. Parity: delete `src/ui`, `src/assets`, `src/qa`, the GTK scenarios and the GTK dependencies | Later |
| 10. Release preparation: repair everything in the table above | Later |
| 11. Merge to `main` and release | Later |

At parity `main.rs` runs `strata-app` and `strata-rune` is removed. `cargo tree` then shows
no `gtk4`, `gdk4*`, `sourceview5` or `gdk-pixbuf`. Release preparation also covers a test
image without GTK packages, the GTK rules in `clippy.toml` and `AGENTS.md`, a review of the
private media runtime patch kit, and a version that sorts above the last 1.x.

## What the split still owes

The proposal listed work the split had to include. Two items are not done, and one is:

- **Shared state under `src/ui`.** The settings schema and the reading of `settings.toml`
  moved to `strata_desktop::settings` ([slice/V5b](slice/V5b-settings-store.md)), by an
  owner ruling against the rule above on moving `src/ui`. `PreferenceManager` itself, the
  per-folder views, the key catalog and tab sessions are still in the root package.
- **The GLib main loop.** Done. `strata-fs` schedules on the default `MainContext`, and
  `strata-platform`'s `glib_loop` runs it on Rune's loop
  ([slice/V5](slice/V5-shell-and-loop.md)).
- **The portal chooser.** Its D-Bus service was to move to `strata-desktop`; its window is
  GTK and stays until the Rune chooser replaces it.

## Decisions on the details

- **`strata-platform` has no separate release.** It keeps the rules that make it
  extractable: `rune` only, no Strata names in its API, one cargo feature per shim.
- **Two binaries on the branch.** The GTK app and the Rune app build side by side until
  parity. No toolkit feature flag.
- **The import was squashed.** The proposal recommended importing with history, so that
  `git blame` would reach the change that wrote a line. The owner ruled for one commit on
  2026-11-10. The history stays in `wmfeht/strata-components`, and the commit hashes the
  imported documents cite are that repository's.
- **Work package documents stay in the tree as Markdown.**
- **The end state is just `strata`.** `strata-app` and `strata-rune` are names for the
  branch.

## Changes to the component plan

These were applied when the component plan was imported as `docs/rune/plan.md`; the plan
lists them under "Amended at the import into Strata", with three more the import produced.

| Where | Says today | Becomes |
| --- | --- | --- |
| §3 layout | A separate repository | Four crates in Strata's workspace on `strata-next`. The crate rules are unchanged |
| §3, decision 2 | `strata-platform` is a published crate with its own tags | A workspace crate with no separate release |
| S5 | Tag scheme and release workflow for `strata-platform` | Dropped. Filing the Rune issues remains |
| §7 | Releases come from the GTK branch | Unchanged: `main` is the GTK branch |
| X1 | A shell example inside the gallery | Unchanged. It is the starting point of `strata-app` |
| Rune rev bumps | A change in the component repository | A change on `strata-next` that can also move the toolchain and the test image |
| Strings | Not mentioned | Components carry no user-facing text. Labels and accessible names arrive as data, so Strata's thirteen catalogs stay the only source |

## Risks

- **Components meet real code late.** Whether the contract types fit Strata's listings at
  100,000 rows is known only once the adapter exists.
- **Strata moves while the components are built.** They target the feature map at v1.1.1.
- **The branch drifts from `main`.** The weekly merge is the control, and it needs a named
  owner.
- **A 1.x fix is lost in a merge.** Resolving a conflict in favour of the branch drops the
  fix without a trace.
- **Red CI hides real regressions.** Mark the jobs that are allowed to fail as such.
- **Release preparation is larger than it looks.** Keep one tracking issue listing each
  thing broken on purpose.
- **Rune is pre-release and takes no pull requests.** A breaking rev can pull the toolchain
  and the test image with it.
- **Nobody outside the team runs the Rune build until the merge.**

## Not verified

- How the test image's provenance check treats an image published from `strata-next`.
- Whether the release workflow could build `strata-next`.
- The Nix output-hash requirement against `package.nix`.
- Whether Rune's PNG goldens pass on a software GPU. They need a Vulkan adapter, the test
  image has none, and they were run only on the NVIDIA card they were blessed on.
- The first-stage hand checks. No drag or portal request was made.
