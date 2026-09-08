# PR #587 verification

Captured from the reviewed build in the pinned GTK 4.14 E2E container on a
private Xvfb display, using disposable fixture files.

- `before-duplicate.png`: `todo.txt` selected before pressing Ctrl+D.
- `after-duplicate.png`: the original remains and `todo (1).txt` is selected.
- `after-self-drop.png`: dropping `documents` onto itself leaves the folder
  intact with no transfer dialog. The two numbered files came from the preceding
  Duplicate and Copy-to-current-folder checks.

These are before/after action captures of the fixed build, not captures of the
original broken implementation. Duplicate, Copy to the current folder, and folder
self-drop were exercised in Columns, Icons, and List views; copied contents and
the unchanged self-drop fixture were checked on disk.
