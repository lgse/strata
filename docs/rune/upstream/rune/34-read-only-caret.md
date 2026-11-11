# 34. A caret and keyboard selection in read-only text

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C10b-2, R2 (C11) |
| Affects | C10b (code view), C11 (document view), C12 (PDF text, which ports its own caret) |
| Retires | C10b's D1 and C11's D3 deviations (no caret or keyboard selection in read-only views) |
| Checked | Re-checked at `2a44156` by source: `El::read_only` is still the ARIA state only (`el/a11y.rs:124–129`); `text_input` (`el.rs:673`) has no read-only mode; and selectable text still takes only Cmd+C and Cmd+A from the keyboard (`selection_key`, `runtime/selection/selectable.rs:707–708`), so no caret, Shift+arrow, Home/End or word steps. The selection work past the pin (`El::selection_style`, `Cx::mark_texts`, `El::unselectable`) paints or scopes selections and adds no caret. Nearest issues: #353 (hit-test a point to a byte offset) and #129 (a handler for a field's caret moves), both open and neither about read-only text. C10b's D1 (`code_view/mod.rs:79`) and C11's D3 (`document_view/mod.rs:82`) are still there; no gallery test covers the missing caret. No program needed. Earlier: by source at `5d421fd` and `04b38ae` (`el/a11y.rs:123`). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Read-only text has no caret: no Shift+arrow selection, Home and End by line, or word steps

Context: Strata's (lgse/strata, a file manager on Rune) source and document previews are read-only,
but users select from the keyboard there, as GTK's `editable(false)` with `cursor_visible(true)`
allows.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
- `text_input` has no read-only mode, so a read-only view can't have a caret, Shift+arrow
  selection, or Home and End by line.
- `El::selectable` text has no caret: no Shift+arrow, Shift+Home/End or word-step extension, no
  Right collapsing the selection, no grapheme steps. Its keyboard handling is Cmd+C and Cmd+A
  (`runtime/selection/selectable.rs:707–708` at `2a44156`).

**Ask.** `text_input(..).read_only()` that keeps navigation, selection and copy and refuses
typing, paste, cut and IME; or a keyboard caret for `selectable` text.
