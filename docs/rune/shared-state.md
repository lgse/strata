# Shared state out of the GTK package

Four stores that only the GTK application could reach move to `crates/strata-desktop`,
beside `strata_desktop::settings`, so the Rune interface uses the same code: the
per-folder views, `PreferenceManager`, the key catalog and tab sessions. This is package B2
of the parity plan, issue [lgse/strata#2125](https://github.com/lgse/strata/issues/2125).
[slice/V5b](slice/V5b-settings-store.md) moved the settings schema the same way and is the
method followed here.

## The owner's ruling

`AGENTS.md` says not to move or reorganize `src/ui` on `strata-next`, because every move
makes the merges from `main` harder. On 2026-11-11 the owner ruled that these four stores
move all the same. The ruling covers the files named below. Any other move under `src/ui`
still needs asking.

Each store moves in its own pull request, and none changes what the GTK application does:
the files on disk keep their format and their place.

| Store | State | Now |
| --- | --- | --- |
| Per-folder views | The model moved | `strata_desktop::settings::folder_views` and `settings::trashed` |
| `PreferenceManager` | Not moved yet | `src/ui/preferences.rs` |
| Key catalog | Not moved yet | `src/ui/keys` |
| Tab sessions | Not moved yet | `src/ui/tabs_session.rs` |

## What counts as toolkit-free

No crate under `crates/` may depend on `gtk4`, `gdk4`, `sourceview5` or `gstreamer`, and
`cargo deny check` enforces it. GLib and GIO are not toolkits in that sense: every shared
crate depends on them, `strata-fs` schedules on GLib's main context throughout, and the Rune
interface runs GLib's loop beside its own. A store that needs a GLib timer or
`glib::MainContext::default()` can therefore move. One that names a widget, a window or a
GDK key cannot, and that part stays in `src/ui` as a wrapper.

## Per-folder views

`folder-views.toml` holds the sort and icon size a folder remembers
([preferences.md](../preferences.md#per-folder-sort-and-thumbnail-size)).

| Moved to `strata_desktop::settings` | Holds |
| --- | --- |
| `folder_views::FolderViews`, `FolderView`, `TakenViews` | The saved values, the use clock and its limit of 5,000 folders, the unsaved changes a save merges over another process's, and the reading and writing of the file |
| `folder_views::FolderKey`, `key_for_location` | How a folder is keyed: by absolute path, or by filesystem UUID and path inside a removable drive |
| `trashed::TrashedValues` | The values of trashed items, kept for the session. Folder colors and custom icons use it too |

The code is the same apart from visibility and paths. An item is `pub` where the
application names it and private otherwise.

What stayed is `src/ui/preferences/folder_view_settings.rs`: `FolderViewStore` and the
`PreferenceManager` methods that resolve a folder's sort, schedule the batched save, follow
what another process saved and publish the change. They read the manager's preferences, its
bindings and its save notices, so they move with `PreferenceManager`.

`src/ui/preferences.rs` imports the two modules at their old names, and no file outside
`src/ui/preferences` changed.

The 13 tests of `folder_views/tests.rs` moved with the model. They run in `strata-desktop`'s
test binary as `settings::folder_views::tests::…`, in the GTK lane with the other shared
crates' tests.

## Merging from `main`

A change on `main` to a file that moved whole is followed by Git's rename detection and
lands in the moved file. Check the result builds: a new item the application names needs
`pub`, and a path that began `super::super::` or `crate::ui::` names something
`strata-desktop` cannot reach.

| On `main` | On `strata-next` |
| --- | --- |
| `src/ui/preferences/folder_views.rs`, `folder_views/keys.rs`, `folder_views/tests.rs` | The file of the same name under `crates/strata-desktop/src/settings/` |
| `src/ui/preferences/trashed.rs` | `crates/strata-desktop/src/settings/trashed.rs` |
| `src/ui/preferences/folder_view_settings.rs` | The same file as on `main` |

[slice/V5b](slice/V5b-settings-store.md#merging-from-main) has the table for the settings
schema.
