# V4 Extract the shared crates

| | |
| --- | --- |
| Status | Done |
| Depends on | V1 |
| Unblocks | V4b, V5 |
| Commits | `008d7e124` to `a16238f74` (cycles cut, direct `gio` and `glib` imports), `ad5d85231`, `7f389c536`, `c659ad46f`, `8f6594205`, `ffb6490eb` (the crates), `96e7593ca` (the layering check), `65eb2da03` (`docs/workspace.md`) |

## What was done

The slice plan asked for one `strata-core` holding the model, `app` with `Browser`, and
the services and adapters the slice reaches. The owner chose the workspace proposal's
layout instead, so the code went into four crates, with a fifth for test helpers.

| Crate | Holds |
| --- | --- |
| `strata-core` | Model, localization, storage, I/O error text, logging, trusted commands, the UI-thread guard, file-activity tracking |
| `strata-preview` | Sandbox launcher, the preview helper, document parsers, preview types |
| `strata-fs` | Listing, file operations, trash, volumes, search, jobs, actions, and `app` with `Browser` |
| `strata-desktop` | Portal and D-Bus setup, the updater, integrations, build information |
| `strata-test-support` | Test helpers that need no toolkit |

`docs/workspace.md` is the maintained description: the layering, how the root package
re-exports moved modules at their old paths, visibility, the `test-support` feature,
translations and log targets.

## Done when, and the evidence

- **No interface toolkit under the shared crates.** `cargo tree -p strata-fs -i gtk4`
  matches no package, and `deny.toml` bans `gtk4`, `gdk4`, `sourceview5` and `gstreamer`
  outside the root package. Checked again in [V0](V0-findings.md).
- **The GTK suite passes.** The full `./scripts/qa.sh` passed at `65eb2da03`: 758 runs.

## What it left for later packages

- **The preference store is still in the root package,** under `src/ui/preferences`. V5
  needs the part of it that reads `settings.toml`.
- **Start-up duties are still in `src/main.rs`:** starting the mount watcher before the UI
  thread is marked, and starting the stall watcher. V5 needs them from a shared crate.
- **`Browser` lends no entries.** Its listings sit behind a `RefCell`, and
  `with_column_entries` hands them out only inside a closure. V6 copies display fields or
  adds an accessor.
- **The key catalog and tab sessions** are still under `src/ui`. The slice does not need
  them.
