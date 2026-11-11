# V4b Import the component crates

| | |
| --- | --- |
| Status | Not started. Three owner decisions are open |
| Depends on | V4 (done) |
| Unblocks | V5 |
| Owns | `Cargo.toml`, `Cargo.lock`, `deny.toml`, `typos.toml`, `mise.toml`, `crates/strata-{platform,kit,components,gallery}`, `tests/golden`, `tests/scripts`, `docs/rune/plan.md`, `docs/rune/work` |
| Lands as | One mechanical commit for the copy, then one per policy file |

## Goal

Bring strata-components' four crates, their test data and their documents into the
workspace at the paths they already have, so the Rune side builds from one lockfile and one
Rune pin. No file under `src/` changes.

This package is not in the slice plan. It is stage 6 of the
[workspace migration](../workspace-migration.md), numbered here because V5 cannot start
without it.

## Decide first

| Decision | Options | Recommendation |
| --- | --- | --- |
| Which revision | `integration` at the day of the import, or a tagged one | The newest `integration` whose own CI is green. V0 used `e4f2df1a`, with Rune at `2a44156b` |
| History | Squash into one commit, or import with history | With history, as the workspace proposal recommends, so `git blame` reaches the change that wrote a line |
| `gstreamer` in `strata-platform` | Relax the `AGENTS.md` rule for `strata-platform`, or keep `media-audio` out of the import | Relax the rule for that one crate and name it in `deny.toml`: the feature is a platform shim, not an interface toolkit |

The owner has also to say when `wmfeht/strata-components` stops taking changes. After the
import, a component change is a change on `strata-next`, and the old repository is archived
with a pointer to the branch.

## What V0 already showed

