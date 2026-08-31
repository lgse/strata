# README product assets

The README showcase is built from four high-resolution product screenshots:

- `strata-columns.png` — Miller-column navigation;
- `strata-search.png` — recursive fuzzy search with thumbnail results;
- `strata-settings.png` — general preferences; and
- `strata-themes.png` — Omarchy following, bundled themes, and custom themes.

`strata-demo.gif` is the 1280×696, 10 fps, 128-color animation displayed by GitHub when reduced motion is not requested. The columns screenshot is its static and reduced-motion fallback.

Run the deterministic generator from the repository root after replacing a source screenshot:

```bash
./scripts/generate-readme-demo.sh
```

The script requires FFmpeg. Keep the GIF small enough for GitHub loading, inspect text at the rendered README width, and confirm that its final transition loops cleanly into the first frame before committing regenerated assets.
