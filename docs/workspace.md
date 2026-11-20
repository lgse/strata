# Workspace crates

Strata is one Cargo workspace. The root package `strata` is the GTK application. The code
that does not need GTK lives in crates under `crates/`, so a second interface can use it.

| Crate | Holds | Depends on |
| --- | --- | --- |
| `strata-core` | Model, localization, storage, I/O error text, logging, trusted commands, the UI-thread guard, file-activity tracking | `gio`, `glib` |
| `strata-preview` | Sandbox launcher, the preview helper it runs, document parsers, preview types | `strata-core` |
| `strata-fs` | Listing, file operations, trash, volumes, search, jobs, actions, the browser's state (`app`) | `strata-core`, `strata-preview` |
| `strata-desktop` | Portal and D-Bus setup, the updater, integrations, build information | `strata-core`, `strata-fs` |
| `strata-qa` | The `--qa` protocol: steps, targets, predicates and their parsers | none |
| `strata-test-support` | Test helpers that need no toolkit. A dev-dependency | none |
| `strata` (root) | The GTK interface, assets, the QA backend, diagnostics | all of the above |

None of the crates may depend on `gtk4`, `gdk4`, `sourceview5` or `gstreamer`.
`cargo deny check` enforces this and the layering: `deny.toml` lists, for each toolkit and
each crate, the only crates that may depend on it directly.

## Paths stay the same in the application

The root package re-exports every moved module at its old path. `crate::model`,
`crate::services::FileSource`, `crate::adapters::…` and `crate::app::Browser` still resolve
there. `src/services/mod.rs` and `src/util/mod.rs` are small facades that combine the
crates' modules with what stayed behind.

Inside a crate, `crate::services` holds that crate's part of the services plus the parts
of the crates below it, so moved code kept its paths too.

## Visibility

An item is `pub` only when another crate names it. Everything else keeps `pub(crate)` or
narrower. `release_channel` in `strata-desktop` is private on purpose: it holds the
never-downgrade bypass, which the interface must not reach.

## Code that only tests use

A crate's `#[cfg(test)]` code is invisible to another crate's tests. Each crate therefore
has a `test-support` feature, and the root package forwards its own `test-support` feature
to them. `--all-features` turns it on; the application's unit tests refuse to build
without it.

- A test hook, a test double or a helper that **another crate's tests use** is gated with
  `#[cfg(any(test, feature = "test-support"))]` and is `pub`.
- A switch in production code that behaves differently under test reads the same gate, so
  the application's tests see the behaviour they saw before the split.
- A helper that only the crate's own tests use stays `#[cfg(test)]`.
- A `tests` module is never compiled outside tests. Fixtures that other crates need live
  beside it, for example `fs_quirks` and `test_support` in `strata-fs`.

The QA runner builds the application without `test-support`, so the binary the scenarios
drive carries no test hooks. Do not turn the feature on from the root package's
dev-dependencies: Cargo would then apply it to that binary as well.

## Translations

`strata-core` embeds the catalogs from `data/locales`. Every crate that calls `t!` declares
`rust_i18n::i18n!` with `strata_core::i18n::Catalogs::compiled()` as its backend, and they
all read the same tables.

## Log targets

A module logs under its crate's name, for example `strata_preview::sandbox::browser`.
`RUST_LOG=strata=debug` still covers every crate. A filter written against the old module
path, such as `strata::sandbox::browser=debug`, keeps working: the logger repeats each
`strata::module` directive for the crates in `EXTRACTED_CRATES` in
`crates/strata-core/src/logging.rs`. Add a new crate to that list.

## Build facts

`strata-desktop`'s build script sets the commit, release tag, build kind and the updater
and integration switches, and embeds `integrations/`. It reads the application version
from the root manifest, because the crate's own version says nothing about a release.
The root build script compiles the resource bundle.

## Adding a crate

1. Create `crates/<name>/` with `edition`, `license`, `authors` and `repository` inherited
   from the workspace, `publish = false` and `[lints] workspace = true`.
2. Add it to `[workspace.dependencies]` with `version = "0.0.0"` and its path.
3. Add its rule to `deny.toml`, and its name to `EXTRACTED_CRATES` if it logs.
4. Give it a `test-support` feature if other crates' tests need its test-only code, and
   forward the root feature to it.

The test image needs no change: its dependency stage stubs every crate from its manifest.
