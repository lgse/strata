# 73. A `Sequence` has no reduced pose

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C20c-3 |
| Affects | C20c (settings search's found-row highlight) |
| Retires | C20c's static fill removed by the host's tick when animations are off |
| Checked | Re-checked at `2a44156` (2026-11-10) by source: `El::sequence` (`el.rs:1191-1193`) says "reduced motion and explicit settle land it immediately", and `LoopTrack::pose` (`rune-motion/src/looping.rs:156-158`) samples `frames.duration()` when reduced; `Sequence` (`rune-motion/src/sequence.rs:23`) has frames and a delay, no reduced pose. No rune-motion change since the pin touches it. Gallery: `mise run test -- c20c::` (7 tests, including `the_highlight_goes_after_a_second_and_a_half_at_rest`) passes on the host-tick static fill (`settings_search/mod.rs:92-95`). Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Under reduced motion a `Sequence` lands its final pose at once, so a highlight that fades out never shows

Context: Strata (lgse/strata), a file manager on Rune.

A settings search flashes the row it jumps to (fade in, hold, fade out). Under reduced motion,
or a settle, the sequence lands its final pose, which is "faded out", so the user never sees
which row was found.

**Ask.** `Sequence::reduced(pose, hold)`: a pose held for a time instead of the final one.
