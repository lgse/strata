# Adaptive Miller-column previews (#885)

Automatic previews fill the space after the last directory column. The default
preferred minimum is two standard columns (600 logical pixels); older columns
scroll left when necessary. The focused directory column, including its resized
width, takes priority over that minimum and over a manually chosen preview width.
Explicitly closing a preview preserves the columns' positions and leaves empty
space on the right; navigation can still reveal a newly focused column.

Dragging the divider overrides automatic sizing for that window session,
including navigation and closing/reopening the preview. A new window starts in
automatic mode. If less than one standard column (300 logical pixels) remains for
the preview after reserving the focused directory column, the preview is temporarily
hidden. It returns when space permits without discarding the preferred width or
the latest selection. Existing document views retain their scroll/zoom state.
Loaded media is paused and resumes only if that same file was playing before
hiding. Late media results never autoplay while hidden. Closing a hidden preview
cancels automatic restoration.

Images, GIFs, videos, and playback controls use a centered section capped at
1280 logical pixels. Media keeps its aspect ratio and grows to at most twice
its native dimensions, fitting both available axes. PDFs and text retain their
full-width viewers. Text-size settings do not change these logical limits.

## Session toggle and neighboring columns

**Appearance → Preview panel** shows the **Space** browsing shortcut and stays
checked while preview mode is enabled, even if no preview is currently visible.
Folders, ZIP files, empty selections, and directory navigation hide unsupported
content without turning the mode off. The next supported selection returns
automatically. Space can enable the mode even on an unsupported selection;
explicit close/toggle actions disable it. Existing media playback shortcuts are
unchanged. This state is local to each window and is not saved to preferences.

Column navigation adapts the 48-logical-pixel peek idea from
[PR #876 by JoeJoeflyn](https://github.com/lgse/strata/pull/876). Real neighboring
columns remain discoverable with the mouse. Clicking an exposed strip only
reveals/focuses that column, including double-clicks; it does not activate hidden
rows or toolbar actions. Fully visible columns keep their normal interactions.
Peeks are best-effort: they yield before a full focused column or an otherwise
usable preview, and trailing empty space is not treated as another column.

| Before this follow-up (`9fa802d`) | After |
| --- | --- |
| ![Earlier-column focus without neighbor peeks](session/before-peeks.png) | ![Focused column with mouse-accessible neighbors](session/after-peeks.png) |
| ![Previous Appearance menu](session/before-appearance.png) | ![Enabled session toggle and Space hint](session/after-appearance.png) |

## Before / after

Synthetic files only. Captured with the pinned GTK 4.14.5 E2E environment,
private Xvfb/private D-Bus, 1× display scale, Medium text, and reduced motion.
Before: `e598625`. After: the implementation in this PR. Wide/small examples
use a 2300×900 window; the deep path uses 1200×760. Captures are cropped to the
application window, not taken from the desktop.

| Example | Before | After |
| --- | --- | --- |
| Remaining space and centered content cap | ![Wide preview before](before-wide.png) | ![Wide preview after](after-wide.png) |
| 160×48 image, at most 320×96 in the preview | ![Small image before](before-small.png) | ![Small image after](after-small.png) |
| Deep path, last column remains visible | ![Deep path before](before-deep.png) | ![Deep path after](after-deep.png) |

## Closing and narrow-window follow-up

These comparisons use `8554812` as the before baseline, with the same private,
pinned rendering environment described above.

| Window | Before | After |
| --- | --- | --- |
| 900 pixels wide: last column takes priority | ![Column obscured before](followup/before-narrow.png) | ![Last column fully visible after](followup/after-narrow.png) |
| 760 pixels wide: temporary preview fallback | ![Preview blocks the browser before](followup/before-hidden.png) | ![Preview hidden to preserve navigation](followup/after-hidden.png) |

Closing leaves the columns in place; the released space is empty rather than
scrolling the columns back to the right:

| Open | Closed |
| --- | --- |
| ![Column positions with preview open](followup/after-open.png) | ![Same column positions after closing](followup/after-closed.png) |

## Tuning and scope

- `MIN_COLUMN_MULTIPLIER` in [`preview/layout.rs`](../../../src/ui/preview/layout.rs)
  controls the automatic minimum independently of individually resized columns.
- `MAX_CONTENT_WIDTH` and `MAX_UPSCALE` in
  [`preview/media_layout.rs`](../../../src/ui/preview/media_layout.rs) control
  the media section and enlargement limits. These are internal defaults, not
  new saved settings.
- The browser and chooser share the sizing binding. Icons and List retain their
  single-pane automatic reservation; manual widths and media limits work there
  too. No desktop, display, or global text-scaling preferences are changed.
- Image decoders no longer enlarge small source files before handing them to the
  preview. Normalized shared thumbnails do not retain reliable native dimensions,
  so image previews wait for the bounded full render instead of using those
  placeholders. The existing in-memory preview cache still applies; PDF
  placeholders and file-list thumbnails are unchanged. This trades the immediate
  low-resolution placeholder on slow image/RAW loads for a reliable scale limit.
