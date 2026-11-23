# Workspace crates

Strata is a single unified Cargo workspace. The root package `strata` (`src/main.rs`) is the
thin binary entrypoint; all shared logic and the [Rune](https://github.com/anomalyco/rune) UI
stack live in layered crates under `crates/`.

## Crate layering

| Crate | Holds | Depends on |
| --- | --- | --- |
| `strata-core` | Model, localization (`data/locales`), storage, I/O error text, logging, trusted commands, the UI-thread guard, file-activity tracking | `gio`, `glib` |
| `strata-preview` | Sandbox launcher, the preview helper it runs, document parsers, preview types | `strata-core` |
| `strata-fs` | Listing, file operations, trash, volumes, search, jobs, actions, the browser's state (`app`) | `strata-core`, `strata-preview` |
| `strata-desktop` | Portal and D-Bus setup, the updater, integrations, build information, saved settings and per-folder views (`settings`), tab sessions (`tabs_session`), and the key catalog with `KeyPress`, key labels, and `xkbcommon` layout resolution (`keys`) | `strata-core`, `strata-preview`, `strata-fs` |
| `strata-qa` | The `--qa` protocol (steps, targets, predicates and their parsers) and the engine that runs a session through its `Backend` trait | none |
| `strata-test-support` | Test helpers that need no UI toolkit (dev-dependency) | `glib` |
| `strata-platform` | Linux shims for what Rune does not yet ship: drag and drop, clipboard, client-side decorations, foreign parenting, Wayland backdrop blur, desktop settings, icon themes, zero-GStreamer Linux PCM audio (`dlopen` PulseAudio/PipeWire/ALSA), single instance, the GLib bridge, and the GLib loop. One cargo feature per shim | `rune` |
| `strata-kit` | Strata's look on `rune-kit`: theme mapping, Lucide and Catppuccin icon catalogs, density, text size, transparency, `overrides.toml` token overrides, and bundled JetBrains Mono font | `rune`, `rune-kit` |
| `strata-components` | Pure, accessor-driven file-manager UI components: collections, views (Columns, Icons, List), chrome, previews, dialogs, and controls | `rune`, `rune-kit`, `strata-kit` |
| `strata-gallery` | Component showcase scenes, golden tests, and integration demos | `rune`, `rune-kit`, `strata-kit`, `strata-components`, `strata-platform` |
| `strata-test-peer` | Second process for compositor session tests (`crates/strata-platform/tests/support/strata-test-peer`) | none |
| `strata-app` | The Strata application on Rune: shell, slots, parts, startup, settings UI, file chooser portal, sandboxed thumbnail worker (`--browser-worker`), windowed/offscreen hosts, and (behind `qa`) the `strata-qa` backend | `rune`, `rune-kit`, `rune-fx`, `strata-platform`, `strata-kit`, `strata-components`, `strata-core`, `strata-preview`, `strata-fs`, `strata-desktop`, and optionally `strata-qa` |
| `strata` (root) | The `strata` binary (`src/main.rs`), integration QA runner (`tests/qa.rs`), and windowed session tests (`tests/window.rs`) | `strata-app` |

`cargo deny --workspace check` (`deny.toml`) enforces the layering and bans `gtk4`, `gdk4`,
`gdk4-wayland`, `gdk4-x11`, `sourceview5`, `gstreamer`, and `gstreamer-app` workspace-wide:

- **Rune is pinned once.** `rune`, `rune-kit`, `rune-fx`, and Rune's `[patch.crates-io]`
  entries pin one git rev in the root `Cargo.toml`. Move them together in a dedicated change.
- **`strata-platform` depends on `rune` alone** (never on `rune-kit`, `strata-kit`, or any
  shared crate), carries no Strata names in its API, and gates each shim behind its own cargo
  feature (`mise run compile-features` verifies each feature in isolation).
- **`strata-components` never depends on `strata-platform` or a shared crate.** Components
  take their state through accessors and own no I/O or user-facing strings.
- **Shared crates (`strata-core`, `strata-preview`, `strata-fs`, `strata-desktop`) never
  depend on Rune or on any Rune-side crate.**
- **`strata-app` is the single crate that joins both sides**, and only the root `strata`
  package depends on it.
- **`strata-qa` stays free of any UI toolkit and of GLib.**
- **Per-crate `clippy.toml`:** `strata-platform`, `strata-kit`, `strata-components`, and
  `strata-gallery` each keep a crate-local `clippy.toml` where their lint rules differ from
  the root `clippy.toml` (for example, `strata-platform`'s GLib bridge owns a private GLib
  context intentionally).

## Parts of the Rune interface

`strata-app`'s window is a shell (`crates/strata-app/src/shell.rs`) with eight named slots:
tabs, location bar, sidebar, view, drawer, footer, overlays, and dialogs. Each slot is filled
by a part module implementing `slots::Part` and registered via `slots::parts!` in
`crates/strata-app/src/lib.rs`:

- Each registration line creates a field on the application state (`app.<part>`) that the
  part's components reach by accessor.
- Registration order defines z-order within overlay slots and the order in which parts receive
  queued `Browser` events.
- A slot with no part adds no element, no accessibility node, and no focus stop.
- The contract for every slot, lifecycle hook, timer, and keyboard route lives in the module
  documentation of `crates/strata-app/src/slots.rs`.
- `Browser` observers only push events into a queue because they fire inside GLib's dispatch
  while Rune owns application state; parts apply those events in `poll` or an update turn.

## Visibility

An item is `pub` only when another crate names it. Everything else stays `pub(crate)` or
narrower. `release_channel` in `strata-desktop` is private on purpose: it holds the
never-downgrade bypass, which the interface must not reach.

## Code that only tests use

A crate's `#[cfg(test)]` code is invisible to another crate's tests. Crates that expose test
helpers therefore provide a `test-support` feature, and the root package forwards its own
`test-support` feature through `strata-app`. Run workspace tests with `--all-features`.

- A test hook, test double, or helper used by another crate's tests is gated with
  `#[cfg(any(test, feature = "test-support"))]` and is `pub`.
- A helper used only by the crate's own tests stays `#[cfg(test)]`.
- A `tests` module is never compiled outside tests. Fixtures shared across crates live beside
  it (for example `fs_quirks` and `test_support` in `strata-fs`).

The QA runner builds `strata` with `--features qa,invariant-checks` and without
`test-support`, so the binary the scenarios drive carries no test hooks. Do not enable
`test-support` from the root package's dev-dependencies, or Cargo would unify it into the QA
binary as well.

## Code that only the QA mode uses

The scripted `--qa` mode is gated behind the `qa` feature (never enabled in release builds).
`strata-app`'s `qa` module implements `strata-qa`'s `Backend` trait, and `strata-core`'s `qa`
feature exposes `strata_core::logging::criticals` so QA replies can report GLib criticals.

## Translations

`strata-core` embeds all thirteen locale catalogs from `data/locales/`. Every crate that
calls `t!` declares `rust_i18n::i18n!` with `strata_core::i18n::Catalogs::compiled()` as its
backend so all crates share the same translation tables.

## Log targets

Modules log under their crate's name (for example `strata_preview::sandbox::browser` or
`strata_app::shell`). `with_extracted_crates()` in `crates/strata-core/src/logging.rs`
expands `strata::<module>` directives (such as `RUST_LOG=strata::shell=debug`) across every
crate registered in `EXTRACTED_CRATES` (`strata_app`, `strata_core`, `strata_desktop`,
`strata_fs`, `strata_preview`); bare `strata=debug` is not expanded, so target crate names
directly (for example `RUST_LOG=strata_app=debug`) for crate-wide filtering. Add new logging
crates to `EXTRACTED_CRATES`.

## Build facts

`strata-desktop`'s build script records the commit hash, release tag, build kind, and the
updater/integration compile-time switches, and embeds `integrations/`. It reads the
application version from the root `Cargo.toml`. `strata-kit`'s build script compiles the
bundled Lucide and Catppuccin SVG catalogs and theme palettes into Rust modules.

## Adding a crate

1. Create `crates/<name>/` with `edition`, `license`, `authors`, and `repository` inherited
   from `[workspace.package]`, `publish = false`, and `[lints] workspace = true`.
2. Add it to `[workspace.dependencies]` in the root `Cargo.toml`.
3. Add its layering rule to `deny.toml`, and add its name to `EXTRACTED_CRATES` in
   `crates/strata-core/src/logging.rs` if it logs.
4. Add a `test-support` feature if other crates' tests need its test-only helpers, and forward
   the root feature to it.

The pinned test image needs no change for a new crate: its dependency stage reads every
`Cargo.toml` under `crates/` and uses `tests/container/stub-workspace.py` to stub targets
automatically ([testing.md](testing.md#the-images-dependencies)).
