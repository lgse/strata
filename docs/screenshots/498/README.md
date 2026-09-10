# Long file chooser filter lists

Captured on a private D-Bus session and a headless 1440x900 display, with an OpenFile
request carrying 80 filters:

```bash
python3 scripts/portal-test.py filters --binary target/debug/strata --filter-count 80
```

- `filter-dropdown-before.png`: clicking **Filter** produces no visible list. The popover is
  sized to all 80 rows and is placed outside the screen, so the options cannot be reached.
  On a desktop session the same list appears clipped at the top and bottom screen edges.
- `filter-dropdown-after.png`: the list is bounded to the room available beside the button,
  opens upward because the control sits near the window bottom, and scrolls through the
  remaining filters.
