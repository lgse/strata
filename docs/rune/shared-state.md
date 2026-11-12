# Shared state out of the GTK package

Four stores that only the GTK application can reach move to `crates/strata-desktop`, beside
`strata_desktop::settings`, so the Rune interface uses the same code: the per-folder views,
tab sessions, the key catalog and `PreferenceManager`. This is package B2 of the parity
plan, issue [lgse/strata#2125](https://github.com/lgse/strata/issues/2125).
[slice/V5b](slice/V5b-settings-store.md) moved the settings schema the same way and is the
method followed here, but for the rule on GLib below.

## The owner's ruling

`AGENTS.md` says not to move or reorganize `src/ui` on `strata-next`, because every move
makes the merges from `main` harder. On 2026-11-11 the owner ruled that the toolkit-free
parts of these four stores move all the same. The ruling names the stores and no file
beyond them: a file that is not part of one of the four moves only when a moved store
cannot compile without it, and the pull request that moves it says so.

Each store moves in its own pull request, and none changes what the GTK application does:
the files on disk keep their format and their place.

| Store | State | Shared part | What `src/ui` keeps |
| --- | --- | --- | --- |
| Per-folder views | The model moved. The file's path, reading and saving have not | `strata_desktop::settings::folder_views` | `src/ui/preferences/folder_view_settings.rs`, whole |
| Tab sessions | Not moved | None yet | `src/ui/tabs_session.rs` |
| Key catalog | Not moved | None yet | `src/ui/keys` |
| `PreferenceManager` | Not moved | The schema, from V5b | `src/ui/preferences.rs` and the files under it |

Until a row says a store moved, its code stays where `main` has it, and a change to it is
made as on `main`.

## What counts as toolkit-free

No crate under `crates/` may depend on `gtk4`, `gdk4`, `sourceview5` or `gstreamer`:
`deny.toml` bans them outside the application and `cargo deny check` enforces it. It has no
rule for GLib or GIO. `strata-desktop` already depends on both, `strata-fs` schedules on
GLib's default main context throughout, and `strata-platform`'s `glib_loop` runs that
context on Rune's loop. `strata-qa` is the one shared crate that stays free of GLib. A
part that names a widget, a window or a GDK key stays in `src/ui`. A part that needs only
a GLib timer or `glib::MainContext::default()` may move.

Three things qualify that:

- **It departs from V5b.** V5b's survey gave GLib as a reason to stay: the save queue
  ("GLib, for the save queue"), `folder_view_settings.rs` ("GLib timers") and the manager's
  timers and idles. That was a choice of scope for the slice, not a rule of the workspace,
  and this package does not follow it.
- **Depending on GLib is not permission to act from its dispatch.** Under Rune a callback
  that GLib's dispatch runs only queues, because Rune holds the application state then; the
  state changes in `poll` or in an update (`AGENTS.md`, "The Rune side"). The stores do
  more than queue today: the folder-view save timer writes and re-enters the manager through
  its thread-local, and three `spawn_local` blocks adopt what another process saved and
  publish to bindings. A shared part may keep the timer and the write. What it publishes
  must reach an interface as something that interface drains, not as a callback that
  changes state.
- **It would be the first scheduling in `strata-desktop`.** The crate uses GLib today for
  paths, key files and program lookup only, and `strata_desktop::settings` uses none. Tests
  of code that schedules need `strata_test_support::PrivateContext`.

## Per-folder views

`folder-views.toml` holds the sort and icon size a folder remembers
([preferences.md](../preferences.md#per-folder-sort-and-thumbnail-size)).

| Moved to `strata_desktop::settings::folder_views` | Holds |
| --- | --- |
| `FolderViews`, `FolderView`, `TakenViews` | The saved values, the use clock and its limit of 5,000 folders, and the unsaved changes a save merges over another process's. `FolderViews::parse` reads the file's text and `to_toml` writes it |
| `FolderKey`, `key_for_location` | How a folder is keyed: by absolute path, or by filesystem UUID and path inside a removable drive |

The code is the same apart from visibility and paths, with two additions. `FolderKey`'s
variants are `#[non_exhaustive]`, so another crate cannot build a key that skipped the
checks of its constructors; nothing outside the module ever built one. And
`key_for_location` says it belongs to the interface thread: it reads `strata-fs`'s
thread-local mount snapshot. An item is `pub` where the application names it and private
otherwise.

**The model does not find, read or write the file.** All of that stayed in
`src/ui/preferences/folder_view_settings.rs`:

- the file's name and `folder_views_path`;
- `FolderViewStore`: the read at start-up, the save timer, the monitors that follow what
  another process saved, and the session's values of trashed folders;
- `save_folder_views`: the lock, the merge over the saved file and the atomic write;
- the `PreferenceManager` methods that resolve a folder's sort, change the values and
  publish the change to bindings.

`folder_views_path` and `save_folder_views` use no GTK and no GLib, and stay all the same.
The rest of the file reads the manager's preferences, bindings and save notices, and a
file that stays whole keeps merging from `main` without a hand. They move with
`PreferenceManager`, into a new file.

`src/ui/preferences/trashed.rs` (`TrashedValues`) stayed too. Its users are
`folder_view_settings.rs` and `item_customizations.rs`, which both stay.

`src/ui/preferences.rs` imports `folder_views` at its old name. No other file of the
application changed.

The 13 tests of `folder_views/tests.rs` moved with the model. They run in
`strata-desktop`'s test binary as `settings::folder_views::tests::…`, in the GTK lane with
the other shared crates' tests.

## Merging from `main`

Git pairs a moved file with `main`'s file by similarity, and a change on `main` then lands
in the moved file. One rule keeps the pairing:

**A moved file takes no new code on `strata-next`,** only what crossing the crate boundary
needs: visibility, paths and a guard such as `FolderKey`'s. New shared code goes in a new
file beside it. A moved file that grows stops resembling `main`'s, and Git pairs something else
with it. Measured in the review of the first move: with `folder_view_settings.rs` moved
into `settings/folder_views.rs`, Git paired those two at 53% and reported `main`'s change to
`folder_views.rs` as a modify/delete conflict.

What a merge reports, and what to do:

| Git reports | Means | Do |
| --- | --- | --- |
| Nothing, and the change is in the moved file | The change was clear of every line the move touched | Build. A new item the application names needs `pub`, and a path that began `super::super::` or `crate::ui::` names something `strata-desktop` cannot reach |
| `CONFLICT (content)` in the moved file | The change is next to a line whose visibility changed. That is the usual case: 32 lines of `folder_views.rs` did | Take `main`'s lines, with `pub` where `main` has `pub(in crate::ui)` on an item the application names |
| `CONFLICT (file location)` | `main` added a file under a directory that moved, such as `src/ui/preferences/folder_views/`. Git placed it under the new directory | Accept the placement, declare its `mod` in the moved parent, and fix its paths |
| `CONFLICT (modify/delete)`, with `main`'s version left at the old path | Git did not pair the file | Apply `main`'s change to the moved file by hand and delete the file at the old path. Never keep it: no `mod` declares that path, so it builds and the change is silently dropped |
| Nothing, for a new file `main` added beside a moved file | Git moves a new file only with a directory that moved whole | If it belongs to the moved store, move it beside the store and declare it there |
| A conflict in `src/ui/preferences.rs` | `main` added a `mod` or an import beside the line a move replaced | Keep both: `main`'s line, and the import of the moved module from `strata_desktop::settings` |

Where each file is:

| On `main` | On `strata-next` |
| --- | --- |
| `src/ui/preferences/folder_views.rs`, `folder_views/keys.rs`, `folder_views/tests.rs` | The file of the same name under `crates/strata-desktop/src/settings/` |
| `mod folder_views;` in `src/ui/preferences.rs` | `use strata_desktop::settings::folder_views;` in the same file |
| `src/ui/preferences/folder_view_settings.rs`, `trashed.rs` | The same file as on `main` |

[slice/V5b](slice/V5b-settings-store.md#merging-from-main) has the table for the settings
schema.
