# Test-suite coverage audit (#837)

Baseline: `14ed2e0`. This is a coverage-owner audit, not a quota for test deletion.
A similar name, shared fixture, or different parameter value does not by itself
make two tests equivalent.

| Inventory | Before | After | Net reduction |
| --- | ---: | ---: | ---: |
| Rust tests (`--all-targets --all-features`, including ignored fixtures) | 1,498 | 1,455 | 43 |
| E2E scenarios (expanded pytest parameters) | 766 | 724 | 42 |
| E2E harness unit cases | 46 | 46 | 0 |
| Total canonical pytest collection | 812 | 770 | 42 |

Two removed Rust entries were ignored, one-off screenshot generators, not running
regressions. Consolidated assertions still execute; fewer test functions are not
the same as fewer checks. These counts do not establish a runtime improvement.

## Rust decisions

Paths below are relative to `src/`.

| Removed or consolidated checks | Retained coverage / reason |
| --- | --- |
| `ui/window/composition/tests/header_geometry.rs`: both header geometry fixtures | Retire exact header/button/icon dimensions and alignment as standalone contracts. Window actions, preview lifecycle, accessibility and preference application remain tested; no claim that visual baselines cover every old pixel assertion. |
| `ui/browser/tests/layout.rs`: chrome insets, equal row heights and caption centering | Keep `thumbnail_size_control_updates_the_grid_and_survives_view_rebuild`: the real size control must update mapped slots and survive mode changes. Column resizing and preview-close grid reflow keep their existing dedicated regressions. |
| `ui/icons_cell/tests.rs`: square/airy proportions, fixed size requests, exact label properties | Keep lazy editor creation and `thumbnails_do_not_obscure_wrapped_names_or_rename_fields`. Real rendered names must wrap and not overlap opaque thumbnails; editor/caret clipping is also covered by `ui/browser/inline_edit/tests/caret.rs`. Exact card proportions are deliberately no longer asserted. |
| `ui/browser/properties/tests/progress.rs`: fixed spinner/value positions and digit widths | Keep throttling and `folder_properties_loads_sizes_and_reports_unavailable_roots`, which exercises intermediate size updates, hidden contents, empty/missing roots and spinner termination. Exact geometry is cosmetic. |
| `ui/controls/tests.rs`: header helper setter/getter assertions | Actual navigation/pane controls retain cursor coverage in `ui/browser_modes/tests.rs`; discard a second test of the same CSS class/alignment assignments. |
| `ui/window/tests.rs`: CSS substring matching and hand-written at-rule allowlist | Keep `ui/browser/tests/hover.rs::application_stylesheet_has_no_parser_errors`, using the actual GTK parser. Exact stylesheet spelling and icon sizes are not behavioral contracts. |
| `ui/browser_modes/tests.rs`: default-width constants, card extents, grid pin setter/getters | Keep minimum-width clamping, live `column_widths::mode_fits_default_width_and_remains_resizable`, viewport column calculation and `ungrouped_icons_reflow_when_the_preview_split_closes`. Remove the unused `_activation` loop, which ran identical inputs twice. |
| `ui/browser_modes/tests/skeletons.rs`: placeholder child counts/classes/constant widths and comparison generator; `ui/browser/tests/loading.rs`: capture generator | Keep `directory_loading_grace_across_modes` for fast/slow, reload, empty, failure and superseded loads. Move non-targetable/non-focusable placeholder checks there, across all views. Retire historical screenshot-generation code, not loading behavior. |
| `ui/theme/tests.rs`: repeated legacy/default fixtures and basic preference round-trips | `fresh_and_legacy_preferences_share_behavioral_defaults` checks both construction paths. `ui/theme/tests/preferences.rs` keeps the exhaustive non-default fixture, startup loading and every setter's publication/persistence checks, including both acceleration booleans, Small/Large text, audio and sidebar order. Enum variants, invalid values, custom maps and salvage tests remain. |
| Eight delete-menu visibility tests | One explicit six-row truth table checks both actions for normal/Trash locations and unknown/supported/unsupported capabilities. It retains all previous assertions and fills the two omitted combinations. |
| Preview print progress, file sizes, timestamps and drag-entry option cases; browser file sizes | Consolidate by function under test, preserving boundary inputs and expected outputs. Preview and browser size formatters are **not** duplicates: one intentionally displays `1.0 MB`, the other `1 MB`. |
| Standalone media close/finalization | `replacing_repeated_media_previews_finalizes_previous_widget_trees` already checks production widget finalization after replacements and final close; normalized-file detachment/deletion retains its separate test. |
| `app/browser/tests.rs::navigation_events_are_delivered_to_every_observer` | Move the Reset assertion into `fan_out_shares_one_event_with_every_observer`, which exercises three observers and publication. Reentrancy and observer mutation tests remain separate. |
| `adapters/local_operations/tests.rs::duplicating_a_file_generates_numbered_name` | `duplicating_a_file_preserves_contents_and_reports_the_generated_name` checks the same provider operation, both files' contents, and the reported generated destination together. Unicode, suffix collisions, directory copies, symlinks and conflict variants remain. |

