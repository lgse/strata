# 76. A handler can't read a layout

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C23-4 |
| Affects | C23 (effects started from a handler) |
| Retires | C23's frame spent recording where an effect's sources are |
| Checked | Re-checked at `2a44156` (2026-11-10) by source: `impl Tx` (`el/tx.rs:151` on) has no `layout` or `presented`. A rect is read only while the view builds (`Cx::layout`, `runtime/layout_read.rs:194`; `rune::memo::layout`, `memo.rs:483`), by the host (`Runtime::presented`, `runtime/elements.rs:402`), or from `El::on_layout` (`el/layout_watch.rs:36`); all were at the pin. No duplicate in the 222 issues (grep `Tx::layout`, `presented`, handler and rect). Gallery: C23's deviation 7 (the host holds the source rows a frame after `Effects::fly`, `motion/mod.rs:105`) stands; `mise run test -- c23::` passes. Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `Tx` can't read where an element is laid out or drawn

Context: Strata (lgse/strata), a file manager on Rune.

An effect started from a key or press handler (fly these items to the Trash) needs where its
sources are now. `Tx` has no `layout(key)` or `presented(key)` (a view reads `Cx::layout`
while it builds, and the host `Runtime::presented`), so the app records rects from
`on_layout` beforehand or spends a frame finding them.

**Ask.** `Tx::layout(key)` and `Tx::presented(key)` (the rect as last drawn).
