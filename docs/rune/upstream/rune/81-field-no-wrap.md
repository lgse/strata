# 81. A multi-line text field that doesn't wrap

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C10b-4 |
| Affects | C10b (script editor with wrap off) |
| Retires | C10b's field made wider than its widest line inside a sideways viewport |
| Checked | Re-checked at `2a44156` (2026-11-10) by source: `text_input` (`el.rs:673`) sets `t.wrap = true` (`el.rs:683`) and `set_single_line(false)`; only `El::single_line` (`el/text.rs:378`) sets `wrap = false`, and it makes the field one line; `El::wrap` (`el/text.rs:261`) only turns wrapping on. Duplicates: none in the 222 issues (grep no-wrap, nowrap, `wrap(false)`, soft wrap, a field scrolling sideways) or live (`gh issue list --search "wrap text_input"`: nothing). Gallery: C10b's wide field in a sideways viewport stands (`code_view/editor.rs:16-17`; its width from the widest line's columns, `editor.rs:353-356`; `code_view/mod.rs:154`). No gallery test checks the editor's wrap-off width (a `wrap` filter matched only the read-only view's wrap-off tests, entry 17's workaround); `mise run test c10::` passes (25 tests, among them the editor's `ctrl_tab_leaves_the_editor`, plus a11y, png and scripts). Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** A multi-line `text_input` always wraps at its width

Context: Strata (lgse/strata), a file manager on Rune.

A code editor without wrap has to make its multi-line field wider than its widest line inside a
sideways viewport, because `text_input` sets `wrap` and only `single_line` turns it off.

**Ask.** `El::no_wrap` (or `wrap(false)`) for a multi-line field, scrolling sideways to keep the
caret in view.
