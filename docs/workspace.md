# Workspace crates

Strata is one Cargo workspace. The root package `strata` is the GTK application. The code
that does not need GTK lives in crates under `crates/`, so a second interface can use it.
That second interface is being built on [Rune](https://github.com/anomalyco/rune), and its
crates live under `crates/` too. [The Rune side](#the-rune-side) describes them.

| Crate | Holds | Depends on |
| --- | --- | --- |
| `strata-core` | Model, localization, storage, I/O error text, logging, trusted commands, the UI-thread guard, file-activity tracking | `gio`, `glib` |
| `strata-preview` | Sandbox launcher, the preview helper it runs, document parsers, preview types | `strata-core` |
| `strata-fs` | Listing, file operations, trash, volumes, search, jobs, actions, the browser's state (`app`) | `strata-core`, `strata-preview` |
| `strata-desktop` | Portal and D-Bus setup, the updater, integrations, build information, the saved settings (`settings`) | `strata-core`, `strata-preview`, `strata-fs` |
| `strata-qa` | The `--qa` protocol (steps, targets, predicates and their parsers) and the engine that runs a session through its `Backend` trait | none |
| `strata-test-support` | Test helpers that need no toolkit. A dev-dependency | none |
| `strata` (root) | The GTK interface, assets, GTK's QA backend, diagnostics | all of the above |

None of the crates may depend on `gtk4`, `gdk4`, `sourceview5` or `gstreamer`.
`cargo deny check` enforces this and the layering: `deny.toml` lists, for each toolkit and
each crate, the only crates that may depend on it directly.

## The Rune side

Four crates and a test tool came from `wmfeht/strata-components` at `e4f2df1a`, at the
paths they had there. `strata-app` and `strata-rune` were written on this branch
([rune/slice/V5](rune/slice/V5-shell-and-loop.md)). The GTK application depends on none of
them.

| Crate | Holds | Depends on |
| --- | --- | --- |
| `strata-platform` | Linux shims for what Rune lacks: drag and drop, the clipboard, client-side decorations, foreign parenting, desktop settings, icon themes, single instance, the GLib bridge and the GLib loop. One cargo feature per shim | `rune` |
| `strata-kit` | Strata's look on rune-kit: theme mapping, icon catalogs, density, text size and the bundled font | `rune`, `rune-kit` |
| `strata-components` | The file-manager components: collections, views, chrome, previews and controls | `rune`, `rune-kit`, `strata-kit` |
| `strata-gallery` | One headless scene per component state, the golden tests and the integration demos | The three crates above |
| `strata-test-peer` | The second process of the compositor tests. It lives at `crates/strata-platform/tests/support/strata-test-peer` | none |
| `strata-app` | The Rune interface: its state and view, the queue `Browser`'s events wait in, start-up, and the offscreen and windowed hosts. Written here, not imported | The four crates above it but the gallery, and `strata-core`, `strata-fs`, `strata-desktop` |
| `strata-rune` | The `strata-rune` binary: `main`, and the window tests | `strata-app` |

- **Rune is pinned once.** `rune` and `rune-kit` name one git rev in the root manifest. Move
  them together, in a change of their own.
- **`strata-platform` depends on `rune` alone,** never on rune-kit, strata-kit or a shared
  crate, and its API carries no Strata names.
- **Components never depend on `strata-platform` or on a shared crate.** They take their data
  through accessors, so the gallery runs on generated data.
- **`strata-app` is the one crate that knows both sides.** It feeds the components from
  `Browser`, and only `strata-rune` uses it. At parity `src/main.rs` runs it and
  `strata-rune` is removed.
- **The shared crates and the application never depend on Rune or on a Rune-side crate.**
- **These crates inherit `version` and `rust-version` from `[workspace.package]`.** The shared
  crates set their own `0.0.0`.
- **Each has its own `clippy.toml`,** with the settings it was written under. Clippy reads
  the nearest file, so the application's rules in the root `clippy.toml` do not reach them.
  That includes the rule on private GLib contexts, which `strata-platform`'s GLib bridge
  would otherwise have to answer at `src/glib_bridge/thread.rs`.
- **The `media-audio` shim was not imported.** It plays PCM through GStreamer, which no crate
  under `crates/` may use. Its sources, example and tests remain in the old repository.

`deny.toml` holds these rules beside the ones for the shared crates. The plan and the work
packages the crates were built from are under [`docs/rune/`](rune/plan.md), and
[`docs/rune/testing.md`](rune/testing.md) covers their goldens, scripts and compositor tests.

## Two lanes

The application and the Rune side never build in one cargo run. `cargo check --workspace`
fails: Cargo gives a dependency the features of every package in the run, Rune turns on
`dlopen` in `yeslogic-fontconfig-sys`, and under that feature the fontconfig functions
`src/assets.rs` calls are not exported.

| Lane | Packages | Selected with |
| --- | --- | --- |
| GTK | The application and the shared crates | `$(python3 scripts/lanes.py gtk)`: `--workspace` and an `--exclude` for each Rune-side package |
| Rune | The Rune-side crates | `$(python3 scripts/lanes.py rune)`: a `--package` for each |

- **`scripts/lanes.py` is the one list.** `mise.toml`, `scripts/check.sh`,
  `scripts/quality.sh`, `scripts/quality_ci.py`, `scripts/test-headless.py` and `ci.yml` read
  it. `scripts/test_lanes.py` fails when a package that depends on Rune is missing from it.
- **`mise run compile` and `mise run clippy` run both lanes,** one after the other.
  `mise run test` runs the GTK lane and `mise run test-rune` the Rune lane.
- **The Rune test tasks build without debug info** (`CARGO_PROFILE_DEV_DEBUG=0`). Each of
  their 71 test binaries links Rune and its renderer. With debug info for every dependency a
  link took more than two gigabytes of memory, and Cargo ran 23 at once. The old repository
  turned debug info off for all dependencies in its dev profile; that setting was not
  carried over, because it would change the GTK application's debug builds too.
- **The pinned test image and CI run the GTK lane only.** The image has no Vulkan driver,
  which the Rune goldens need. Formatting, the dependency policy and spelling cover the
  whole workspace in CI. Compiling, linting and testing the Rune side is a local step until
  a CI job for it exists.
- **The GTK application builds with the features it had before the Rune side joined.** Its
  lane selects the packages it always did.
- **An editor that checks the whole workspace reports the fontconfig errors.** Point it at
  one lane's packages.

## Paths stay the same in the application

The root package re-exports every moved module at its old path. `crate::model`,
`crate::services::FileSource`, `crate::adapters::…` and `crate::app::Browser` still resolve
there. `src/services/mod.rs` and `src/util/mod.rs` are small facades that combine the
crates' modules with what stayed behind. `src/ui/preferences.rs` does the same for the
saved settings: the schema, its defaults and the reading of the file are
`strata_desktop::settings`, and `PreferenceManager` wraps them
([rune/slice/V5b](rune/slice/V5b-settings-store.md)).

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
5. If it depends on Rune or on a Rune-side crate, add it to `RUNE_SIDE` in
   `scripts/lanes.py` and give it the Rune side's `clippy.toml`.

For a shared crate the test image needs no change: its dependency stage stubs every crate
directly under `crates/` from its manifest. That stage does not load the workspace as it is
now. It copies no manifest for `strata-test-peer`, which sits deeper, and it deletes the
example sources `strata-gallery`'s manifest names. Until the stage handles both, the image
publisher fails on `strata-next`. The base image the quality and QA runs use is keyed by
`tests/container/` alone and is not affected.
