# 71. An offstage subtree: kept mounted, out of layout, input and the tree

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C20c-2 |
| Affects | C20c (tabbed form's kept pages) |
| Retires | C20c's kept pages as opacity 0, `inert` and `a11y_hidden` in a shared grid cell, still laid out every frame |
| Checked | Re-checked at `2a44156` by source: no builder keeps a subtree mounted while taking it out of layout, hit testing, Tab order and the tree (no `offstage`, keep-alive or `display: none` counterpart in `El`; `inert`, `a11y_hidden` and `opacity` remain the parts). Gallery: C20c's kept pages are still opacity 0, `inert` and `a11y_hidden` in a shared grid cell (`controls/tabbed_form/mod.rs:56`); `controls::tabbed_form::tests::a_hidden_kept_page_takes_neither_focus_nor_the_tree` and `a_kept_page_keeps_its_scroll_offset_and_a_rebuilt_one_starts_at_the_top` pass. Earlier: by source at `5d421fd` and `04b38ae`. No program needed. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `El::offstage(bool)`: keep a subtree's state while taking it out of layout, hit testing, Tab order and the tree

Context: Strata (lgse/strata), a file manager on Rune.

A tabbed settings window keeps each page's scroll offsets, focus memory and measured rows while
another page shows. Nothing does that: today a hidden page is opacity 0, `inert` and
`a11y_hidden` in a shared grid cell, which still lays it out every frame.

**Ask.** `El::offstage(bool)`, as SwiftUI's hidden `TabView` pages or a web `display: none` that
keeps state.
