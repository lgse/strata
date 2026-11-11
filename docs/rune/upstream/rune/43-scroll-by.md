# 43. `Tx::scroll_by`, a viewport's offset, and arrow keys that scroll a focused viewport

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C19-1, U-C7-2 (its offset and `scroll_by` half) |
| Affects | C19 (shortcut reference), C7 (tab strip wheel) |
| Retires | C19's row-step reveals for arrow scrolling; C7's offset read back from where its content drew |
| Checked | Re-checked at `2a44156`: the body's repro, re-run as a probe (`repros-head/src/bin/rv_43.rs`: a focusable `.scroll(..)` column of 100 rows of 20 pt, focused by Tab), prints the offset `0.0` after Down, Down, `180.0` after Page Down, and still `180.0` after Up. By source, the focused scroll view's key routing is still `page_scroll` for Page Up, Page Down, Home and End only (`runtime/input/keys.rs:114–131`), and `Tx` still has no `scroll_by` (`el/tx/scroll.rs`: `scroll_page`, `scroll_to` and the rest, the same as at the pin) and `Cx` no `scroll_offset`. In the gallery, `mise run test c19` passed (10 gallery tests and 3 more), and C19's row-step reveals are still there (`footer/mod.rs:258`, `footer/reference/keys.rs:101`). Earlier: by source at `5d421fd` and `04b38ae`, and C19's `--tree` probe at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Up and Down don't scroll a focused scroll view, and there's no `Tx::scroll_by` or way to read a viewport's offset

Context: Strata (lgse/strata), a file manager on Rune.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
- Up and Down don't scroll a focused scroll view (the key routing takes only Page Up, Page Down,
  Home and End for it). Repro: a focusable `.scroll(..)` column of 100 rows of 20 pt; focus it,
  press Down; its offset stays 0 (Page Down moves it to 180).
- `Tx` has no `scroll_by(key, dx, dy)`, and no API reads a viewport's offset, so a list that
  scrolls by arrows without a row cursor reveals rows itself, and an app mapping a wheel by hand
  reads where its content drew.

**Ask.** Arrows that scroll the focused viewport by a line when it overflows, as Page Down does
by a page; `Tx::scroll_by(key, dx, dy)`; and `Cx::scroll_offset(key)`.

Related: (link to 10).
