# 22. Comment on #21: what freedesktop icon lookup needs for a file manager

| | |
| --- | --- |
| Kind | Comment on [#21](https://github.com/anomalyco/rune/issues/21) (app icons off macOS) |
| Drafts | U-P6-1 |
| Affects | P6 (icon theme lookup), C21 (Open With), file-type icons everywhere |
| Retires | P6's GTK-4-style lookup (search path, `Inherits`, `index.theme` sizes, its worker thread and LRU) |
| Checked | Re-checked at `2a44156`, by running the fixture below headless (`rv_22`, calling `rune::image::app_icon_rgba`, which `app_icon_scaled` runs on its worker): the theme-only icon gives `Err(no icon for $T/share/applications/fixture.desktop)`, and the same PNG under `icons/hicolor/48x48/apps` is found (48×48). By source, `freedesktop::icon_file` (`image.rs:500`) still scans `icons/hicolor` only (`:508`), takes sizes from directory names (`:510–516`), looks in `apps/` only (`:520`), and a missing icon and a decode failure are both an `ImageError` (`:436–444`). #21 is still open; its one comment (2026-10-26) already reports the missing current theme and `Inherits` (it cites a PR #29 and commit `35329d7`; Rune takes no PRs and the commit isn't in its history), so that point is cut to a pointer. In the gallery, P6's own lookup is still there (`strata-platform/src/icon_theme/mod.rs:21`, U-P6-1); the `icon_theme` tests (43) passed. Earlier: by source at `5d421fd` (`image.rs:481–489`). |
| Status | Ready to file, trimmed: the current theme and `Inherits` are already in #21's 2026-10-26 comment, so that bullet now points to it; the rest is new to the thread (re-verified at 2a44156, 2026-11-10) |

Paste from here.

---

Strata (lgse/strata), a file manager on Rune, shows other apps' icons (Open With) and MIME-type
icons, and hits these limits of `ImageCache::app_icon` on Linux at `2a44156`:

- **The user's theme.** As the 2026-10-26 comment found, `freedesktop::icon_file`
  (`image.rs:500`) still scans `icons/hicolor` only (`:508`).
- **`index.theme` sizes.** It reads a size from the directory name (`48x48`, `@2`), not the
  theme's `Size`, `Type`, `Threshold`, `MinSize`, `MaxSize` and `Scale`.
- **Lookup by name.** It takes a `.desktop` path only, and looks in `apps/` only, so a MIME icon
  name (`text-x-python`, in `mimetypes/`) or a list of fallback names (as `GThemedIcon` gives)
  can't be looked up.
- **Not found.** A missing icon and a broken file both resolve to `Image::error`.
- It lists every size directory again on each call.

Headless, with `XDG_DATA_HOME=$T/share`:

```sh
mkdir -p $T/share/icons/Fixture/48x48/apps $T/share/applications
printf '[Icon Theme]\nName=Fixture\nInherits=hicolor\nDirectories=48x48/apps\n\n[48x48/apps]\nSize=48\nType=Fixed\n' \
  > $T/share/icons/Fixture/index.theme
cp any-48px.png $T/share/icons/Fixture/48x48/apps/fixture-app.png
printf '[Desktop Entry]\nType=Application\nName=Fixture\nExec=true\nIcon=fixture-app\n' \
  > $T/share/applications/fixture.desktop
```

`ImageCache::new(waker).app_icon_scaled("$T/share/applications/fixture.desktop", 48.0, 1.0)`
fails with `no icon for $T/share/applications/fixture.desktop` at `2a44156` (we ran
`rune::image::app_icon_rgba`, which it runs on the worker), while the same PNG under
`icons/hicolor/48x48/apps` is found. Nothing names a theme to look in.

What would cover it: an image source taking the theme by name,
`ImageCache::themed_icon(names, theme, size_pt, scale)`, following the Icon Theme
Specification, with "not found" apart from a decode error. Or, smaller: a way to run a path
lookup on the cache's worker (`cache.file_with(|| -> Option<PathBuf>, size_pt, scale)`), so a
crate doing the lookup needs no thread of its own.
