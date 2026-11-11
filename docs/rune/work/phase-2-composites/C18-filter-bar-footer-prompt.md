# C18 FilterBar and FooterPrompt

| | |
| --- | --- |
| Phase / lane | 2 · Chrome |
| Plan | §4 C18, `browser/search/filter`, `.../filter/enter-target`, `integration/10xer-mode` |
| Depends on | C1a (cursor and filled selection), W1 (matched text) |
| Unblocks | M3 |
| Owns | `crates/strata-components/src/filter_bar/`, `scenes/c18.rs` |
| Branch | `feat/<issue>-c18-filter-bar-footer-prompt` |

## Goal

The Ctrl+F filter with its Enter target, and the 10xer footer prompts with the chord panel.

## Scope

- **FilterBar:** a TextInput that reports the query. The host filters the source. The bar
  picks the Enter target (the first match, or the cursor if it matches), and C1e draws its
  outline from a target id. Enter opens it, and Escape clears and restores the selection.
- **FooterPrompt:** a footer prompt with a label (`/`, `f`, `s`, `go ›`, `rename ›`), its
  input, and completion hints as data.
- **Chord panel:** after a chord prefix, a panel lists the next keys with rune-kit `Keybind`
  chips, from a key map given as data.

## Tests

Scripts: filter narrows (with a fake source), the Enter target follows the query, Escape
restores, each prompt opens, submits and cancels, and the chord panel shows after a prefix.
A11y golden: the prompt names, and the panel as a list.

## Done when

The filter and 10xer prompt bullets pass headless.
