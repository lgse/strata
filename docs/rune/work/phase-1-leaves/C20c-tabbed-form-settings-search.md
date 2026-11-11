# C20c Tabbed form and settings search

| | |
| --- | --- |
| Phase / lane | 1 · Controls |
| Plan | §4 C20 (Tabbed form, Settings search), `settings/preferences`, `integration/custom-actions` |
| Depends on | W1 (matched text), W2 |
| Unblocks | M6 (with C20a, C20b, C20d) |
| Owns | `crates/strata-components/src/controls/tabbed_form/`, `.../settings_search/`, `scenes/c20c.rs` |
| Branch | `feat/<issue>-c20c-tabbed-form-settings-search` |

## Goal

The two settings composites that are not plain rune-kit `Page` rows.

## Scope

- **Tabbed form:** the action editor's notebook. A SegmentedControl over pages, with each
  page kept mounted or rebuilt by a flag, and a per-page "has errors" mark.
- **Settings search:** a search field over a row index (page, section, row key, label and
  keywords, all as data). Results show with matched text. Choosing one jumps to the page,
  scrolls the row into view, and highlights it briefly.

## Tests

Scripts: switching tabs by keyboard, the error mark, search then Enter jumps and highlights,
Escape clears. A11y golden: tab roles, and the results list as a listbox.

## Done when

The settings search bullets of `settings/preferences` pass headless.
