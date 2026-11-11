# 94. The single-colour icon rasterizer ignores dashes, miter limits and clip paths

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-K3-3 |
| Affects | K3 (icon catalogs drawn under Theme colors) |
| Retires | The three Catppuccin icons listed as drawn differently under Theme colors |
| Checked | Re-checked at `2a44156` with a probe (`repros-head/src/bin/rv_94.rs`, `--snap` offscreen, compared by pixel): each `currentColor` body with `stroke-dasharray`, `stroke-miterlimit` or `clip-path` draws the same pixels as the body without it (0 of 36,864 differ), while the same bodies in `#ffffff`, which go through resvg, differ (2,048, 272 and 12,800 pixels). By source: the single-colour rasterizer reads only the subset its module docs list (`text/icon.rs:1–4`; strokes at `:253–267`, a miter join with no limit read), and `El::icon_one_color` sends only bodies that name their own colours through resvg, never a `currentColor` one (`text/color_icon.rs:60`). Gallery: K3's table of the three attributes is still there (`crates/strata-kit/icons/README.md:52–62`). Earlier: from K3's catalogue at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Icons drawn through the single-colour rasterizer lose `stroke-dasharray`, `stroke-miterlimit` and `clip-path`

Context: Strata (lgse/strata), a file manager on Rune.

Icon bodies drawn in one colour (tinted to the text colour) go through Rune's own rasterizer,
which ignores three SVG attributes that resvg (the colour path) keeps:
- `stroke-dasharray`: a dashed line draws solid;
- `stroke-miterlimit`: Rune's default limit is used;
- `clip-path`: drawn unclipped.

**Repro** (at `2a44156`, offscreen with `--snap`): draw each body below with `rune::icon(&ICON,
96.0)` beside the same body without the attribute. Each pair draws the same pixels. With
`stroke="#ffffff"` or `fill="#ffffff"` in place of `currentColor` (so resvg draws them), each
pair differs.

```
<path d="M2 12h20" stroke="currentColor" stroke-width="4" stroke-dasharray="4 4" fill="none"/>
<path d="M8 22L12 6L16 22" stroke="currentColor" stroke-width="2" stroke-miterlimit="10" fill="none"/>
<defs><clipPath id="c"><rect width="12" height="24"/></clipPath></defs><rect x="2" y="2" width="20" height="20" fill="currentColor" clip-path="url(#c)"/>
```

The second has a miter ratio of about 5.1: SVG's default limit, 4, bevels it, and 10 keeps the
point.

**Ask.** Support those three in the single-colour path, or draw single-colour icons through
resvg with the colour substituted.
