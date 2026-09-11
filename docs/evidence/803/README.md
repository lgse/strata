# PR #803 review evidence

Captured from the real application on private Xvfb and D-Bus using the pinned
GTK 4.14.5 E2E environment. These are automated real-input captures, not a
manual desktop session.

The regression selects `001.txt` and `003.txt`, wheels down, then updates and
renames another visible file from outside Strata. It asserts that the scroll offset stays
unchanged and the old filename disappears. It then wheels back to the top to
verify both selected rows, including rows previously outside GTK's visible
accessibility tree. The screenshots show that final state:

- Columns: [before restricting passive focus restoration](columns-before.png),
  [after](columns-after.png)
- Icons: [before restricting passive focus restoration](icons-before.png),
  [after](icons-after.png)
- [List after](list-after.png)

The before captures are from the failed scroll-preservation assertions, before
wheeling back up. They demonstrate the review regression, not pixel-comparison
baselines; fixture paths differ between runs.

A second regression inserts an entry before the keyboard cursor and verifies
that the next-entry key still selects the expected file in all three views.

Reproduce with:

```bash
STRATA_CONTAINER_ENGINE=podman ./scripts/e2e.sh tests/e2e/scenarios/test_monitor_selection.py --keep-artifacts
```

For a manual check, open a directory with at least 100 files in each view,
select two files, scroll down, and rename another file from a terminal. The
listing should not jump; scroll back up to confirm the selection remains.
