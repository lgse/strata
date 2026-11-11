# C17 SearchPalette

| | |
| --- | --- |
| Phase / lane | 2 · Chrome |
| Plan | §4 C17, `browser/search`, `browser/navigation/folder-jump`, `integration/10xer-mode/places` |
| Depends on | C15, W1 (matched text) |
| Unblocks | M3 |
| Owns | `crates/strata-components/src/search_palette/`, `scenes/c17.rs` |
| Branch | `feat/<issue>-c17-search-palette` |

## Goal

One modal palette for Ctrl+K search, folder jump and the 10xer folder picker.

## Scope

- A C15 modal with a TextInput over a virtual result list.
- Results stream in while the query runs, from the host through `rune::Latest`. A new query
  supersedes the old one, and late answers are dropped.
- Matched characters are drawn with W1's matched text.
- The empty, loading, no-results and error states are each distinct.
- Arrows move, Enter opens, a modifier opens in a new tab (as Strata defines it), and Escape
  closes.
- One component, with modes as data (title, placeholder, result kind), not three copies.

## Tests

Scripts: typing streams results, a superseded query's results never show, the keyboard, and
each state. A11y golden: a combobox with a listbox, the active descendant, and the result
count as a live region.

## Done when

The search and folder-jump bullets pass headless.
