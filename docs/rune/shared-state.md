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
| Per-folder views | The model moved, and the file code: its path, the read at start-up and the save. The store has not: when a save is scheduled, the monitors, the notices | `strata_desktop::settings::folder_views` and `settings::folder_view_file` | `src/ui/preferences/folder_view_settings.rs`: `FolderViewStore` and the manager's methods |
| Tab sessions | Moved, whole | `strata_desktop::tabs_session` | One import in `src/ui/mod.rs`. When to save and how to restore stay in `src/ui/window/composition/tabs.rs` |
| Key catalog | Moved, behind a backend seam: the key and modifier types, the catalog, the command registry and `Keymap`, `KeyPress` and its normalizer, the alias rewrite and the labels. Reading a GDK event, GTK's own effect for an alias, capture, composition and the conflict check have not | `strata_desktop::keys` | `src/ui/keys/`: the reader and the old names (`mod.rs`), the GDK backend, and `alias.rs`, `capture.rs`, `composition.rs` and `conflicts.rs` with every test |
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

**The model does not find, read or write the file.** That is the file code, which moved
after it, as text, into a new file beside the model:

| Moved to `strata_desktop::settings` (`settings/folder_view_file.rs`) | Holds |
| --- | --- |
| `folder_views_path` | Where the file is, with its name |
| `load_folder_views` | The read at start-up: the read half of `FolderViewStore::load`. It gives the values, and the error when the file was there and could not be read |
| `save_folder_views`, `FolderViewsSave` | The save: the lock (waited for at exit, not otherwise), the merge over what another process wrote, the atomic write, and the result: busy, saved with what the other process had, or failed |

They use `strata-core` and the model, and no GTK or GLib, and nothing in the file
schedules. They are the store's tools, not an interface's:

- **Nothing but the GTK application's `FolderViewStore` calls them until the shared store
  lands.** `save_folder_views` merges the caller's unsaved changes over the file, so two
  writers in one process would each save without the other's.
- **`load_folder_views` is called once in a process,** at start-up, by whoever owns the
  store. It notes the file's stamp with the settings writer, which is how a later re-read
  tells another process's write from this one's. A second call notes the stamp again, and a
  change another process made before it is never adopted.
- **A part of the Rune interface waits for the store** and does not read
  `FolderViews::view` itself. On GTK a folder's saved view is read behind
  `remembered_key`, which answers nothing while `remember_folder_views` is off, and behind
  the key rules of `key_for_location`. The store carries that gate; the model does not.

What stayed in `src/ui/preferences/folder_view_settings.rs` is the store:

- `FolderViewStore`: the values in memory, the save timer, the two monitors that follow
  what another process saved, and the session's values of trashed folders;
- the two delays, and `schedule_folder_views_write` with its timer;
- the `PreferenceManager` methods that resolve a folder's sort, change the values, start a
  write and take its result, adopt what a monitor found, publish the change to bindings and
  show the notices.

