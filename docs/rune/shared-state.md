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
| Tab sessions | Moved, whole | `strata_desktop::tabs_session` | One import in `src/ui/mod.rs`. When to save and how to restore stay in `src/ui/window/composition/tabs.rs` |
| Key catalog | Not moved | None yet | `src/ui/keys` |
| `PreferenceManager` | The save bookkeeping moved. The manager, its getters and setters, its bindings and its notices have not | The schema, from V5b, and `strata_desktop::settings::Saving` | `src/ui/preferences.rs` and the files under it, with `persistence.rs` as the wrapper around `Saving` |

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

## Tab sessions

`tabs.toml`, in the configuration directory, holds the open tabs and which one is active
([preferences.md](../preferences.md#consumers-and-intentional-scopes)).

`src/ui/tabs_session.rs` and its 8 tests moved whole to `strata_desktop::tabs_session`. It
is the whole store: the file's stored form, `save`, `remove`, `load_restorable`, the limit
of 32 restored tabs, and which locations may be restored (absolute paths; a URI of a known
scheme without credentials; Trash and Recent only as their roots).

Four lines changed, the visibility of what the application names: `RestoredSession`, `save`,
`remove` and `load_restorable`. Nothing was added. The store names no GTK or GDK type and
schedules nothing: a save hands its bytes to the shared settings writer and drops the
receiver, so nothing runs from GLib's dispatch.

What stayed decides when and what. `src/ui/window/composition/tabs.rs` persists the tabs
when they change, removes the file while `restore_tabs` is off, and builds the GTK tabs
from a restored session. `PreferenceManager::set_restore_tabs` removes the file when the
setting is turned off. Neither file changed. `src/ui/mod.rs` imports the module at its old
name, so `crate::ui::tabs_session` still resolves.

An interface reads with `load_restorable()` and writes with `save(&locations, active)` and
`remove()`. None calls back.

- `load_restorable` reads the file where it is called, and blocks. It belongs to start-up,
  before the first window shows, which is the one place
  [preferences.md](../preferences.md#consumers-and-intentional-scopes) allows a read on the
  interface thread. It is not for `poll` or an update.
- `save` and `remove` only queue: they put the bytes on the settings writer's queue and
  return. Calling them from one thread, the interface's, keeps the saves in order.

One thing differs outside behaviour: the module's own log line, for a session that cannot
be serialized, is logged under `strata_desktop::tabs_session`. A filter written as
`strata::ui::tabs_session` no longer selects it, because the path inside the crate changed
too, and neither does a filter on a parent such as `strata::ui`
([workspace.md](../../workspace.md#log-targets)). A failed write is logged by the settings
writer, under `strata_core::storage::writer` as before.

### What the seam does not carry

The store holds locations and an index. The rules for using it are in GTK code that is
deleted at parity, and no QA scenario seeds or reads `tabs.toml` or sets `restore_tabs`, so
the parity ledger holds no interface to them. Their only tests are GTK's, in
`src/ui/window/composition/tabs/tests.rs`. Both interfaces write the same file, so a
difference reaches the user:

| Rule | Where GTK has it |
| --- | --- |
| Restore on a plain launch only: `restore_tabs` on, and no folder argument, reveal request or unlock flow | `src/ui/window/launch.rs:104`, `docs/preferences.md:137` |
| Only that window persists its tabs | `src/ui/window/launch.rs:98`, `src/ui/window/composition/tabs.rs:670` |
| No save while restoring, or a restore that stops half way replaces the session | `tabs.rs:670` |
| No save of an empty set. The store does not refuse one: it writes a session with no tabs, `load_restorable` then answers `None`, and the last good session is gone | `tabs.rs:690` |
| `remove()` in place of a save while `restore_tabs` is off, and when the setting is turned off | `tabs.rs:673-676`, `src/ui/preferences/file_settings.rs:114-119` |
| A restored local folder that no longer exists loses its tab, unless it is the only one, which opens the default folder | `tabs.rs:726-752` |
| The stored form is `main`'s to change. Any version but 1 is refused and the next save writes version 1 over it, and the 1.x releases read the same file. A field for per-tab state is a change on `main` first | `crates/strata-desktop/src/tabs_session.rs:108` |
| The writer is flushed at exit, or a save on its way to the file is lost. The windowed Rune host flushes; the offscreen host does not, which a restore scenario on the Rune target would need | `crates/strata-app/src/host/windowed.rs:90` |

The line numbers are as of the move.

## `PreferenceManager`

Three things make up the manager: the values (`Preferences`, shared since V5b), the
bookkeeping of saving them, and what GTK does around a change. The bookkeeping moved.

### What moved: `strata_desktop::settings::Saving`

`settings/saving.rs` is a new file. It owns no values and no `RefCell`: an interface holds
its `Preferences` however it holds state, changes them, and commits.

| | |
| --- | --- |
| `load() -> (Preferences, Saving)` | Reads the file, salvaged and brought within range, and starts the bookkeeping. What loading repaired is saved with the first change |
| `Saving::commit(&mut self, &Preferences) -> Commit` | Finds a change by comparing with the values of the commit before, and queues a save of the keys changed since a save last succeeded. The commit says whether anything changed, hands back the pending save, and names a problem: the file was unreadable at start-up, or the values could not be put in the file's form |
| `PendingSave::finished().await`, `Saving::saved(FinishedSave) -> SaveOutcome` | The interface awaits the save where it awaits things and tells the bookkeeping how it went: saved, failed, or superseded by a later save. A writer that went away is a cancelled write |
| `record_adopted`, `rebase` | Values another process saved and this one adopted are not saved again; a repair made at start-up is not a change |
| `revision`, `setting_unsaved`, `unsaved`, `load_failure`, the start-up locale and renderer | What an interface reads |

Nothing in it schedules or calls back, so `strata_desktop::settings` still uses no GLib main
context. Its bodies are the manager's, moved as text: `load`, `save_preferences`,
`queue_settings_save`, `settings_saved`, and the bookkeeping inside
`normalize_loaded_theme_selection` and `adopt_defaults`.

Two things it keeps exactly as GTK had them, because changing either changes what is written:

- A commit that changes nothing still queues a save while one is owed. That is the retry
  after a failed write, and it also means a save queued while another is on its way.
- A pending save that is dropped and never reported loses nothing on disk, but its keys stay
  marked and are written again with every later save. `Commit` is `#[must_use]`.

### What stayed in `src/ui`

- **`PreferenceManager`, with every getter and setter.** `preferences: RefCell<Preferences>`
  is as on `main`, and seven bookkeeping fields became one, `saving: RefCell<Saving>`. No
  setter file changed.
- **`persistence.rs`, the wrapper.** `save_preferences` records the change for the bindings,
  commits, spawns the await of the save on GLib's context, publishes to the bindings, and
  tells the notices, in the order it always did.
- **`bindings.rs` and `save_notice.rs`, whole.** `PreferenceChanges` keeps its own snapshot
  and revision: its notify loop re-runs on that revision and folder views bump it. The
  bookkeeping has its own baseline and revision, for an interface without
  `PreferenceChanges`; in GTK both see the same values at the same calls, and
  `save_preferences` asserts in debug builds that they agree.
- **The process-wide effects of loading:** reduced motion, the date format and the locale
  are applied by `PreferenceManager::load`, from what the shared `load` returned.
- **`shared()`, `existing()` and the thread-local.** The bookkeeping is not a singleton.

### The rule for a setter's body

A getter or setter that only reads or assigns a field has nothing to share: the field of
`Preferences` is the API. A body that validates or computes moves when a Rune package first
needs it, onto `Preferences` or a free function in `strata_desktop::settings`, as
`store_sort` did, in that package's own `contract` pull request, with the GTK setter calling
it. Until then it stays where `main` has it. Every range rule is already shared, in
`Preferences::normalize_loaded` and the normalizers beside it.

So this package closes with the manager's getters and setters still in `src/ui`. What moved
is its state bookkeeping and its persistence.

### Tests

`src/ui/preferences/tests` and `fixtures.rs` did not move. They drive the real path, setter
to commit to writer, through the manager, and are the saved-preferences coverage
[preferences.md](../preferences.md) describes. `settings/saving/tests.rs` adds seven tests of
what only the bookkeeping promises, each in a process of its own and with no GLib context:
only changed keys are written, an unreadable file is never saved over, a failed save is
tried again, only the latest save counts, a save nobody reports is written again, adopted
values are not saved back, and a start-up repair is no change.

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
| `CONFLICT (content)` in the moved file | The change is next to a line whose visibility changed. That is the usual case: 32 lines of `folder_views.rs` did, and 4 of `tabs_session.rs` | Take `main`'s lines, with `pub` where `main` has `pub(in crate::ui)` (the per-folder views) or `pub(crate)` (the tab session) on an item the application names |
| Nothing, and the application fails to build on an item of `tabs_session` | `main` added a `pub(crate)` item. It builds in `strata-desktop`, where `pub(crate)` is still valid, and fails where the application names it, not in the moved file | Make the item `pub` in the moved file |
| `CONFLICT (file location)` | `main` added a file under a directory that moved, such as `src/ui/preferences/folder_views/`. Git placed it under the new directory | Accept the placement, declare its `mod` in the moved parent, and fix its paths |
| `CONFLICT (modify/delete)`, with `main`'s version left at the old path | Git did not pair the file | Apply `main`'s change to the moved file by hand and delete the file at the old path. Never keep it: no `mod` declares that path, so it builds and the change is silently dropped |
| Nothing, for a new file `main` added beside a moved file | Git moves a new file only with a directory that moved whole | If it belongs to the moved store, move it beside the store and declare it there |
| A conflict in `src/ui/preferences.rs` or `src/ui/mod.rs` | `main` added a `mod` or an import beside the line a move replaced | Keep both: `main`'s line, and the import of the moved module from `strata_desktop` |

Where each file is:

| On `main` | On `strata-next` |
| --- | --- |
| `src/ui/preferences/folder_views.rs`, `folder_views/keys.rs`, `folder_views/tests.rs` | The file of the same name under `crates/strata-desktop/src/settings/` |
| `mod folder_views;` in `src/ui/preferences.rs` | `use strata_desktop::settings::folder_views;` in the same file |
| `src/ui/preferences/folder_view_settings.rs`, `trashed.rs` | The same file as on `main` |
| `src/ui/tabs_session.rs`, `tabs_session/tests.rs` | The file of the same name under `crates/strata-desktop/src/` |
| `mod tabs_session;` in `src/ui/mod.rs` | `use strata_desktop::tabs_session;` in the same file |
| A getter or setter of `PreferenceManager`, a binding, a notice, a test or the fixture | The same file as on `main`: none of them moved or changed |
| `PreferenceManager`'s fields and `load` in `src/ui/preferences.rs`; `src/ui/preferences/persistence.rs`; the bookkeeping in `adopt_defaults` (`folder_view_settings.rs`) and in `normalize_loaded_theme_selection` | Rewritten around `Saving`. A change from `main` conflicts there and is applied by hand: to `crates/strata-desktop/src/settings/saving.rs` when it changes what is recorded or saved, to the wrapper when it changes what GTK does around it |
| A new process-wide effect of loading | `PreferenceManager::load` in `src/ui/preferences.rs`, after the shared `load` |

[slice/V5b](slice/V5b-settings-store.md#merging-from-main) has the table for the settings
schema.
