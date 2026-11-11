# P7 Media output

| | |
| --- | --- |
| Phase / lane | 1 · Desktop (moved earlier from plan phase 5: it has no dependencies) |
| Plan | §4 P7, `preview/preview-panel/media` |
| Depends on | W0 |
| Unblocks | C13's real playback (C13 builds on fake frames meanwhile) |
| Owns | `crates/strata-platform/src/media/` |
| Feature | `media-audio` |
| Branch | `feat/<issue>-p7-media-output` |

## Goal

Play PCM with a clock that video frames can follow. Strata already decodes with FFmpeg in a
sandbox. This WP is the output side only.

## Scope

- Decide between GStreamer `appsrc` and PipeWire. Record the choice, with license, packaging
  and latency, in the module docs. The plan allows either. Neither needs GTK.
- An audio sink that takes interleaved PCM and its format, and reports a playback clock:
  position, play, pause, seek and flush.
- A frame pacer: given a clock and time-stamped RGBA frames, pick the frame to show and write
  it into a `rune::Pixels` in place. That needs no view commit, as plan P7 requires.
- Underrun and device-loss events.

## Tests

- Unit tests for the pacer against a fake clock (dropped and repeated frames).
- An ignored test for the sink, run by hand, that plays a tone.

## Done when

An example plays a generated tone and a generated video ramp in sync. Pausing and seeking
keep them in sync. The license is in `deny.toml`'s allow list.
