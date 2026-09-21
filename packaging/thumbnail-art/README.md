# Thumbnail artwork

Strata's fallback category thumbnails are generated SVG illustrations built from unmodified Lucide geometry. They use semantic color placeholders rather than fixed colors, so the UI renderer substitutes the active built-in, custom, or Omarchy palette at runtime.

Regenerate the assets with:

```bash
python3 packaging/thumbnail-art/generate.py
```

`lucide-nodes.json` contains the curated Lucide geometry; `generate.py` owns the category mapping. The placeholder mapping is owned by `ui::thumbnail`: surface, muted, border, and the active accent. Real content previews remain content-colored. Audio spectrum thumbnails use that same accent and refresh in place when the theme changes.

Lucide is distributed under the ISC license; attribution and the complete license are recorded in `THIRD_PARTY_LICENSES.md`.
