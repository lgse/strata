# Compact directory subheaders (#588)

Before: commit `36fe7dd7d64b2d6fb4756737f5f2ebd0436d2314`.
After: the approved 40px icon/list toolbars, with centered 32px buttons,
matching 16px icons, and the filter vertically aligned with the window close icon.
Heights exclude the 1px bottom border; the main header remains 46px.

These are unscaled 1200×300 crops from the pinned GTK 4.14 icon/list visual
baselines, showing the same synthetic fixture. The original captures are in
`tests/e2e/baselines/gtk-4.14/` at the corresponding revision.

Refresh the baselines with `STRATA_E2E_UPDATE_BASELINES=1 ./scripts/e2e.sh -k baseline`
(`STRATA_CONTAINER_ENGINE=podman` for rootless Podman), then crop with:

```sh
for mode in icons list; do
  magick "tests/e2e/baselines/gtk-4.14/$mode-view.png" \
    -crop 1200x300+0+0 +repage "docs/screenshots/588/$mode-after.png"
done
```
