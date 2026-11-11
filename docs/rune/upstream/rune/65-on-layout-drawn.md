# 65. `on_layout` reports a layout spring's end, not where the frame draws

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C2-3 |
| Affects | C2 (column hit tests during a width spring) |
| Retires | C2's drop targets ahead of what is drawn while a column's width springs |
| Checked | Re-run at `2a44156` as probe `rv_65` (C2's `p6`, extended); the same program at `5d421fd` prints the same. Over 12 frames, the second column's drawn x goes 310, 331, 356, 381, 404 … 484 while its `on_layout` hears 500 once. `on_drawn_layout` heard nothing in those frames: it reports where a spring comes to rest (`el/layout_watch.rs:78`). `El::drawn_rect_into(&Live<[f32; 4]>)` (`el/layout_watch.rs:132`, there at the pin as well) held the drawn x on every frame, the same values as `ElementInfo::rect`. That is the ask's "drawn counterpart". Gallery: C2's spans still come from layout (`columns/mod.rs:177`, `:350`); `columns::tests` and `c2::` pass. Earlier: C2's probe `p6` at the pin. |
| Status | Drop: already in Rune at the pin. `El::drawn_rect_into` writes an element's drawn rect on every frame of its own or an ancestor's spring (probe `rv_65`), the drawn counterpart the ask names. C2 can read its columns' drawn spans through it; that is a C2 follow-up, not an upstream gap. |

Paste from here.

---

**Title:** During a layout spring, `on_layout` hears the final rect once while the element is drawn moving

During a width spring, a sibling's `on_layout` hears its final rect once, while
`ElementInfo::rect` moves over several frames, so an app hit-testing by layout is ahead of what
is drawn.

**Repro** (headless, `5d421fd`): two columns in a row; widen the first from 300 to 500 in an
animated commit: the second's `on_layout` hears x 500 at once, while its drawn x goes 310, 381,
440, 473 over the next frames.

**Ask.** The drawn rect in `on_layout` (or a drawn counterpart), or a flag saying a spring is
moving it.