The whole import was done once in a throwaway worktree. From
[V0's build probe](V0-findings.md):

- Both sides build in one workspace on Rust 1.99.1. Debug builds took 51 s for the gallery
  and the probe binary and 55 s for the GTK app.
- The lockfile grows from 448 to 669 packages. Two packages the GTK app uses move:
  `bitflags` 2.13.1 to 2.13.2 and `smallvec` 1.15.2 to 1.16.5. The GTK app's tree gains no
  Rune crate.
- 22 packages gain a second or third version, among them `zbus` 5 and 6 and `x11rb` 0.13
  and 0.14.
- The copied test data collides with no path Strata has.

## Scope

### The copy

| In strata-components | On `strata-next` | Note |
| --- | --- | --- |
| `crates/strata-platform`, `strata-kit`, `strata-components`, `strata-gallery` | Same paths | Includes the test peer at `crates/strata-platform/tests/support/strata-test-peer` |
| `tests/golden/`, `tests/scripts/`, `tests/fixtures/code/` | Same paths | |
| `tests/README.md` | Folded into `docs/testing.md` | |
| `docs/plan.md` | `docs/rune/plan.md` | Amended as the workspace migration lists |
| `docs/work/**` | `docs/rune/work/**` | Relative links to the plan keep working |
| `docs/decisions/`, `docs/upstream/`, `docs/testing.md` | `docs/rune/decisions/`, `docs/rune/upstream/`, a Rune section in `docs/testing.md` | Not in the proposal's table; they exist at `e4f2df1a` |
| `Cargo.lock` | Regenerated | Never hand-merged |
| `clippy.toml`, `rustfmt.toml`, `LICENSE`, `CLAUDE.md`, `CHANGELOG.md` | Dropped | Identical or superseded |

### The manifest

- `[workspace] members` gains `crates/strata-platform/tests/support/strata-test-peer`,
  which `crates/*` does not match.
- `[workspace.package]` gains `version = "0.0.1"` and `rust-version = "1.99.1"`, which the
  component manifests inherit. The alternative is to edit four manifests.
- `[workspace.dependencies]` gains 24 entries: `rune` and `rune-kit` at one git rev, the
  three component crates and the test peer by path, and `gstreamer`, `gstreamer-app`,
  `futures-util`, `tokio`, `zbus` 6, `wayland-backend`, `wayland-client`,
  `wayland-protocols`, `wayland-protocols-misc`, `wayland-protocols-wlr`, `x11rb` 0.13,
  `xkbcommon`, `png`, `swash`, `syntect`, `two-face`, `unicode-segmentation` and
  `unicode-width`.
- No existing shared entry changes. Strata's `toml`, `serde`, `gio`, `glib`, `libc`,
  `futures-channel` and `serde_json` already satisfy the component crates.
- `[workspace.lints.clippy]` gains `await_holding_lock` and `await_holding_refcell_ref`,
  the union of both lint sets.
- Each imported crate sets `publish = false`, as `docs/workspace.md` asks.
  `strata-platform` does not set it today.
- Carry over the component workspace's profiles if the perf tests are to run:
  `[profile.dev.package."*"] debug = false` and `[profile.perf]`. The first changes how
  Strata's dependencies build in debug, so ask before adding it.

### The dependency policy

`cargo deny check` passes after the copy, but only because it follows what the root package
reaches. Switch `mise.toml`'s `deny` task and the CI step to `cargo deny --workspace
check`, then make it pass:

| Failure with `--workspace` | Change in `deny.toml` |
| --- | --- |
| Six packages from a git source | `allow-git = ["https://github.com/anomalyco/rune"]` |
| BSL-1.0 on `clipboard-win` and `error-code` | Allow BSL-1.0, or an exception per crate. Both are Windows-only |
| RUSTSEC-2025-0141, `bincode` unmaintained | An `ignore` entry with its reason, once the dependent crate is named |
| `gstreamer` and `gstreamer-app` used by `strata-platform` | Per the decision above |

Add the layering rows for the Rune side, as the
[dependency rules](../workspace-migration.md#dependency-rules) state them: `rune` and
`rune-kit` wrapped only by the Rune-side crates, `strata-kit` only by `strata-components`,
`strata-gallery` and `strata-app`, and so on. The root package must reach none of them.

### The rest of the tooling

- **`typos.toml`:** exclude `tests/golden/**/*.png`, and merge what the component
  repository's `typos.toml` adds.
- **`mise.toml`:** the `bless` and `gallery` tasks, and a Rune test task scoped with `-p`.
  The Rune tests clear `DISPLAY` and `WAYLAND_SOCKET` and point `WAYLAND_DISPLAY` at a
  socket that does not exist.
- **SPDX headers.** Strata licenses every source file. Check the imported files and add
  the header where it is missing.
- **`docs/workspace.md`:** add the four crates to the table and their rules.
- **`AGENTS.md`:** add the component repository's "Crate rules" and "Rune" sections, the
  golden rules, and the two-branch rules. State the exception for work package documents.
- **`THIRD_PARTY_LICENSES.md`:** the fonts and icons `strata-kit` bundles need their
  notices. The full regeneration for the Rune binary waits for release preparation.

### Needs approval before it is done

The slice rules allow no new CI job and no image change without the owner's word.

- **A `rune` job in `ci.yml`:** format, check, Clippy and tests for the imported crates, as
  the component repository's `ci.yml` runs them. Its `platform.yml` runs compositor tests
  that need sway.
- **The test image.** Its dependency stage builds `--bin strata` only, so it caches no
  Rune-side crate. Without a change, a Rune build in the image fetches and builds about 220
  crates on every cold cache.
- **The CI dependency guard.** `ci.yml` fails a default build whose `cargo tree` holds
  `x11rb`, `wayland-client` or `xkbcommon`. If it reads only the root package's tree it
  keeps passing. Confirm that rather than assume it.

## Out of scope

- Any change to a component's behaviour. A gap found while importing is written down, not
  fixed here.
- `strata-app` and `strata-rune`. They start in V5.
- The Nix package, the release workflow and the AUR packages. They are allowed to break on
  the branch.

## Tests

| Check | Command | Expect |
| --- | --- | --- |
| The workspace builds | `cargo check --workspace --all-targets --all-features --locked` | Clean |
| The GTK app is untouched | `./scripts/quality.sh` and `./scripts/qa.sh`, in the pinned image | As before the import. The lockfile moved two of its packages, so this is a full run |
| The Rune side | The component repository's test task, headless, for the four crates | The results that revision had in its own repository |
| Goldens | The golden tests, with no blessing | No golden changes. If one does, find out why before blessing anything |
| Lint | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | Clean under Strata's `clippy.toml`. V0 did not check this |
| Policy | `cargo deny --workspace check`, `typos` | Clean |
| No Rune in the GTK app | `cargo tree -p strata -e normal` | No `rune` crate |

Whether the PNG goldens pass without a GPU is not known. The component repository blessed
them on an NVIDIA card and records the software renderer as pending.

## Done when

- The four crates build and their tests pass from the workspace root.
- The GTK suite and the Rust quality run pass in the pinned image.
- `cargo deny --workspace check` passes with the rules above.
- `wmfeht/strata-components` is archived, or the owner has said when it will be.

## Risks

- **The component work is not finished.** The proposal's gate was "components first".
  Importing now moves that work, and its reviews, onto `strata-next`.
- **Strata's lint settings may reject component code.** `clippy.toml` here differs from
  the one the crates were written under.
- **A second copy drifts.** Between choosing the revision and archiving the old
  repository, changes can land in both. Keep that window short.
