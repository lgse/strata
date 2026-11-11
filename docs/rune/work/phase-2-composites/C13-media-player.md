# C13 MediaPlayer

| | |
| --- | --- |
| Phase / lane | 2 · Preview |
| Plan | §4 C13, `preview/preview-panel/media`, `.../audio`, `.../video` |
| Depends on | C14a (video surface), C9. It takes P7's clock and frames as data, and builds on fake ones. |
| Unblocks | M5 |
| Owns | `crates/strata-components/src/media_player/`, `scenes/c13.rs` |
| Branch | `feat/<issue>-c13-media-player` |

## Goal

The media preview UI, driven by a clock and frames that it does not produce.

## Scope

- A video surface: a C14a viewport whose `Pixels` the host (P7's pacer) updates in place.
- An audio view: artwork, tags, and a live spectrum drawn with `canvas` shapes from band
  data.
- A waveform scrubber with chapter ticks, and storyboard frames on hover.
- Transport controls, and the timeline key map shared with quick preview (a key map as data).
- A hand-off intent: open in the default player at the current position.
- A trait for the playback clock (position, duration, state), so P7 and a fake both fit.

## Tests

Scripts with a fake clock: play, pause and seek by the scrubber and by keys, the storyboard on
hover, chapter ticks, and the hand-off intent. A11y golden: a slider for the timeline, and
named transport buttons.

## Done when

The media bullets pass headless with fake data. A hand test with P7 is recorded once P7 has
landed.
