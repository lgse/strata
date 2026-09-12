# Adaptive Miller-column previews (#885)

Automatic previews fill the space after the last directory column. The default
minimum is two standard columns (600 logical pixels); older columns scroll left
when necessary. Dragging the divider overrides automatic sizing for that window
session, including navigation and closing/reopening the preview. A new window
starts in automatic mode. Very narrow windows constrain the displayed width
without discarding the session's preferred width.

Images, GIFs, videos, and playback controls use a centered section capped at
1280 logical pixels. Media keeps its aspect ratio and grows to at most twice
its native dimensions, fitting both available axes. PDFs and text retain their
full-width viewers. Text-size settings do not change these logical limits.

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