Three log lines moved with the file code and are now logged under
`strata_desktop::settings::folder_view_file`: the two warnings of a file with invalid
entries and of a file that cannot be loaded, and the debug line of a save made unlocked.
`RUST_LOG=strata=…` still covers them; a filter on `strata::ui::preferences` does not
([workspace.md](../workspace.md#log-targets)).

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

## The key catalog

Which key does what, and how a press is matched and labelled
([keyboard-shortcuts.md](../keyboard-shortcuts.md)). It moved to `strata_desktop::keys` so
that the Rune interface matches the same catalog through the same tables.

| Moved to `crates/strata-desktop/src/` | Holds |
| --- | --- |
| `keys.rs`, from `src/ui/keys.rs` | `Binding`, `KeyPress` and what a handler asks of it, `Match`, `normalize` with its tables (the modifier keys, the keypad, the digit row, US QWERTY), and the lists of direct, text-editing and custom-action commands. The whole file but the reader |
| `keys/bindings.rs` and the eight files under `keys/bindings/` | The catalog: every shortcut as a named constant |
| `keys/command.rs` | `Command`, `Map`, `registry()`, `Keymap` with reading and writing a saved entry, and `active_keymap()` |
| `keys/label.rs` | `Keys`, `Shown`, `Digits`, `Letters`, `label` and the functions beside it |

The moved files changed in visibility, in paths, and where they called GDK or the
preference manager, which are the three seams below. Five new files hold what is new:

| New in `crates/strata-desktop/src/keys/` | Holds |
| --- | --- |
| `key.rs`, `key_constants.rs` | `Key` and `Modifiers`, and the keys Strata names in code |
| `backend.rs` | `Backend`, `NeverFires` and `install` |
| `active.rs` | `Active`, `Fixed` and `set_active` |
| `alias.rs` | `rewrite` and `Context`: the half of `src/ui/keys/alias.rs` that touches no widget, moved as text |
| `parts.rs` | `KeyPress::from_parts`, for an interface whose events are not GDK's |

### Keys and modifiers

`Key` is an XKB keysym, which is the number `gdk::Key` holds, and its constants have GDK's
names (`Key::Return`, `Key::_1`, `Key::KP_Enter`). `Modifiers` has GDK's names and bits. So
the catalog reads as it did, and the application converts without a table:
`ui::keys::gdk_key`, `key_from_gdk`, `gdk_modifiers` and `modifiers_from_gdk`.
`modifiers_from_gdk` drops the pointer-button bits of a GDK state, which every reader of a
press already masked off.

**A key Strata names in code is a constant in `keys/key_constants.rs`,** and that is where
a new one goes: a key a catalog entry binds, a key a table of the normalizer names, or a
named key an interface delivers, whether or not the catalog binds it. The name is X's
keysym name, with a leading underscore where it starts with a digit, and the value is the
one in `xkbcommon-keysyms.h`. While the GTK application exists, its test
`ui::keys::tests::constants` holds every constant to GDK's key of the same name. The list
is a macro so that the test, and a backend, can walk it.

So while GTK exists a constant must be a name GDK reads, and the test fails for one that
is not. A named key the Rune interface delivers and GDK does not name is not added
quietly: on GTK a binding to it would be saved as a number, which is not read back, so
the two interfaces would save it differently. The package that needs the key says so in
its file and in [`parity/owner.md`](parity/owner.md); if the constant is then added, the
test lists it as a name GDK does not read, with the reason. The list is empty today.

### The key library is the interface's: `Backend`

Six questions have no answer without a key library: a key's name, the key of a name, the
character a key types, the key of a character, and a key's lower and upper case. Two more
are GTK's accelerator strings, which `settings.toml` saves: reading one and writing one.
`Backend` is those eight methods. An interface installs its own once, at start-up, with
`keys::install`, and the shared code asks it.

- **The GTK application installs GDK** (`src/ui/keys/gdk_backend.rs`): each method is one
  GDK or GTK call, the call the moved code made before. So what the application parses,
  writes, labels and matches is unchanged by construction, and GTK stays an oracle that a
  mistake in another backend cannot reach. `main` installs it first. A test binary installs
  it as it loads, from `.init_array`, because a `#[test]` has no start-up and one that
  forgot would fail only for what it happened to read.
- **The Rune interface installs `keys::Table`,** a backend in `strata-desktop` that needs
  no library (`keys/table.rs`). `strata-app` installs it when the UI thread is claimed,
  which every host and every hosted test does, and refuses the claim if another library
  was installed first. What the table answers, and what it does not, is below.
- A lookup with no backend installed panics, with a message that says so. That is why the
  tests of the moved code stayed in the GTK lane: they ask GDK.
- The seam is one process-wide value, and it goes when GTK does. `install` answers whether
  its call was the one that installed.

**What `Table` answers.** It is the first piece of the Rune side's key library, and the
table the `xkbcommon` backend will sit behind, so that a key the code declares reads and
writes the same whatever library a user has installed:

| | The table | Not yet |
| --- | --- | --- |
| Names | Every declared constant, read by its own name. Three keys are written by the other X name GDK writes them by, and read by both: `dead_perispomeni` for `dead_tilde`, `Arabic_switch` for `Mode_switch`, `KP_Next` for `KP_Page_Down`. `VoidSymbol` has no name, and its name reads as no key | Any key the code does not declare has no name, where GDK names it or writes a number |
| Characters | A printable Latin-1 character is its own key, and any other character its Unicode keysym (`0x0100_0000` plus the code point), as the X protocol defines both. The eighteen keypad keys that type a character (the digits, space, `=`, `*`, `+`, `,`, `-`, `.`, `/`) type it, so a keypad digit's text is its digit, as on GTK | No other named key types anything, such as a Cyrillic or Greek key given by its legacy keysym. So `normalize` and `KeyPress::from_signal`, which read a non-Latin key's character to find its Latin fallback, find none: an interface on the table builds its presses with `from_parts` and gives what was typed as `Typed::Character` |
| Case | The ASCII letters | Every other key is its own lower and upper case |
| Accelerator strings | None is read: `Binding::parse` answers `None`, so `Keymap` reports a saved entry as one it cannot read and the shipped keys apply | `Binding::accelerator` panics, because a made-up string would be saved over a user's binding. Reading and writing arrive with the `xkbcommon` backend's reader and writer |

With it installed, every catalog entry matches and is labelled as on GTK: the comparison
made for the move, run again with the table, gave the same keys and the same labels in both
maps and all thirteen languages. `strata-desktop`'s own tests hold the table without GDK,
and the GTK application's `ui::keys::tests::table` holds every answer in the left column to
GDK's. That test ties the three written names to the GDK of the test image: a GTK that
writes one of those keys by another name fails it, and the table follows the image.

`install` takes a second answer that is also the interface's own: `NeverFires`, whether a
binding in a map's entry can ever fire there, which `label` uses to leave such a binding
out. The GTK application's answer is `ui::keys::conflicts::never_fires`, from its own key
stages. The Rune interface answers that every binding fires. For the shipped keys the two
agree: GTK's check leaves out none of them, in either map, with type-to-search on or off.
They differ only for a key a user added, which the Rune side does not read yet
([`parity/owner.md`](parity/owner.md#b2-shared-state-out-of-the-gtk-package)).

### The settings that are read at each press: `Active`

`Map::active()`, `active_keymap()` and the labels read three live values: whether 10xer
mode is on, whether typing starts a search, and the keymap. On `main` they call
`PreferenceManager::existing()`. Here they ask the `Active` an interface registered for its
thread with `keys::set_active`, and fall back to the shipped defaults when none is
registered or it has gone, as they did when no manager existed.

`PreferenceManager` implements `Active`, and `PreferenceManager::shared()` registers a weak
reference to the manager it makes, so the values are read at the same moments from the same
manager. An interface that holds its values some other way registers its own, and `Fixed`
is one that holds them as plain fields.

### What an interface does with a press

1. Build the press: `KeyPress::from_parts(KeyPressParts { … })`, with the key, the modifiers
   as a binding sees them, the modifiers held, what the press typed, the digit position
   and whether the key came from a fallback. The GTK application's `normalize` derives the
   same from a GDK event, and `ui::keys::tests::parts` holds the two to the same press.
2. Read it through the aliases: `rewrite(press, Context { typing })`, where `typing` says
   whether what has focus takes text. A release is not rewritten.
3. Match it: `press.is(COPY)`, `press.digit_in(TAB_NUMBERS)`, and the rest of `KeyPress`.

`KeyPress::unclaimed_alias()` gives the canonical key of an alias nothing claimed, for an
interface that then wants its toolkit's own effect for that key, as GTK does.

### What stayed in `src/ui/keys/`

- **`mod.rs`, the reader.** `on_press`, `on_release` and `read_event`, with the two raw key
  signals and the read of a `gdk::KeyEvent`, are the text that was in `src/ui/keys.rs`.
  It re-exports every shared name at the path it always had, so `crate::ui::keys::Binding`,
  `ui::keys::bindings::COPY` and `keys::label` resolve as before, and it implements
  `Active` for the manager. No file is left at `src/ui/keys.rs`.
- **`alias.rs`, the GTK half:** what owns typing where a press lands, read from the focused
  widget, and GTK's own effect for an alias nothing claimed.
- **`capture.rs` and `composition.rs`,** unchanged.
- **`conflicts.rs`,** with three import lines changed. It knows which keys GTK's widgets
  and input methods take, so it is GTK's. What the Rune settings report as a conflict is an
  open question for the owner (lgse/strata#2133, lgse/strata#2150).
- **`PreferenceManager`'s keymap methods** (`src/ui/preferences/keymap.rs`), unchanged:
  they use the conflict check.
- **Every test.** `src/ui/keys/command/tests.rs` kept its path, through a test-only module
  in `mod.rs`, so its test names and the names its GTK tests re-run themselves by did not
  change.

Outside `ui::keys`, a file that named `gdk::Key` or `gdk::ModifierType` for a binding or a
press now names `ui::keys::Key` and `ui::keys::Modifiers`: an import line in most, and a
conversion where a key goes to GTK, as in `focus_navigation::activate_native_binding`. No
handler changed.

Nothing in the moved code logs, so no log target changed.

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
| `record_adopted`, `rebase` | Values another process saved and this one adopted are not saved again; a repair made at start-up is not a change. Each is safe only as GTK calls it: `record_adopted` for keys the caller found not unsaved, and `rebase` at start-up, before the first commit |
| `revision`, `setting_unsaved`, `unsaved`, `load_failure`, the start-up locale and renderer | What an interface reads |

Nothing in it schedules or calls back, so `strata_desktop::settings` still uses no GLib main
context. `load` applies nothing process-wide: the caller sets the locale from
`Saving::startup_locale`. Its bodies are the manager's, moved as text: `load`, `save_preferences`,
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
- **`shared()`, `existing()` and the thread-local.** `Saving` keeps no instance of itself,
  and there must be exactly one for the file in a process. The settings writer coalesces
  the saves queued for a path and gives every waiting save the result of the newest
  contents, so a second `Saving` would be told its save succeeded and forget keys that were
  never written. GTK's one is the shared manager's. The Rune interface holds one for the
  process, and whether a second Rune window shares it is an open question for the owner.

### The rule for a setter's body

A getter or setter that only reads or assigns a field has nothing to share: the field of
`Preferences` is the API. A body that validates or computes moves when a Rune package first
needs it, onto `Preferences` or a free function in `strata_desktop::settings`, as
`store_sort` did, in that package's own `contract` pull request, with the GTK setter calling
it. Until then it stays where `main` has it. Every range rule is already shared, in
`Preferences::normalize_loaded` and the normalizers beside it.

So this package closes with the manager's getters and setters still in `src/ui`. What moved
is its state bookkeeping and its persistence.

### The Rune interface

`strata-app`'s `settings::Saved` holds a `Preferences` and a `Saving`. It used to keep its
own queued table and its own set of changed keys, which never emptied.

- **A part writes** with `shell::saved(app).change(|settings| …)`, from a handler or a queued
  update. The values change, the commit queues the save, and the end of the save is awaited
  on GLib's context, where it only queues: the bookkeeping hears the result when GLib is
  next pumped and the queue applied. After a change made in a handler that is the `poll`
  that follows. After one made in a queued update, which is how the shell saves the sort and
  the hidden-files choice, it is the next pump, whenever that comes. Nothing is lost on
  disk; until the result is heard the keys count as unsaved, and a change made meanwhile
  writes them again. A failed save is logged; this interface has no notice yet. The rules a
  part keeps are in `crates/strata-app/src/slots.rs`, "Saving a setting".
- **A part reads** `Setup::settings` once when it is built, and keeps what it shows in its
  own state. Nothing follows a change made by another window or another process.
- The shell saves the sort and the hidden-files choice the same way.
- **One `Saved` for the process.** `startup::load_settings` makes it and the window owns it.
  A second window on Rune would have to share it, which is an open question for the owner.

Sharing the bookkeeping changed what the Rune interface writes, to what the GTK application
writes:

| | Before | Now, as GTK |
| --- | --- | --- |
| A file that does not read whole and is salvaged (one that lacks `mode` or `theme`, or holds a bad entry) | Only the keys this process changed were written | What loading filled in or dropped is saved with the first change |
| The retired key `auto_refresh_interval` | Left in the file | Removed by each save |
| A file that reads whole and holds a value out of range: a volume above 1, a search limit that is not one of the choices, a column width of zero | The value in range was used and the file kept the old one | The value in range is saved with the first change |
| A key this process changed earlier | Written again by every later save, over what another process had saved since | Written until a save succeeds, then left alone |
| A save that failed | Its keys waited for the next change | The next change tries again, and so does a change back to the same value |

What still differs, with the package that will meet each:

| | GTK | The Rune interface | Package |
| --- | --- | --- | --- |
| Start-up effects | Applies the language, reduced motion and the date format | Applies the language only | A18, lgse/strata#2150 |
| A change another process saves | Adopts the sort and the icon size, and re-reads folder views | Follows nothing | A18, lgse/strata#2150 |
| A sort chosen in a folder | Stored for that folder while per-folder settings are on, and as the default otherwise | Always stored as the default | A8, lgse/strata#2140, and A10, lgse/strata#2142 |
| A file that could not be read at start-up | A notice on the first change, and nothing saved over it | Nothing saved over it, and no notice after the warning in the log | A18, lgse/strata#2150 |
| The old global `omarchy_variant` | Moved into the per-theme map when an Omarchy theme is active (`set_active_omarchy_theme`), and the old key is then removed by each save | Neither: nothing reads the old key, and it stays in the file | A18, lgse/strata#2150 |
| A recency sort handed to the save | `set_sort_preferences` returns early and stores nothing | `Saved::store_view` has no such guard | A3, lgse/strata#2130. Not observable today: `Browser::record_sort` returns early at the Recent root, so neither interface is ever handed one |

### Tests

`src/ui/preferences/tests` and `fixtures.rs` did not move. They drive the real path, setter
to commit to writer, through the manager, and are the saved-preferences coverage
[preferences.md](../preferences.md) describes. `settings/saving/tests.rs` adds eight tests of
what only the bookkeeping promises, each in a process of its own. A save's result is polled
by hand, so they show that delivering it needs no executor and no dispatch; the helper that
waits for the writer does iterate GLib's default context. They cover: only changed keys are
written, a salvaged file is saved with every value loading filled in, an unreadable file is
never saved over, a failed save is tried again, only the latest save counts, a save nobody
reports is written again, adopted values are not saved back, and a start-up repair is no
change.

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
| `src/ui/preferences/folder_view_settings.rs` | The same file as on `main`, less the file code and with the lines the rows on `Saving` below name. Git keeps pairing it with `main`'s by its path |
| In that file on `main`: `FILE_NAME`, `EXIT_LOCK_WAIT`, `folder_views_path`, the read in `FolderViewStore::load`, `FolderViewsSave`, `save_folder_views` | `crates/strata-desktop/src/settings/folder_view_file.rs`. A change from `main` to one of them conflicts in `folder_view_settings.rs`, against lines that are gone, and is applied to the new file by hand |
| `src/ui/preferences/trashed.rs` | The same file as on `main` |
| `src/ui/tabs_session.rs`, `tabs_session/tests.rs` | The file of the same name under `crates/strata-desktop/src/` |
| `mod tabs_session;` in `src/ui/mod.rs` | `use strata_desktop::tabs_session;` in the same file |
| A getter or setter of `PreferenceManager`, a binding, a notice, a test or the fixture | The same file as on `main`. Five lines of those files changed, and a change from `main` beside one conflicts there: `appearance_settings.rs:200` and `:213` (the two start-up values, now read from `Saving`), and three assertions that read `saving.borrow().unsaved()` (`bindings/tests.rs:92` and `:96`, `tests/preferences/saving.rs:259`) |
| `PreferenceManager`'s fields and `load` in `src/ui/preferences.rs`; `src/ui/preferences/persistence.rs`; in `folder_view_settings.rs`, the load-failure read in `adopt_defaults_saved_elsewhere` and the bookkeeping in `adopt_defaults`; the bookkeeping in `normalize_loaded_theme_selection` | Rewritten around `Saving`. A change from `main` conflicts there and is applied by hand: to `crates/strata-desktop/src/settings/saving.rs` when it changes what is recorded or saved, to the wrapper when it changes what GTK does around it |
| A new process-wide effect of loading | `PreferenceManager::load` in `src/ui/preferences.rs`, after the shared `load` |
| A new call of `changes.record` or `changes.reset_baseline` | It merges clean and builds, and the next `save_preferences` then panics in debug builds, so in the QA runs: the bindings' snapshot and `Saving`'s no longer agree. Add the matching call beside it: `commit` for a save, `record_adopted` for adopted values, `rebase` for a start-up repair |
| A new use of `last_queued`, `dirty_keys`, `persistence_dirty`, `saves_requested`, `load_failure` or a start-up value | It merges clean and does not build: the field is in `Saving` now |
| `src/ui/keys.rs` | `crates/strata-desktop/src/keys.rs`, less the reader. A change from `main` to `on_press`, `on_release`, `read_event`, `signals`, `read`, `normalize_event` or `Event::read` conflicts there, against lines that are gone, and is applied by hand to `src/ui/keys/mod.rs` |
| `src/ui/keys/bindings.rs`, the files under `bindings/`, `command.rs`, `label.rs` | The file of the same name under `crates/strata-desktop/src/keys/` |
| `src/ui/keys/alias.rs` | The same file as on `main`, less the rewrite. A change from `main` to `FALLBACK`, `Context`'s fields, `Aliasing`, `Alias`, `structural` or `rewrite` conflicts there, against lines that are gone, and is applied by hand to `crates/strata-desktop/src/keys/alias.rs`. `Context::of` and `Context::of_controller` are the free functions `context_of` and `context_of_controller` |
| `src/ui/keys/capture.rs`, `composition.rs`, `conflicts.rs` and every test under `src/ui/keys/` | The same file as on `main` |
| A new item in a moved key file | It merges clean. `pub(crate)` builds in `strata-desktop` and fails where the application names the item: make it `pub`, and re-export it from `src/ui/keys/mod.rs` |
| A new `#[cfg(test)]` item in a moved key file, such as a test's helper or a listing | It merges clean and compiles in `strata-desktop`, where the application's tests cannot see it: they are another crate's, and `cfg(test)` is on only for the crate under test. Make it `#[cfg(any(test, feature = "test-support"))]` and `pub`, as the move did for every such item, and re-export it from `src/ui/keys/mod.rs` under `#[cfg(test)]`. A new name that `src/ui/keys/command/tests.rs` takes from `super::` is added to the test-only `mod command` in `src/ui/keys/mod.rs` |
| A call of GDK or GTK in a moved key file, or of `PreferenceManager` | It merges clean and does not build. A question about a key goes through `Key`'s methods, to the backend; a live setting through `Active`. A new question is a new method of the trait, answered in `src/ui/keys/gdk_backend.rs` |
| `Key::…` for a key the catalog did not name before | It merges clean and does not build: add the constant to `crates/strata-desktop/src/keys/key_constants.rs` |
| `gdk::Key` or `gdk::ModifierType` in a new binding, a new `.ignoring(…)` or a new `KeyPress::from_signal`, anywhere in `src/ui` | It merges clean and does not build. Name `ui::keys::Key` and `ui::keys::Modifiers`; convert with `keys::gdk_key` and `keys::gdk_modifiers` where the value goes to GTK, such as a test's emission of `key-pressed` |
| `tests/qa/mutations/quick-preview.patch` | One line differs from `main`'s, by the owner's exception of 2026-11-12 to the rule that the oracle is not edited: the mutation binds `crate::ui::keys::Key::F35` where `main`'s binds `gtk::gdk::Key::F35`. Keep this side's line in a conflict. A new mutation from `main` that names a GDK key in a binding needs the same one-line change, and the owner's word for it |

[slice/V5b](slice/V5b-settings-store.md#merging-from-main) has the table for the settings
schema.
