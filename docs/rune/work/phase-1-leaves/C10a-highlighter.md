# C10a Highlighter

| | |
| --- | --- |
| Phase / lane | 1 · Preview |
| Plan | §4 C10 (Highlighting, Detection, Virtual lines, Colours, Shared), decision 4 |
| Depends on | S4 (backend), W2. K1 supplies real syntect themes later, and a default theme is used meanwhile. |
| Unblocks | C10b, C11 |
| Owns | `crates/strata-components/src/code_view/highlight/` |
| Branch | `feat/<issue>-c10a-highlighter` |

## Goal

The highlighting engine on its own, with no view, so that C10b's view and C11's code blocks
share one way to colour a file.

## Scope

- syntect with the backend chosen in S4 (`onig`, the owner's decision), plus `two-face`'s
  extra syntaxes. The syntax set loads once, lazily, off the UI thread.
- Detection: file name and extension (`find_syntax_for_file`), then the first line
  (`find_syntax_by_first_line`) for shebangs and modelines, else plain text.
- Checkpoints: the parse and highlight state every N lines (N = 64, from S4).
  `highlight(range)` resumes from the nearest checkpoint, so jumping to the end of a large
  file never parses it on the UI thread.
- Requests go to a dedicated highlighter thread, which owns every `ParseState`: on `onig` a
  parse state is neither `Send` nor `Sync` (S4, B1). Results are colour-only `Span`s per line.
- A theme switch re-colours from the cached scopes without re-parsing.
- Edits: `edit(line, …)` invalidates from that line. The editable variant keeps a parse state
  per line (S4's workaround 1): the foreground re-parses from the edit to the bottom of the
  viewport plus overscan, then a background pass runs until the state equals the cached one
  again. On `onig` an edit inside a heredoc or fence runs to the block's end (S4, B2), so
  lines past the pass are provisional until it ends. This is for C10b's editable variant.
- Adapters for rune-kit's `Markdown::highlight` hook, for C11, and for TextArea spans.

## Tests

- Unit: detection by name, extension, shebang and modeline; a resume from a checkpoint gives
  the same spans as a full parse; once the background pass finishes, an edit's spans equal a
  full parse's.
- Perf (the S4 fixtures): a viewport at the end of the 50k-line file highlights within the
  8 ms reading at p95 on the highlighter thread, which N = 64 is sized for. A one-line edit
  in the 5,000-line script stays under 2 ms.

## Done when

The tests pass, and the shared adapters are used by a gallery scene that draws a Markdown code
block.
