# Contributing to Strata

Thanks for helping build Strata. The project is early, so discuss large changes in an issue before investing in an implementation.

## Development setup

Install [mise](https://mise.jdx.dev), then install the pinned toolchain (Rust
1.98.1, Python, `cargo-deny`, `typos`, `cargo-watch`):

```bash
mise install
```

GTK 4, Fontconfig, a C toolchain, and `pkg-config` still come from the system
package manager. On Arch Linux:

```bash
sudo pacman -S --needed base-devel fontconfig gtk4 gtksourceview5 poppler-glib
```

Run the application:

```bash
mise run dev
```

To rebuild and restart the running application whenever code or bundled assets
change, use the development watcher. On Arch, Debian/Ubuntu, and Fedora, it
installs missing native GTK libraries (prompting for `sudo`) when needed:

```bash
mise run start-dev
```

Project coding-agent skills are listed in `skills-lock.json`. After cloning,
restore them into the gitignored `.agents/` directory (requires Node.js for
`npx`):

```bash
npx skills experimental_install
```

Commit `skills-lock.json` when adding or updating skills with `npx skills add`
or `npx skills update`. Do not commit `.agents/`.

## Branching and pull requests

All changes to `main` go through a pull request. The normal process is:

1. Create or select a GitHub issue and assign it before starting work.
2. Update local `main`, then create a branch from it.
3. Name the branch `<type>/<issue-number>-<short-kebab-description>`, such as `feat/6-sandbox-previews` or `fix/42-preview-timeout`.
4. Make focused Conventional Commits and push the branch.
5. Open a pull request that references the issue and wait for CI.
6. The maintainer tests the pull request before it is merged. Do not push directly to `main`.

Use the Conventional Commit form for commit messages and pull request titles:

```text
<type>(optional-scope): <imperative description>
```

Common types are `feat`, `fix`, `docs`, `refactor`, `test`, `perf`, `build`, `ci`, and `chore`. Examples:

```text
feat(preview): sandbox untrusted image parsing
fix(navigation): preserve selection after reload
```

Use `!` after the type or scope and add a `BREAKING CHANGE:` footer when a change is incompatible. Keep unrelated changes in separate commits.

Commits must be authored by the contributor submitting them. Do not submit
commits authored or co-authored by an AI coding agent. If an agent created
commits, remove them and recreate the changes and commits as your own work under
your own identity before opening or updating the pull request.

## Required checks

Before opening a pull request:

```bash
mise run check
```

Without mise, `./scripts/check.sh` runs the same Cargo checks.

The always-available checks are:

```bash
cargo fmt --all --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

CI additionally runs:

- `cargo-deny` for security advisories, dependency licenses, duplicate versions, and unapproved sources
- `typos` for spelling
- Compilation with the latest stable Rust release

`mise install` provides `cargo-deny` and `typos`. Without mise, install them with:

```bash
cargo install --locked cargo-deny
cargo install --locked typos-cli
```

## End-to-end GUI tests

`./scripts/check.sh` clears desktop display variables and skips display-dependent
Rust tests. Run `./scripts/test-headless.py` to include those tests on a private
Xvfb display without opening windows on your desktop. The end-to-end suite also
uses a private display and checks the real application and resulting files:

```bash
./scripts/e2e.sh
```

It needs Xvfb, `at-spi2-core`, the Python AT-SPI bindings, D-Bus, and
ImageMagick; the script names the packages when one is missing. See
[end-to-end GUI testing](docs/e2e-testing.md) for how scenarios are written,
how failure artifacts are collected, and how to regenerate the visual
baselines.

## Performance fixtures

Generate and profile deterministic large directories with:

```bash
./scripts/generate-fixture.sh target/fixtures
cargo build --release
STRATA_BINARY=target/release/strata ./scripts/profile-fixture.sh target/fixtures/100000
```

See [the performance baseline](docs/performance-baseline.md) for recorded results and measurement guidance.

## Engineering expectations

- Keep filesystem, search, preview, and operation work off the GTK thread.
- Make asynchronous work cancellable or reject stale results.
- Preserve native paths; do not assume filenames are valid UTF-8.
- Put product state and behavior outside widgets where practical.
- Add an abstraction only for real variation, isolation, or testability.
- Add focused tests for state transitions and filesystem edge cases.
- Avoid `unwrap`, `todo!`, `unimplemented!`, and debug macros in committed code.
- Follow the [unsafe code policy](docs/unsafe-code.md); never use `#[allow(unsafe_code)]`.
- Preserve licensing and attribution for every new asset and dependency.

See the [architecture principles](docs/architecture.md) before making structural changes.
Track planned work in GitHub issues rather than committed task checklists.

For user-visible PRs, upload sanitized before/after screenshots or a short video
through GitHub's description/comment editor and include the generated attachment
links in the Visual evidence section. Keep review captures and reports out of Git;
use local scratch or ignored `target/` paths while preparing them. The GitHub CLI
can embed existing URLs, but `gh pr create`/`gh pr edit` do not upload local media.
If an owner must complete the upload, explicitly mark the evidence as pending.
Maintained README media and visual-regression test baselines remain versioned.

## Asset policy

Only package assets Strata uses. Bundled icons use `strata-` names to avoid collisions, while their upstream origin is recorded in [third-party notices](THIRD_PARTY_LICENSES.md). Do not add generated placeholders or assets of unclear provenance.
