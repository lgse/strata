# C10b CodeView

| | |
| --- | --- |
| Phase / lane | 2 · Preview |
| Plan | §4 C10 (virtual lines, find, scrollbar, editable variant), `preview/preview-panel/find`, `.../documents/scrollbar`, `integration/custom-actions` |
| Depends on | C10a, C9 (implements its header trait) |
| Unblocks | M5. Strata's action script editor. |
| Owns | `crates/strata-components/src/code_view/` (except `highlight/`), `scenes/c10.rs` |
| Branch | `feat/<issue>-c10b-code-view` |

## Goal

Replace `sourceview5` for source and plain-text previews, and the custom-action editor.

## Scope

- Virtual lines, so large files open at once, with line numbers and soft wrap (a toggle).
- Lines draw plain until C10a's spans arrive, so nothing moves when colours land.
- Find: a field in C9's header, all matches highlighted, next and previous, and a match count.
- Selection across lines, with copy.
- The document-position scrollbar: a thumb that follows the line position, not estimated
  heights.
- An editable variant on TextArea with C10a's spans, re-highlighting on edit.

## Tests

- Scripts: open the 50k-line fixture and jump to the end with no UI stall (commit budget),
  find next and previous, wrap toggle, select and copy, edit re-highlight.
- A11y golden: a document, with lines as text.
- Perf: the 50k-line scroll under `RUNE_COMMIT_BUDGET_MS=8`, and a 5,000-line script edit.

## Done when

The source preview and find bullets pass headless, and the perf runs meet their budgets.
