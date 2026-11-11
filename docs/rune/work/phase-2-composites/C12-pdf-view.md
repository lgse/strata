# C12 PdfView

| | |
| --- | --- |
| Phase / lane | 2 · Preview |
| Plan | §4 C12, `preview/preview-panel/pdf` |
| Depends on | C14a, C15 (password prompt), C9 |
| Unblocks | M5 |
| Owns | `crates/strata-components/src/pdf_view/`, `scenes/c12.rs` |
| Branch | `feat/<issue>-c12-pdf-view` |

## Goal

PDF pages from Strata's sandboxed renderer, as a scrolling, zoomable and selectable view. The
renderer and its protocol are unchanged.

## Scope

- A virtual column of pages, sized from page dimensions before the images arrive.
- Each page is `Pixels`, filled from the renderer's PNG pages (data). A page re-renders at the zoom
  level: the view asks for a scale through C14a's "zoom changed" report.
- **Text layer:** glyph boxes from the renderer (data). It draws the selection, and builds
  the copied text in reading order.
- Find over the text layer.
- A password prompt on C15 that hands the password to the host.

## Tests

Scripts with fake pages and glyph boxes: scroll, zoom triggers re-render requests for visible
pages only, select across a page boundary and copy, find, and the password prompt. A11y
golden: a document with pages.

## Done when

The PDF bullets pass headless.
