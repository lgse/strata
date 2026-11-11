# 26. Linux: play app-decoded PCM with a playback clock, without `Player`

| | |
| --- | --- |
| Kind | New issue (related #30, #100, #112) |
| Drafts | U-P7-1 |
| Affects | P7 (media audio), C13 (media player) |
| Retires | P7's GStreamer `appsrc` sink and its clock |
| Checked | Re-checked at `2a44156` by source: `Player` off macOS still uses `video/unsupported.rs` (`video/mod.rs:38–41`), which fails every player at once (`unsupported.rs:15–16`). `MediaSource` is still `File` or `Url` only (`video/mod.rs:102–105`), and no crate in the workspace takes PCM (the only other audio code is `Recorder`, which captures). #30, #100 and #112 are still open, with #506 (`Player` volume) and #366 (a Windows backend) beside them; none asks for PCM. P7's GStreamer `appsrc` sink is still there (`strata-platform/src/media/mod.rs:71`, `:85–87`). No program needed. Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Linux: play app-decoded PCM with a playback clock

Context: Strata (lgse/strata), a file manager on Rune, previews media by decoding in a sandboxed
helper that streams RGBA frames and PCM back. The frames already work (`Pixels::set` from any
thread, no view commit). The sound has nowhere to go: `Player` takes a file or URL and has no Linux
backend (#30, #100, #112), and Rune has no API that takes PCM.

**Steps.** At `2a44156`, look for a way to play a buffer of interleaved S16 stereo at 48 kHz and
read back how much of it has been heard: there is none outside `Player`, whose Linux backend
reports "unsupported" (`video/mod.rs:38–41`, `video/unsupported.rs:15–16`), and whose
`MediaSource` is a file or a URL.

**Ask.** A small output: open it with a sample format, rate and channel count; push interleaved
PCM; play, pause and flush; read a position that follows the device (what has been heard, its
delay counted); and a callback or waker for underruns, the end, and a lost device. Video then
paces on that position. A Linux backend for `Player` wouldn't cover it: the app has PCM, not a
file.

Related: #30, #112, #506 (`Player` volume).
