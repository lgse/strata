# 52. Comment on #365: a scroll bar with a trough, apart from its viewport

| | |
| --- | --- |
| Kind | Comment on [#365](https://github.com/anomalyco/rune/issues/365) (always-visible thin scroll bar with track inset and minimum thumb length) |
| Drafts | R-C24-2 |
| Affects | C24 (its own overlay bar) |
| Retires | C24's own bar (with 51) |
| Checked | At `2a44156`: #365 open, no comments (live, 2026-11-10). Probe run (`repros-head/src/bin/rv_52.rs`, headless): in a 600 pt `scroll_x` viewport holding a 900 pt `.scroll()` viewport with `.scroll_bar(&kit)`, the strip spans x 886–900. Source (`rune-kit/src/scroll_bar.rs`): `ScrollBarStyle` has no track colour (fields `:67`–`73`), the inset and least thumb are set only by the presets (no public setter beside `track_offset`, `:275`), and the bar wraps its viewport (`attach`, `:779`; placed at `viewport[1 - a] - end`, `:493`). `96def2a2` changed only the press. Gallery: C24's own bar is still there (`scroll/mod.rs:107`). Earlier: C24b at the pin and at `dev` 111 commits on; read at `04b38ae` (only right-to-left had changed, `0f12557b`). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

Two more needs from Strata's (lgse/strata, a file manager on Rune) scroll bars, beside the inset and
minimum thumb:

- **A track fill.** Strata-style bars draw a trough (and a hover fill for it). No
  `ScrollBarStyle` field sets a track colour.
- **A bar placed apart from its viewport.** The bar wraps the viewport it scrolls (`attach`,
  placed at `viewport[1 - a] - end` across it), so a vertical bar can't sit at the visible edge
  of a vertical viewport inside a wider sideways one: in a 600 pt `scroll_x` viewport holding a
  900 pt `.scroll()` viewport with `.scroll_bar(&kit)`, the strip lands at the inner
  viewport's right edge (x 886 to 900 at `2a44156`), 300 pt past the view.

Could a bar take a `ScrollPos` and a target to scroll, and let the app place it? With
`track_inset` and `min_thumb` on any style (as asked here) and track colours, an app could use
rune-kit's bar in both cases.
