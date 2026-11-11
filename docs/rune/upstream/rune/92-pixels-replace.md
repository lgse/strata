# 92. `Pixels::set` drops the image it replaces

| | |
| --- | --- |
| Kind | New issue (related #148) |
| Drafts | U-P7-2 |
| Affects | P7 (frame pacer) |
| Retires | A 3.7 MB allocation and free per 720p frame |
| Checked | Re-checked at `2a44156` by source: `Pixels::set(w, h, rgba: Vec<u8>)` (`pixels.rs:90`) still assigns `frame.rgba = rgba` under the lock (`:96`), dropping the old buffer; the public API is still `new`, `source_url`, `source_set`, `set` and `size`, with no replace or in-place fill. `pixels.rs` changed past the pin only for animated images in the atlas. Gallery: P7's `FramePacer` still hands each frame's `Vec` to `set` (`crates/strata-platform/src/media/pacer.rs:225`), and the module docs cite U-P7-2 (`media/mod.rs:8–9`). #148 (read `Pixels` back) is still open and asks for a borrow, not the buffer back. No program needed. Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `Pixels::set` drops the image it replaces, so a video stream allocates every frame

Context: Strata (lgse/strata), a file manager on Rune.

Stream 1280×720 RGBA at 30 fps into one `Pixels` with `set(w, h, rgba)`: each call takes a new
`Vec<u8>` and drops the previous one under the lock, so a producer allocates and frees 3.7 MB a
frame and can't reuse a buffer.

**Ask.** A way to get the replaced buffer back (`Pixels::replace(w, h, rgba) -> Vec<u8>`) or to
fill a buffer in place (`Pixels::update(w, h, |rgba| …)`), so a pacer recycles its frames.

Related: #148 (read `Pixels` back).
