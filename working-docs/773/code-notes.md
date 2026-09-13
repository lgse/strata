# Round 1 code notes: #773 cover-art audio vs hardware preview

## What changed

Latest `main` no longer has the plan’s encode path (`input_has_video`, `-select_streams v`, `-map 0:v:0`, `h264_vaapi`, Opus WebM). `preview-media` streams raw decoded frames (`src/sandbox_helper/media.rs`, #839). Cover-art FLAC/M4A still became a video track because probe metadata fell back to the first `attached_pic` stream.

The equivalent of “probe/map with `V` and take the audio-only path”:

- Select preview video only from streams that are not `attached_pic` (no fallback to the still).
- Cover-art-only files now have `video: None` and follow the existing audio-only decoder (PCM, width 0, software only, no VAAPI/Vulkan).
- Movie + cover still maps the motion stream by probed index (`-map 0:{movie}`), not the still.

`media_backends`, VAAPI/Vulkan flags, timeouts, sandbox `prlimit` (`--core=0` on the current decode helper), and preferences are unchanged.

## Files

- `src/sandbox_helper/media.rs` — drop attached-pic fallback in `metadata`
- `src/sandbox_helper/media/tests.rs` — FLAC/M4A/JPEG cover fixtures; movie+cover MP4; JSON probe cases
- `docs/preview-sandbox.md` — attached pictures are not preview video

## Tests run

Private Xvfb; never `DISPLAY=:1`. Host needed GStreamer `-dev` packages and `CXX=g++` / gcc `libstdc++` `RUSTFLAGS` for `unrar_sys` on this Cloud image.

```text
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
  GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  cargo test --all-targets --all-features sandbox_helper
```

- fmt: pass
- clippy `-D warnings`: pass
- `sandbox_helper`: **19 passed**, 0 failed, 0 ignored (1576 filtered out)

Includes 773-01–773-06 coverage via:

- `audio_only_and_attached_cover_art_do_not_require_a_hardware_video_decoder`
- `movie_with_attached_cover_keeps_the_motion_video_stream`
- `metadata_and_size_parsing_fail_closed_on_bad_sources_and_protocol_values`
- existing audio-only, video, and hardware-order tests in the same filter

Omitted: full `./scripts/quality.sh test` and `./scripts/e2e.sh` (bounded helper/probe; no GPU device or `prlimit` policy change). `cargo-deny` / `typos` are not installed here; not treated as a CI pass.

## Remaining gaps

- Intel `h264_vaapi` SIGSEGV cannot be reproduced on Cloud/CI (no `/dev/dri` encode). Manual: `test-cases.md` 773-08. Do not collect cores.
- The retired encode cmdline from 0.14.0 is already gone on this branch; this round prevents the still from being treated as preview video on the current decoder.
- Inconclusive probe still fails closed (`probe` error), not “assume video”. Missing files already fail.
- Matroska attachment covers are still out of scope.
