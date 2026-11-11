# C11 DocumentView

| | |
| --- | --- |
| Phase / lane | 2 · Preview |
| Plan | §4 C11, `preview/preview-panel/documents/*`, `.../odt` |
| Depends on | C10a (code blocks), C9 |
| Unblocks | M5 |
| Owns | `crates/strata-components/src/document_view/`, `scenes/c11.rs` |
| Branch | `feat/<issue>-c11-document-view` |

## Goal

Render Strata's own document model natively: Markdown, the HTML subset, RTF, DOCX and ODT,
plus spreadsheets.

## Scope

- **The model:** a document type here that mirrors Strata's parsed model, at the plan's
  watermark commit. Strata converts into it. The parsers stay in Strata.
- **Blocks:** headings, paragraphs, lists, tables, images (as `Pixels`), links (an intent),
  and code blocks coloured by C10a.
- Selectable text across blocks, and find highlights. Use rune-kit `Markdown` as the
  reference for table sizing and multi-block selection, but not as the renderer.
- **Spreadsheets** (CSV, TSV, XLS, XLSX, ODS): a virtual grid with sheet tabs, row numbers
  and column letters, built on rune-kit Table cells. It needs horizontal and vertical
  scrolling on large sheets.

## Tests

Scripts: select across blocks and copy, find, a link click intent, switching sheet tabs, and
scrolling a 100k-row sheet. A11y golden: headings with levels, lists and tables. Scenes: one
fixture document per format, and a sheet, in light and dark.

## Done when

The documents and ODT bullets pass headless.
