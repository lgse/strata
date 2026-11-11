# C19 Footer and ShortcutReference

| | |
| --- | --- |
| Phase / lane | 2 · Chrome |
| Plan | §4 C19, `app/shortcut-reference` |
| Depends on | C15 (the F1 panel), W1 (matched text) |
| Unblocks | X3 (the clipboard badge), M3 |
| Owns | `crates/strata-components/src/footer/`, `scenes/c19.rs` |
| Branch | `feat/<issue>-c19-footer-shortcut-reference` |

## Goal

The window footer, and the F1 key map panel.

## Scope

- **Footer:** an F1 button, an item and selection count (with plurals as data), the "Files on
  clipboard" badge (a flag the host sets), and slots for C16's jobs element and C18's
  prompt.
- **ShortcutReference:** a C15 panel that shows the active key map (data: category, action
  and chords) as rune-kit `Keybind` chips. It has search with matched text, and category
  filters.

## Tests

Scripts: F1 opens and closes, search narrows, a category filter, the counts update from data.
A11y golden: the counts as a status region, and the panel's table.

## Done when

The shortcut-reference bullets pass headless.
