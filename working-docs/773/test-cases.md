# Test cases: #773 cover-art audio vs VAAPI

Narrow set. Automated cases run on Cloud/CI **without** `/dev/dri`. The Intel SIGSEGV case is owner/manual only. Never request a core dump.

## 773-01 — Probe: cover-art FLAC is not preview video

**Purpose.** Attached pictures must not trip `input_has_video`.

**Setup.** Mux 1s sine + PNG with `-disposition:v attached_pic` into `cover.flac` (same recipe as the issue comment). FFmpeg must be on PATH.

**Steps.**

1. `ffprobe -select_streams v` lists a video stream; `-select_streams V` lists none.
2. Call `input_has_video(&cover.flac)`.

**Expected.** `input_has_video` is false. Existing sine-only OGG fixture stays false. Missing path stays true (inconclusive probe).

## 773-02 — Probe: cover-art M4A and JPEG cover

**Purpose.** Reporter formats, not only PNG-in-FLAC.

**Setup.** `cover.m4a` (AAC + PNG attached pic) and `cover-jpeg.flac` (audio + MJPEG attached pic).

**Steps.** Same as 773-01 for each file.

**Expected.** `input_has_video` is false for both.

## 773-03 — Helper: Automatic/VaApi/Software normalize cover-art to Opus WebM

**Purpose.** Even with a VAAPI policy, cover-art audio must take the #817 audio-only cmdline (no `-hwaccel`, no `0:v:0`).

**Setup.** Fixtures from 773-01/02. Call `render_media_preview` with `Automatic`, `VaApi`, `Vulkan`, and `Software` at `MediaPreviewSize::new(520, 800)`.

**Steps.**

1. Assert the bytes start with WebM EBML `\x1a\x45\xdf\xa3`.
2. `ffprobe` the result: one audio stream, `opus`, no video stream.
3. Assert `media_command(..., has_video=false)` contains `-map 0:a:0? -vn` and does not contain `-hwaccel`, `-c:v`, or `0:v:0`.

**Expected.** All four policies succeed without needing a DRM node. No ffmpeg crash in the test process.

## 773-04 — Cmdline: video maps `0:V:0`

**Purpose.** Motion video must skip attached pictures at map time.

**Setup.** Existing `media_commands_select_the_backend_and_preserve_limits` plus a movie+cover MP4 (H.264 + AAC + PNG `attached_pic`).

**Steps.**

1. Video `media_command` strings contain `-map 0:V:0 -map 0:a:0?` and do **not** contain `-map 0:v:0`.
2. `input_has_video(&movie.mp4)` is true; `select_streams V` is the movie stream only.
3. Software `render_media_preview` of that MP4 still has a video stream (not audio-only).

**Expected.** Cover on a real movie does not demote the file to audio-only and does not map the still as `0:v:0`.

## 773-05 — Regression: audio-only without cover, and real video without cover

**Purpose.** #817 and the video pipeline stay intact.

**Setup.** Existing `audio_only_inputs_normalize_to_opus_for_every_backend_policy` and `video_and_unreadable_inputs_keep_the_video_pipeline`.

**Steps.** Re-run those tests; do not rewrite their fixtures.

**Expected.** Unchanged: tone.ogg → Opus; clip.mkv and missing.mkv keep the video pipeline.

## 773-06 — Regression: hardware failure still falls through to software

**Purpose.** Do not break `run_media_backends` while avoiding VAAPI for covers.

**Setup.** Existing `hardware_failures_fall_back_and_first_success_stops` and `forced_backend_failure_goes_directly_to_software`.

**Steps.** Re-run; no GPU.

**Expected.** Unchanged skip/success order. This does **not** prove a live `h264_vaapi` SIGSEGV falls back.

## 773-07 — Targeted automated command (code stage)

**Purpose.** Nonzero, scoped Rust coverage. Cloud: private Xvfb, never `DISPLAY=:1`.

**Setup.** `/workspace` on the staging branch. FFmpeg/ffprobe installed (host image already has 6.1.1).

**Steps.**

```bash
./scripts/test-headless.py sandbox_helper
```

Cloud equivalent:

```bash
xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
  GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  cargo test --all-targets --all-features sandbox_helper
```

Confirm collection includes 773-01–773-06 names (or the existing tests they extend). Count must be nonzero.

**Expected.** All selected tests pass. Do not run full `e2e.sh` for this bounded helper change. Do not treat a local `cargo-deny`/`typos` skip as a CI pass.

## 773-08 — Manual: Intel VAAPI host (not CI)

**Purpose.** Confirm the reported crash is gone on hardware CI cannot see.

**Setup.** Machine like the issue: Intel render node, FFmpeg 9 if available, Strata with hardware video previews **on**, `automatic` (then `vaapi`). Folder with cover-art `.flac` / `.m4a` and a normal `.mp4`. Single-click / Space quick preview. **Do not** upload cores; if a crash still happens, note `coredumpctl list` timestamps only.

**Steps.**

1. Preview cover-art audio: player appears (audio), no “Process crashed: ffmpeg”, no new ffmpeg SIGSEGV.
2. Preview a normal video: hardware path may still run; playback works or software fallback works without a cover-art-style still-frame crash.
3. Toggle hardware acceleration **off**: cover-art audio still previews (workaround remains valid).

**Expected.** Step 1 is the acceptance test for #773. CI green without this host is **not** proof the SIGSEGV is fixed.
