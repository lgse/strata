# C20d IconPicker

| | |
| --- | --- |
| Phase / lane | 4 · Controls |
| Plan | §4 C20 (IconPicker), `browser/folder-customization` |
| Depends on | C3, K3 |
| Unblocks | M6 |
| Owns | `crates/strata-components/src/controls/icon_picker/`, `scenes/c20d.rs` |
| Branch | `feat/<issue>-c20d-icon-picker` |

## Goal

Replace GTK's `EmojiChooser` with a searchable grid of the bundled icons and emoji.

## Scope

- A C3 grid over a source of icons (K3's catalogs by name) and emoji (with their names and
  keywords as data).
- A search field with matched text, and category tabs.
- Choosing one sends an intent. A "recent" section comes from data.
- Opens in a Popover, or inline.

## Tests

Scripts: search narrows, keyboard moves through the grid, Enter chooses, and switching
categories. A11y golden: a grid of named options.

## Done when

The folder-customization icon bullets pass headless.
