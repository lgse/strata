# Balanced Icons tile padding (#600)

[Before](before.png) · [After](after.png)

Native GTK widget captures on a private Xvfb display, using the application's
hover background. Rows show 64, 128, and 256 px slots; columns show a short name
and a wrapping/ellipsized name. The before capture restores the original
full-slot icon rendering and top-aligned filename on the same cards.

The icon now has a nine-logical-pixel rendering inset and short filenames are
centered within the existing two-line label area. Card and thumbnail slot
measurements are unchanged; long filenames retain their two-line allocation.

The actual hovered browser tile is also covered by the canonical GTK 4.14
[`icons-hover` visual baseline](../../../tests/e2e/baselines/gtk-4.14/icons-hover.png).
