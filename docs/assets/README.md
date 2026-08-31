# README demo assets

The README showcase has three layers:

- `strata-screenshot.png`, `strata-grid.png`, and `strata-explorer.png` are the high-resolution source captures for the three browser modes.
- `strata-demo.mp4` is the 1920×1056, 30 fps source demo assembled from those captures. Linking the showcase image opens this high-resolution version.
- `strata-demo.gif` is the 1280×704, 10 fps, 128-color derivative displayed by GitHub when reduced motion is not requested. The columns capture is the static and reduced-motion fallback.

Run the deterministic generator from the repository root after replacing any source capture:

```bash
./scripts/generate-readme-demo.sh
```

The script requires FFmpeg with the `libx264` encoder and rebuilds both outputs. Keep the GIF small enough for GitHub loading, inspect text at the rendered README width, and confirm that its final transition loops cleanly into the first frame before committing regenerated assets.
