# 82. Ask whether a grapheme draws

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C20d-1 |
| Affects | C20d (emoji picker) |
| Retires | C20d's D3 (an emoji version cap instead of hiding what can't draw) |
| Checked | Re-checked at `2a44156` (2026-11-10) by source and a probe. Source: no coverage query in `TextSystem` (`text.rs:148`) or the `text` module; `fallback.rs` and `system_fonts.rs` are unchanged since the pin. Probe `repros-head/src/bin/rv_82.rs` (headless): a public `TextSystem::new` plus `glyphs` (`text/glyph_emit.rs:11`) marks colour glyphs (`GlyphQuad::color`, `text/atlas.rs:139`, at the pin too), so 😀, ♈️, 🫩 and U+1FAEA print `color=true`, while ♈ (an outline) and U+E000 and U+10FFFD (no font: a missing-glyph box) all print one quad `color=false outline=false`: colour is visible at a raster's cost, missing isn't. This machine's Noto Color Emoji 2.057 covers Emoji 17, so no real emoji shows tofu here. Duplicates: none in the 222 issues (grep coverage, missing glyph, tofu, notdef, has_glyph, `unknown_glyph`) or live (`gh issue list --search "glyph emoji draws"`: only #103). Gallery: C20d's D3 version cap stands (`controls/icon_picker/mod.rs:126`, `:159`); `mise run test icon_picker` passes (16). Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** No way to ask whether a grapheme draws (or draws in colour) before showing it

Context: Strata (lgse/strata), a file manager on Rune.

GTK's emoji chooser hides emoji Pango can't draw (`pango_layout_get_unknown_glyphs_count`). A
Rune picker shows a missing-glyph box for emoji newer than the system's emoji font: `text("🫩")`
(Emoji 16.0) with a Noto Color Emoji older than that draws tofu, and nothing in `TextSystem` says
so beforehand. Rasterising it through `TextSystem::glyphs` tells colour from not
(`GlyphQuad::color`), but a missing-glyph box and an outline come back alike.

**Ask.** `TextSystem::draws(&str) -> Option<Presentation>` (colour, outline, or missing).

Related: #102, #103.
