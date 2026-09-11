# Browser spacing and icon sizing (#804)

Before images were supplied by the owner. After images use synthetic files in a
private Xvfb/D-Bus session on the pinned GTK 4.14.5 environment. The themes and
viewport sizes differ; these are behavior/layout examples, not pixel-diff baselines.

## Pane controls

Buttons are vertically centered with top/bottom gaps matching the right inset.

| Before | After |
| --- | --- |
| ![Before: tall pane controls](columns-before.png) | ![After: compact pane controls](columns-after.png) |

## List view

The first row has the same eight-pixel gap above it as on either side. Its icon
slot aligns with the Name heading; row selection and column resizing remain intact.

![Before: List spacing](list-before.png)

![After: List spacing](list-after.png)

## Icons view

File glyphs are height-limited to the folder glyph without stretching their aspect
ratio. Photo thumbnails keep their existing sizing. Labels retain usable width
and two lines at the new 32px minimum.

| Before | After |
| --- | --- |
| ![Before: taller file glyph](icons-before.png) | ![After: balanced glyph heights](icons-after.png) |

![After: 32px Icons setting](icons-32-after.png)

To regenerate after images, create `target/ui-evidence` and run
`./scripts/test-headless.py browser_chrome_insets`. This requires the private
headless test dependencies; never use the desktop display or session bus.
