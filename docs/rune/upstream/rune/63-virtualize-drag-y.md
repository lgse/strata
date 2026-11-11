# 63. `on_drag` on a `.virtualize` stack reports y from the mounted rows

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C1e-1 |
| Affects | C1e (all pointer input on the container) |
| Retires | C1e's content coordinates rebuilt from the window point, the viewport's recorded origin and the scroll offset |
| Checked | Re-run at `2a44156` as probe `rv_63`; the same program at `5d421fd` prints the same. It puts `on_drag` on a `virtual_rows` stack of 1,000 rows of 20 pt in a 400 pt viewport. A press at window y 100 gives `Drag::y` 100 at offset 0, and 900 at offset 5,000 (content y 5,100), where the stack draws from row 210, its first mounted row (rect y −800). `Drag`'s docs say "from its top-left as drawn" (`el.rs:119`), and no content offset is given beside it. Gallery: C1e still rebuilds content coordinates from the window point (`collection/element/route.rs:6`); `collection::element::tests::a_drag_intent_carries_the_window_point` passes. Earlier: C1e's probe at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** A `.virtualize` stack's `on_drag` reports y rebased to the first mounted row, not the stack's top

Context: Strata (lgse/strata), a file manager on Rune.

An `on_drag` on a virtual list (`.virtualize` or `virtual_rows`) reports `Drag::y` relative to
the first mounted row, so the same content point gives different y as rows mount and unmount
while scrolling. A list that hit-tests presses against its own index geometry can't use it.

**Repro** (headless, `2a44156`; the same at `5d421fd`): `on_drag` on a `virtual_rows` stack of
1,000 rows of 20 pt in a 400 pt viewport; press at window y 100: `Drag::y` is 100 at offset 0,
and 900 at offset 5,000 (content y 5,100), measured from row 210, the first mounted.

**Ask.** Report `x` and `y` in the list's content coordinates (or say in the docs which origin
they use, and give the content offset beside them).
