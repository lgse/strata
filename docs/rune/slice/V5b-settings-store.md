# V5b The settings store without GTK

| | |
| --- | --- |
| Status | Done |
| Depends on | V4 |
| Unblocks | V5's settings, V6, V8 |
| Owns | `crates/strata-desktop/src/settings.rs` and `settings/`, and what stays of `src/ui/preferences` |
| Commits | `ce3b5b83d` (the move), `a8f13eef9` (`strata-app` reads the file) |

What `settings.toml` holds, its defaults, its stored forms and the reading of the file now
live in `strata_desktop::settings`, with no toolkit. `PreferenceManager` stays in
`src/ui/preferences.rs` and wraps them. The Rune interface reads the same file through the
same code.

[V5](V5-shell-and-loop.md) said to survey `src/ui/preferences` first, and to give the move
its own document if it touched more than a few files. It touched thirteen.

## The owner's ruling

`AGENTS.md` says not to move or reorganize `src/ui` on `strata-next`, because every move
makes the merges from `main` harder. V5 recommends moving the store. Asked on 2026-11-11
with the survey below, the owner chose to move it now.

The ruling covers the preference files named here. Any other move under `src/ui` still
needs asking.

## Survey

`PreferenceManager` holds GTK directly, so it cannot move as it is:

- `bindings::PreferenceChanges` keeps a weak `gtk::Widget` anchor for every binding.
- Two `save_notice::SaveNotices` keep the `gtk::Window`s that show "Settings can't be saved".
- The resolved `ui::keys::Keymap`, which needs GTK to parse accelerators.
- The folder-view store and the item customizations schedule on GLib timers and idles.

Its setters and getters are spread over nine `impl PreferenceManager` files. They read the
fields of `Preferences` directly, publish through the bindings, and call into GTK code
(thumbnails, motion, the tab session).

The serialized `Preferences` struct needs no GTK. It did need three things from elsewhere
under `src/ui`: the key catalog (`ui::keys`), the view-mode and density types
(`ui::browser_modes`) and the Icons view's size limits (`ui::icons_cell`).

Of the 22 files in the folder, with `src/ui/preferences.rs` itself:

| File | Needs | Now |
| --- | --- | --- |
| `preferences.rs` | GTK (bindings, notices) | Split. The struct and its four enums moved; the manager stays |
| `defaults.rs` | Nothing | Moved whole |
| `corner_roundness.rs` | Nothing | Moved whole |
| `list_columns.rs` | Nothing | Moved whole |
| `stored.rs` | `ui::browser_modes` | Split. The mode and density conversions stay |
| `persistence.rs` | GLib, for the save queue | Split. Reading, salvage and the merge moved; the save queue stays |
| `text_size.rs` | `ui::keys` | Split. The type moved; the shortcut mapping stays as a trait |
| `keymap.rs` | `ui::keys` | Split. The saved overrides moved; resolving them against the catalog stays |
| `folder_view_settings.rs` | GLib timers | Stays. Its save lock, `lock_saves`, moved |
| `appearance_settings.rs`, `file_settings.rs`, `input_settings.rs`, `item_customizations.rs` | GTK | Stay: manager methods |
| `layout_settings.rs`, `preview_settings.rs`, `sidebar_settings.rs` | Nothing | Stay: manager methods |
| `bindings.rs`, `save_notice.rs`, `desktop_scale.rs` | GTK | Stay |
| `folder_views.rs`, `folder_views/keys.rs`, `trashed.rs` | Nothing | Stay. See "Left for later" |
| `fixtures.rs` and `tests/` | The manager | Stay. See "Tests" |

## What moved

About 1,180 lines went to `crates/strata-desktop/src/settings.rs` and `settings/`. The code
is the same apart from visibility and paths.

| In `strata_desktop::settings` | Holds |
| --- | --- |
| `Preferences`, `InterfaceRenderer`, `OmarchyVariant`, `FileIconColors`, `UnitSystem` | What the file holds, each value as stored |
| `defaults` | The value of every setting a file omits |
| `stored` | `sort_preferences`, `store_sort`, the search-limit choices, the ranges values are kept within, `Preferences::normalize_loaded` |
| `persistence` | `settings_path`, reading, `salvage_preferences`, `record_changes`, `merged_settings`, `lock_saves` |
| `CornerRoundness`, `TextSize`, `ListColumnId`, `ListColumns`, `KeymapOverrides` | The stored types |
| `MIN_ICONS_THUMBNAIL_SIZE`, `MAX_ICONS_THUMBNAIL_SIZE` | The range loading clamps to. `ui::icons_cell` re-exports them |
| `Preferences::load_read_only` | New: read, salvage and normalize, for an interface that never saves. Removed in V8, when `strata-app` began to save `show_hidden` and the sort: its `settings::Saved` reads with `read_loaded_preferences` so that it knows when the file could not be read |

The fields of `Preferences` are `pub` now, because the manager's methods read them from
another crate. Nothing else became reachable that was not: `strata-desktop` is used by the
application and by `strata-app` only.

`strata-desktop` gained a dependency on `strata-preview`, for the thumbnail worker limits
and the video backend. `deny.toml` allows it.

## What stayed, and why

- **`PreferenceManager` and every method on it.** The setters publish through GTK bindings.
  A store that both interfaces write through needs change notification without widgets,
  which nothing asks for until the Rune interface has a settings page.
- **The save queue** (`save_preferences`, `queue_settings_save`). It answers on the GLib
  context and reports to the notice windows. What it writes is `merged_settings`, which
  moved.
