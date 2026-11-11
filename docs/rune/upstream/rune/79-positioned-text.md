# 79. Selectable text laid over glyph boxes the app places

| | |
| --- | --- |
| Kind | New issue (unlike #353, which is hit-testing shaped text) |
| Drafts | U-C12-1 |
| Affects | C12 (PDF text layer) |
| Retires | C12's port of a PDF text selection (caret, word and line, copy) and its silent accessible text |
| Checked | Re-checked at `2a44156` (2026-11-10) by source: `text()` (`el.rs:55`) and `El::selectable` (`el/text.rs:461`) still select only text Rune shapes; `TextSpec` (`el/text_spec.rs:8`) takes content, spans and wrapping, no glyph rects. The selection work since the pin doesn't cover it: `Cx::mark_texts` (`49265f3c`, #453; `runtime/selection/text_marks.rs`) paints byte ranges of a selectable text Rune laid out, `El::selection_style` (`c2ca2c86`, #447) recolours that selection, and `El::unselectable` (`d8ccf735`) is web-only opt-out. #353 (open) is point-to-offset hit testing and a field's selection rects, not this; nothing since. Gallery: C12's own selection, copy and silent text layer stand (`pdf_view/mod.rs:196-201`); its cursor tests pass. Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Positioned text: selectable, copyable, accessible text over glyph boxes the app places

Context: Strata (lgse/strata), a file manager on Rune.

A PDF's text layer comes with a box per character from the renderer. Rune's selection runs on
text it shapes itself, so the app needs its own selection, copy and accessible text for it, and
assistive technology hears none of the selection.

**Ask.** `El::positioned_text(text, rects)` that takes part in selection, copying and the
accessible text.

Related: #353.
