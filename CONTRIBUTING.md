# Contributing to Strata

Thanks for helping build Strata. The project is early, so discuss large changes in an issue before investing in an implementation.

## Development setup

Install Rust, GTK4, Fontconfig, a C toolchain, and `pkg-config`. On Arch Linux:

```bash
sudo pacman -S --needed base-devel rust fontconfig gtk4 gtksourceview5 poppler-glib
```

Run the application:

```bash
cargo run
```

To rebuild and restart the running application whenever code or bundled assets
change, use the development watcher. On Arch, Debian/Ubuntu, and Fedora, it
installs missing native dependencies (prompting for `sudo`) and installs
`cargo-watch` automatically when needed:

```bash
make start-dev
```

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

## Required checks

Before opening a pull request:

```bash
./scripts/check.sh
```

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

Install the optional local tools with:

```bash
cargo install --locked cargo-deny
cargo install --locked typos-cli
```

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

See the [architecture principles](docs/architecture.md) and [work breakdown](docs/todo.md) before making structural changes.

## Asset policy

Only package assets Strata uses. Bundled icons use `strata-` names to avoid collisions, while their upstream origin is recorded in [third-party notices](THIRD_PARTY_LICENSES.md). Do not add generated placeholders or assets of unclear provenance.
