# 77. Cursors: all-scroll, wait and progress

| | |
| --- | --- |
| Kind | New issue (beside #145, #233, #292 for resize shapes) |
| Drafts | U-C24-5, R-C14b-2 |
| Affects | C24 (middle-click autoscroll), C14b (model view rendering) |
| Retires | C24's `Move` cursor for autoscroll; C14b's default arrow while rendering |
| Checked | Re-checked at `2a44156` (2026-11-10) by source: `Cursor` (`cursor.rs:15-42`) is `Default`, `Pointer`, `Text`, `Grab`, `Grabbing`, `NotAllowed`, `Move`, `ResizeColumn`, `ResizeRow`, `Crosshair`, `ZoomIn`, `ZoomOut`, `Hidden`, `Image`; the only change since the pin is `ZoomIn` and `ZoomOut` (`776952b0`, for Markdown images), so no all-scroll, wait or progress. `Cursor::Image` (at the pin too) can draw an app's PNG but not the theme's shape. Duplicates: #145, #233 and #292 ask for diagonal resize shapes only; none since (`gh issue list --search "cursor in:title"`). Gallery: `mise run test cursor` runs `c24::the_pointer_over_the_list_shows_the_scroll_cursor_while_autoscroll_runs`, which passes asserting `Cursor::Move` (`scroll/mod.rs:126`, `:190`); C14b's default arrow while rendering stands (`viewers/model/mod.rs:149`). Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `Cursor` has no all-scroll, wait or progress shape

Context: Strata (lgse/strata), a file manager on Rune.

`Cursor` (`cursor.rs:15` at `2a44156`, which has added `ZoomIn` and `ZoomOut`) has `Move`
but not CSS `all-scroll` (the middle-click autoscroll cursor on Linux and Windows), and no
`wait` or `progress`, which a view shows while it renders. winit has all three
(`CursorIcon::AllScroll`, `Wait`, `Progress`).

**Ask.** `Cursor::AllScroll`, `Cursor::Wait` and `Cursor::Progress`.

Related: #145, #292 (resize shapes).