## E2E decisions

Paths below are relative to `tests/e2e/scenarios/`. No scenario is made optional,
ignored or conditional on changed files. Existing real-input interactions remain
real-input interactions; the harness and CI gates are unchanged.

| Consolidation | Net cases removed | Retained owner |
| --- | ---: | --- |
| Four accessibility listing inventories plus initial-location smoke test | 12 | `test_accessibility.py::test_listing_names_descriptions_and_selection_semantics` checks all entries, descriptions, container/pane names, presentation, focusability and selection in all three modes. |
| Standalone menu semantics, menu shortcut description, dialog semantics and inline-field naming | 4 | Existing file-menu scenario now checks roles, names and Copy's accelerator in every mode; permanent-delete confirmation checks dialog role/name. `test_inline_renaming.py::rename_field` requires the focused editable **Rename**, including new files and both new-folder routes. |
| Shortcut switching, round-trip, menu checkmarks and saved-view smoke test | 7 | `test_view_switching.py::test_shortcut_round_trip_preserves_selection_and_updates_the_appearance_menu` makes an actual transition through Icons/List/Columns, preserving selection and checking menu state after each. Both pointer/menu transitions also verify persistence. Directory/sort/focus preservation stays separate. |
| Properties open/close smoke | 1 | `test_file_properties_describes_the_file_without_pin_actions_and_closes` adds the filename assertion to the existing file-property restrictions and Escape workflow. |
| Compress Enter smoke | 1 | `test_enter_submits_compress_and_extract_to_dialogs` checks compress dismissal and archive creation before extracting, then verifies extracted contents. Invalid-name and other dialog submit paths remain separate. |
| Keyboard new-folder creation smoke | 3 | All three modes already run `test_leaving_a_valid_name_commits_it` with new folder + Enter, using Ctrl+Shift+N. Retain its filesystem checks and add a listing assertion after Enter. |
| Separate permanent-delete cancel/confirm launches | 1 | One scenario checks no early deletion, cancellation preserving the file, reopening and confirmed deletion without trashing. Symlinked-parent cases remain separate. |
| External creation/removal refresh launches | 3 | `test_refresh_reconciles_external_file_creation_and_removal` checks both effects of one F5 in each mode. Dedicated monitor/selection regressions remain. |
| Missing startup argument smoke | 1 | `test_missing_directory_can_be_restored_and_retried` already has the same unavailable-location and Retry checks, then restores and opens the location. |
| Escape direction parameter | 6 | Both previous/next keys still run for every mode and single/multiple selection; reset and assert the original selection before each direction, then verify selection, cursor and unchanged panes. |
| Filtered rename's unused menu `focus_filter` parameter | 3 | Explicit tuples retain menu once per mode and both focus states for F2/Ctrl+R. The menu branch never read `focus_filter`, so the removed cases were identical executions. |

## Deliberately retained

- All 72 valid rename focus-exit combinations, plus invalid-name and Escape
  lifecycle matrices. Different GTK focus-walk exit targets are not interchangeable.
- Drag origins, modifiers, density/padding hit targets, no-op/failed drops and
  animation/lifetime regressions; wheel/popover routing and marquee scrolling.
- Archive format/encryption/cancellation vectors, filesystem identity and symlink
  safety, remote URI handling, clipboard undo and cross-volume decisions.
- Saved preferences before Settings opens, live two-window propagation, view
  rebuilds and chooser-specific policy.
- Recursive and directory-only result activation: even an immediate-child query
  passes through different result/model paths. Likewise, current-folder Properties
  and entry Properties use distinct targeting routes in each view.
- The nine visual baselines and the expensive deferred-search-scroll regression.
  Cost alone is not evidence that a test is redundant.
- Harness/provenance/sharding tests. They enforce collection and result integrity,
  rather than duplicate application workflows.

## Validation and maintenance

The scope crosses test modules and GUI workflows, so validation uses the full
pinned `scripts/quality.sh test`, pre-push `fmt`/`clippy` phases, and canonical
`STRATA_CONTAINER_ENGINE=podman scripts/e2e.sh`, not a filename-based subset.
Displays and D-Bus sessions remain private. The view-switching and filter-results
mutation selectors still select the complete changed scenario files; the exact
rename-caret selector is unchanged.

Delete scheduling hints only for removed/renamed nodes. Do not invent new timings
for merged scenarios or treat a timing file as a coverage allowlist. Refresh
measurements from a complete passing CI attempt as documented in
[e2e-testing.md](e2e-testing.md#reproducing-and-maintaining-shards).

For future additions, first identify the missing behavioral distinction. Prefer
adding assertions to an existing matching setup, use tables for pure-function
input vectors, and retain separate launches where initial state or independent
input/event routing is the actual regression.
