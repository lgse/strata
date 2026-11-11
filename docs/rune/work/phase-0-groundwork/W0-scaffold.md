# W0 Parallel-ready scaffold

| | |
| --- | --- |
| Phase / lane | 0 · Groundwork |
| Plan | §3 (layout, rules), §6 (testing) |
| Depends on | — |
| Unblocks | Every WP |
| Owns | `Cargo.toml`, `crates/*/Cargo.toml`, `crates/*/src/lib.rs`, `crates/strata-gallery/**`, `tests/**`, `mise.toml`, `.github/workflows/ci.yml` |
| Branch | `build/<issue>-w0-scaffold` |

## Goal

Lay out the workspace so that ~30 branches can run at once without editing the same lines.
After W0, a WP adds files in its own folders and touches shared files only by the rules in
[the index](../README.md#shared-files).

## Scope

- **strata-components:** one module per component, declared now and empty except for a doc
  comment that links its WP doc: `collection` (C1a–C1e, split into submodules), `columns`,
  `icons_grid`, `list_table`, `rename`, `location_bar`, `tabs`, `sidebar`, `preview_drawer`,
  `code_view`, `document_view`, `pdf_view`, `media_player`, `viewers`, `dialog`,
  `progress_dock`, `search_palette`, `filter_bar`, `footer`, `controls` (C20a–C20d),
  `context_menu`, `folder_peek`, `motion`, `scroll`, `thumbnails`, `mount_prompts`.
  W1's `contract` module is declared too. Split components get their submodules declared
  as well, so no two WPs edit the same `mod.rs`: `collection/{selection, pointer, keyboard,
  marquee, element}`, `code_view/highlight`, `viewers/viewport` and `controls/{slider,
  number_field, color_picker, tabbed_form, settings_search, icon_picker}`.
- **strata-platform:** every feature from plan §3 declared and empty (`dnd`, `clipboard`,
  `csd`, `foreign-parent`, `settings`, `icon-theme`, `single-instance`, `glib-bridge`), plus
  `media-audio` for P7. Each gates one empty module. The `transfer` module (P0) is internal and
  enabled by `dnd`, `clipboard` and `foreign-parent`.
- **strata-kit:** the `look`, `tokens`, `theme`, `icons`, `overrides` and `slots` modules,
  declared for W2 and K1–K4 to fill.
- **Gallery:**
  - Split `strata-gallery/src/lib.rs` into `scenes/<id>.rs`, one per component. Each file
    provides its scenes, its demo state type and its view.
  - `Gallery` holds one `Default` demo-state field per component, declared now.
  - The scene list is put together from each module's list. Scene names are prefixed with
    the component id (`c1-empty-dark`).
  - A gallery index page lists the components.
- **Test harness** in `strata-gallery/tests/`:
  - The a11y golden check, moved to `tests/golden/a11y/<id>/<scene>.json`.
  - PNG goldens under `tests/golden/png/<id>/`, with a tolerance taken from rune-kit's gallery
    tests.
  - A script runner for `tests/scripts/<id>/*.json`. Each script carries a `bullets` list
    of feature-map Behavior bullet ids, and the runner fails if it is empty.
- **Tasks:** `mise run bless [filter]` takes a filter, the way `test` does. Add
  `mise run compile-features` to check `strata-platform` with each feature alone, so no module
  relies on another's dependencies. `check` and CI run it.
- **Docs:** `docs/testing.md` covers the script format, golden folders and bless filters.
  Update `tests/README.md` to match.

## Out of scope

Any component, token or shim code. Any new third-party dependency.

## Done when

- `mise run check` passes, and the two existing scenes pass under their new names and paths.
- A throwaway branch that adds a scene to two different components, in two worktrees,
  merges with no conflict outside `Cargo.lock`. Try it, then delete the branch.
- `mise run bless c1` rewrites nothing outside `tests/golden/*/c1/`.