- **Conversions to GTK-side types.** `BrowserMode` and the density scales are defined in
  `src/ui/browser_modes`, a GTK module. The Rune interface has one view and reads
  `browser_mode` as text until it has more.
- **Resolving keymap overrides.** That needs the key catalog, which is still under
  `src/ui`.

## How the application reaches it

`src/ui/preferences.rs` re-exports the moved names at their old paths. About 240 files name
`PreferenceManager` or a preference type, and outside `src/ui/preferences` three of them
changed. `src/ui/window/keyboard.rs` and `src/ui/chooser/shortcuts/editing.rs` gained one
import each, for `TextSizeShortcut`, and `src/ui/icons_cell.rs` re-exports the two limits
it used to define.

## How the Rune interface reads it

`strata-app` reads the file (`settings::Saved::load`) before it claims the UI thread. It sets
the process language from `language`, and builds its `Browser` with `sort_preferences`:
`show_hidden`, `folders_first`, `sort_key`, `sort_direction`, `natural_sort` and
`case_sensitive_sort`.

The other keys V5 lists are parsed with the file and not used yet: `type_to_search`,
`single_click_previews`, `tenxer_mode`, `arrow_navigation_scoped`, `group_by_type`,
`filter_include_subfolders`, `browser_mode`, `browser_column_width` and
`columns_show_sizes`. V6 and V8 read them where the Columns view and the scenarios need
them.

Since V8 the Rune interface saves two things, as the GTK application's
`set_sort_preferences` does: the hidden-files choice and the sort, when its `Browser` says
they changed. `strata-app`'s `settings::Saved` queues the save on the shared writer and
merges only the keys it changed. It reads `type_to_search` and `tenxer_mode` for the
Columns view's letters. A file it cannot read gives it the defaults and is never saved
over.

## Merging from `main`

A change on `main` to code that moved conflicts when `main` merges into `strata-next`. Git
reports it in the file under `src/ui/preferences`, as a change to lines the branch deleted.
Apply it to the moved file by hand:

| On `main` | On `strata-next` |
| --- | --- |
| A field of `Preferences` in `src/ui/preferences.rs` | `crates/strata-desktop/src/settings.rs`, as a `pub` field |
| `defaults.rs`, `list_columns.rs`, `corner_roundness.rs` | The file of the same name under `settings/` |
| A normalizer or `normalize_loaded` in `stored.rs` | `settings/stored.rs` |
| Reading, salvage or the merge in `persistence.rs` | `settings/persistence.rs` |
| The `TextSize` type, the `KeymapOverrides` struct | `settings/text_size.rs`, `settings/keymap.rs` |
| A setter or getter on `PreferenceManager` | The same file as on `main`. A new name it uses from the moved code joins the import list at the top of `src/ui/preferences.rs` |

A new setting on `main` therefore needs a hand merge of its field and its default. Its
setter, its binding and its tests merge as before.

Files that moved after the slice, starting with the per-folder views' model, are in
[shared-state.md](../shared-state.md#merging-from-main).

## Tests

The tests did not move. `src/ui/preferences/tests` and `fixtures.rs` exercise the moved
code through the manager and through the re-exports, and they are the saved-preferences
coverage `docs/preferences.md` describes: the exhaustive fixture, loading, recovery, saving
and relocations.

`read_preferences` was `#[cfg(test)]`. Another crate's tests cannot see that, so it is
behind `strata-desktop`'s `test-support` feature now, which the application's tests turn
on.

`strata-app` has one test of its own:
`host::offscreen::tests::saved_settings_reach_the_browser_and_the_language`. It writes a
`settings.toml` with one malformed entry, and checks the browser's sort, its hidden-files
choice and the process language.

## Validation

At `a8f13eef9`, on 2026-11-11. The pinned-image runs are from the worktree at
`~/dev/strata-next-verify`, and their logs are in its `target/`.

| Command | Where | Result |
| --- | --- | --- |
| `./scripts/quality.sh`, all three phases | Pinned image | Passed. 3,948 tests passed exactly once and 771 are ignored, the counts from before the move. `v5-final-quality.log` |
| `./scripts/qa.sh` | Pinned image | Passed: 787 tests, 758 scenario runs, 2 ignored. `v5-final-qa.log` |
| `./scripts/test-headless.py preferences` | Host, private display | Passed: 116 tests |
| `mise run test-rune` | Host | Passed: 2,298 tests, 252 ignored, with `strata-app`'s settings test among them |
| `cargo deny --workspace check` | Host | Passed, with `strata-preview` allowed under `strata-desktop` |

Not run: `./scripts/qa-mutation.sh`. No mutation patch touches a file that moved or
changed.

## Left for later

- **A store both interfaces write through.** The Rune interface needs it with its settings
  page: the setters, the dirty-key tracking and a change notification that takes no widget.
- **Per-folder views.** `folder-views.toml` holds the sort and icon size a folder
  remembers. Its model (`folder_views.rs`) needs no GTK, but its store runs on GLib timers.
  The Rune interface applies the default sort to every folder until it moves. No scenario
  in the slice seeds that file.
- **The key catalog.** `KeymapOverrides` is read and kept, and nothing on the Rune side
  resolves it.
- **The date format and reduced motion.** `PreferenceManager::load` applies both
  process-wide. `strata-app` applies only the language, the one V5 needs.
- **Tests of the moved code in its own crate.** They run in the application's test binary
  today, and leave with `src/ui` at parity.
