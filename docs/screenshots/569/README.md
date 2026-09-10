# Open With follow-ups

Cropped captures from private Xvfb sessions, using the isolated `chooser_apps`
fixture in `tests/e2e/scenarios/test_open_with.py` on the same native GTK toolkit.
No user MIME associations were changed.

- **Before** (`4d15322`): the NoDisplay default, Review Text Viewer, is absent;
  Alternative Viewer has an unresolvable icon and shows a broken-image tile.
- **After**: the default appears first, both missing-icon cases use the themed
  fallback, and Tab reaches Open without visiting every remaining application.

The row-name regression assertion also fails against the before build and
passes after the fix. The pinned-container suite is the separate E2E gate.
