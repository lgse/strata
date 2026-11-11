# 83. Comment on #102: the outline route isn't DejaVu's alone

| | |
| --- | --- |
| Kind | Comment on [#102](https://github.com/anomalyco/rune/issues/102) (default-presentation emoji render as outlines when DejaVu covers them) |
| Drafts | C20d's #102 comment draft |
| Affects | C20d (`drawn_text` adds U+FE0F) |
| Retires | C20d's `drawn_text` workaround |
| Checked | Re-checked at `2a44156` (2026-11-10) by probe and source. Probe `repros-head/src/bin/rv_83.rs`, C20d's P1 ported (one runtime, one emoji a frame, colour iff an image atlas page turns dirty) over the same 1,898-emoji TSV, headless, with Rune's bundled fonts (P1 used strata-kit's) on this machine (no DejaVu; Noto Color Emoji 2.057, Noto Sans Symbols and Symbols 2): 1,867 colour, 31 not; 23 turn colour with U+FE0F (😐 ⛪ ⛲ ⛺ ⛽ ⚓ ⛵ ⛳ ♈–♓ ⛎ 🆎 ⬜); the other 8 are flags the atlas reuses (P1's false negatives). P1's list had ⚡ where this has ⬜, so the primary face takes one or two. Source: `fallback::routes` (`text/fallback.rs:98`) routes a grapheme to the emoji stack only when it holds U+FE0F (`:109`), unchanged since the pin. #102 is open with no comments (`gh issue view`, 2026-11-10); its body says "a font setup without DejaVu hides the bug", which this contradicts. Gallery: C20d's `drawn_text` stands (`controls/icon_picker/grid.rs:197`); `mise run test icon_picker` passes, `an_emoji_draws_with_one_emoji_presentation_selector` among them. Earlier: C20d's P1 at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

From Strata (lgse/strata), a file manager on Rune: the outline route isn't DejaVu's alone. With no
DejaVu installed (Noto Color Emoji 2.057 CBDT, Noto Sans Symbols and Symbols 2), 23 of the 1,898
emoji up to Emoji 15.1 still draw as outlines, from Noto Sans Symbols or the text face. Probe, at
`2a44156`: one runtime, one emoji a frame, counted as colour when an image-atlas page turns dirty.
Every one draws in colour with U+FE0F after it, so an emoji picker adds it as it draws; a fix in
`fallback::routes` for `Emoji_Presentation` characters would cover every font.
