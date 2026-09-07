# Modifier-click focus regression

Captured from real pointer-driven scenarios on private Xvfb displays with synthetic
fixtures and the native GTK toolkit.

Sequence: click `todo.txt`, open `documents`, wait for its first row to receive
focus, then Shift-click the `todo.txt` filename in the parent column.

- `before-parent-focus.png`: the range is selected, but keyboard focus remains in
  the child; the parent selection uses inactive styling.
- `after-parent-focus.png`: the clicked parent row receives focus and the same
  range uses active styling. The child stays open.

The after capture uses modified-descending sorting with previews disabled, matching
the reported preferences. Fixture timestamps are equal, so entry order is unchanged.
The random fixture path differs between runs.
