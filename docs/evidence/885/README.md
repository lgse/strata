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
Folders, ZIP files, empty selections, and directory navigation clear unsupported
content without turning the mode off. In Icons view the preview space stays
reserved with a quiet placeholder; Columns and List hide the panel. The next
supported selection returns automatically. Space can enable the mode even on an
unsupported selection; explicit close/toggle actions disable it. Media controls
require **Ctrl+Alt**: Space plays/pauses, Left/Right seeks five seconds, Up/Down
changes volume, and M toggles mute. Plain keys keep their normal browsing behavior,
including Space to close the preview. These controls are listed in **F1 Shortcuts**.
This state is local to each window and is not saved to preferences.

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

## Stable Icons browsing area

In Icons view, preview mode—not the selected file type—controls the grid's
available width. While enabled, unsupported selections and folders show
**No preview for this selection** without moving cards into different rows or
columns. Old content and metadata are cleared immediately. Turning preview mode
off intentionally releases the space; window/divider resizing and density changes
can still reflow the grid. Very narrow windows apply a geometry-only fallback,
reserving usable browsing space before deciding whether the preview can fit.
Long directory headings are ellipsized rather than widening and horizontally
panning the grid. Columns and List retain their existing behavior.

These comparisons use `060f9d2` as the before baseline in the same pinned private
rendering environment. The supported file and ZIP selections use the same
window size and preview session:

| Selection | Before | After |
| --- | --- | --- |
| Previewable text | ![Previous grid with preview](icons/before-supported.png) | ![Stable grid with preview](icons/after-supported.png) |
| ZIP in the same enabled session | ![Previous grid reflows](icons/before-unsupported.png) | ![Reserved slot prevents reflow](icons/after-unsupported.png) |

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

## Merge-review verification

After merging main `030b1d9d`, exercised the Appearance toggle in Icons, deleted
the displayed file, then selected another file. The deleted content cleared
without releasing the reserved grid width, and the next selection restored the
preview. Captured in private Xvfb/D-Bus with the pinned GTK 4.14.5 environment
using synthetic files only.

| Appearance-opened preview | After deleting the file | Next selection |
| --- | --- | --- |
| ![Displayed file](review/appearance-preview.png) | ![Reserved empty preview](review/deleted-preview-placeholder.png) | ![Session restores preview](review/next-selection-preview.png) |

## Keyboard controls and resize-border follow-up

Media controls require Ctrl+Alt so plain arrows, Space, and typing retain their
browsing behavior. The F1 reference lists playback, seeking, volume, and mute.
The preview divider shares one normal column-colored edge, highlights on hover,
and uses the column-resize cursor. The horizontal scrollbar thumb stays inset
from that edge so it cannot intercept divider dragging.

| Before | Normal border | Hover highlight |
| --- | --- | --- |
| ![Accent divider and scrollbar against its edge](review/divider-before.png) | ![Single normal divider and inset scrollbar](review/divider-after.png) | ![Hovered preview divider](review/divider-hover.png) |

![F1 media keyboard reference](review/media-modified-shortcuts.png)

These captures use synthetic fixtures, private Xvfb/D-Bus, and the pinned GTK
4.14.5 environment. The before capture predates the resize-border changes.

## Complete scrollbar bezel

Visible horizontal scrollbars have a 1px semantic border across the full top
edge, including the space outside the thumb. The same rule covers column and
mode scrollers; hidden scrollbars do not leave a border behind. The preview
resize handle remains reachable at scrollbar height.

| Before | After |
| --- | --- |
| ![Open scrollbar bezel](review/bezel-before.png) | ![Complete scrollbar bezel](review/bezel-after.png) |

These captures use the same private-display synthetic fixture as the divider
follow-up above.

## Stable preview opening

Close-only trailing scroll space is cleared before the opening animation's first
allocation. It no longer creates a transient horizontal scrollbar that shifts
the column footer and then disappears. Real overflow continues to scroll.

[Before reopening](review/opening-before.mp4) ·
[After reopening](review/opening-after.mp4)

Both recordings use synthetic content with animations enabled in private
Xvfb/D-Bus. The regression observes every painted frame across cold opening and
reopening in the browser and chooser at wide and narrow widths; the existing
scrolling and divider-picking cases remain the coverage owners for real overflow.

Media playback also now reaches the full source duration rather than stopping at
30 seconds. The decoder, transport, and GStreamer PCM regressions cover complete
35-second playback, hour-end seeks, long GIFs, and duration/clock boundaries.
Buffer and cancellation limits remain unchanged; see the current
[media sandbox contract](../../preview-sandbox.md).

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
  placeholders. The existing in-memory preview cache still applies. After the
  #898 integration, PDFs also skip shared placeholders to preserve page geometry
  and verified page counts; file-list thumbnails are unchanged. This trades the immediate
  low-resolution placeholder on slow image/RAW loads for a reliable scale limit.
