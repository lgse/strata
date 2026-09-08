# Compact top bar

These are unscaled 1200×260 crops of the canonical GTK 4.14 columns-view
baseline, showing the top of the same fixture window:

- `before.png`: committed baseline at `4d15322606a9026684d37a350d0977f38eb52448`.
- `after.png`: regenerated baseline with the header minimum height reduced
  from 54px to 46px. Icon sizes and button targets are unchanged.

Regenerate the after image after explicitly updating and reviewing the visual
baselines as described in `docs/e2e-testing.md`:

```bash
magick tests/e2e/baselines/gtk-4.14/columns-view.png \
  -crop 1200x260+0+0 +repage docs/screenshots/577/after.png
```
