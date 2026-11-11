# 64. Stop a glide in place

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C1e-7 |
| Affects | C1e (keyboard and marquee scrolls), C2 (column strip glides) |
| Retires | C1e's stop by a jump to a computed target, or a glide on a stiff spring |
| Checked | Re-checked at `2a44156` by source. Every one of `Tx`'s scroll requests goes to a target (`el/tx/scroll.rs`: `scroll_to` :79, `scroll_to_instant` :99). The scroll-mode rewrite (`d751f071`, #401) adds `ScrollState::halt`, which stops momentum or a glide where it is (`runtime/scroll/offset.rs:253`), but it is `pub(super)`: only the runtime's own jumps, reveals and touch reach it. Gallery: C1e still stops a glide by a jump or a glide on a stiff spring (`collection/element/seam.rs:291`); `collection::element::tests::a_glide_effect_scrolls_the_viewport` and `entering_a_folder_stops_a_glide` pass. Earlier: by source at `5d421fd` and `04b38ae`. No program needed. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** No way to stop a running scroll glide where it is

Context: Strata (lgse/strata), a file manager on Rune.

A glide started with `scroll_to` (or Rune's reveal) can only be replaced by another request to a
target. An app that stops its own glide (a key that moves the cursor back, a marquee that
takes over) computes where the glide is and jumps there, or glides there on a stiff spring so it
doesn't jerk. Since #401 the runtime has the stop itself (`ScrollState::halt`,
`runtime/scroll/offset.rs:253` at `2a44156`), but an app can't reach it.

**Ask.** `Tx::stop_scroll(key)`: stop a viewport's glide or momentum at its drawn offset.
