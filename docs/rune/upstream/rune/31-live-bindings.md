# 31. Live values for more than rects, opacity and text

| | |
| --- | --- |
| Kind | New issue (related #420, #421) |
| Drafts | U-C13-1, U-C13-3, U-C16-2, U-C16-3, R-C4-4 |
| Affects | C13 (spectrum, waveform), C16 (progress bars), C4 (column resize) |
| Retires | C13's 96 `live_rect` bars in place of a shape; C16's track width read back through `on_layout_tx` and its four-a-second accessibility commits; C4's view commit on every resize move |
| Checked | Re-checked at `2a44156` by source: the bindable properties are the same set as at the pin (`live.rs:227`: opacity, offset, scale, fill, rect, blur, trim, and text, border, shadow and gradient colours). The colour bindings (`el/live_colors.rs:9–27`) came in `a53f5fe4`, before the pin, and cover none of these asks. Still nothing binds a shape's points, rotation (`LoopSpec::rotation`, `rune-motion/src/looping.rs:36`, turns icon quads only), a fractional width (`live_rect` is in points, `el.rs:882`), a column width, or `El::range_value` (`el/a11y.rs:146`, set at build). rune-kit's `Progress::new` still takes a plain `Option<f32>` (`rune-kit/src/feedback.rs:125`). #420 (open) already asks for live rotation, so that ask is cut here and the body points to it. In the gallery, `mise run test a_resized_track_moves_the_fill` and `committed_text_lands_at_most_four_times` passed (1 test each: C16's track width read back and its four-a-second commits). C13's notes (`media_player/mod.rs:215–231`), C16's (`progress_dock/mod.rs:318–323`) and C4's (`list_table/mod.rs:264`) are still there. No program needed. Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file, trimmed: live rotation is dropped from the ask, since #420 (open) already asks for it; the body points to #420. Re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10. |

Paste from here.

---

**Title:** Live values: bind shape points, a fractional width, a column width and an accessibility value without a view commit

Context: Strata (lgse/strata), a file manager on Rune: its media preview (spectrum, waveform),
progress bars, and a list table whose columns resize by dragging. `Live` and `LiveText` move rects,
offsets, scale, opacity, blur, a shape's trim, colours and text without running the view; these
can't be moved that way at `5d421fd` (unchanged at `2a44156`):

- **Shape geometry.** A shape's points are set at build, so a waveform that grows in, or a
  spectrum drawn as shapes, rebuilds the view each frame (96 bars need 96 `live_rect` elements
  instead).
- **A fractional width.** A progress fill can't follow a `Live<f32>` fraction of its track:
  `live_rect` is in points, so the track's width comes back through `on_layout_tx`, and
  rune-kit's `Progress` takes no live value.
- **An accessibility value.** A progress bar's `range_value` is set at build, so assistive
  technology hears it only on commits. `LiveText` does reach the tree; nothing binds a range.
- **A column width.** Resizing a table column commits the view on every move to give each
  mounted row's cell its width: 30 rows of 5 cells, a 100-frame drag, 100 view runs.

**Ask.** Live bindings for these: shape points (or a polygon's scale on one axis),
`El::live_w_frac(&Live<f32>)` (or `Progress::live`), `El::live_range_value`, and a live width
that cells follow (as `live_offset` is a live position). Live rotation on any element, not only
icon quads, is #420's ask; a media preview needs it too.

Related: #420, #421, #301 (a small live rect re-rasters the window).
