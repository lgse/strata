# Round 1 QA: #773 skip cover art as preview video

- **verdict:** `pass-with-nits`
- **staging_pr:** https://github.com/lgse/strata/pull/934 (draft, left draft)
- **head_sha tested:** `3add247fb756ba8301628e76234bcc599c4dc91d` (PR tip at QA start)
- **product code:** `fe16981ba1de0ba2f62af96235fa61640618fb27`
- **n:** 773 (`working-docs/773/`)
- **agent_id:** `bc-a02ab8a5-c849-5485-aeaa-9cc5b551789c`

Functional/exploratory of the draft staging PR. Not a second code review.

## Product non-nits

None. Every planned case this environment can run passed.

## Nits / process

Same class as round 1 review (not product breaks):

- `working-docs/773/test-cases.md` still names `input_has_video`, Opus WebM, and `-map 0:V:0`. On this tree the helper probes JSON, streams raw PCM / `rawvideo`, and maps `-map 0:{index}`. Cases were exercised against that contract.
- Cover-art cmdline assertions inspect the audio argv (`h264_vaapi` was never on that argv). `stream()` width 0 under Automatic/VaApi/Vulkan is the useful check; those policies succeeded here without `/dev/dri`.
- `Input.cover` remains unreachable leftover. Harmless.

## Coverage

| Case | Result |
| --- | --- |
| 773-01 cover-art FLAC is not preview video | pass (`audio_only_and_attached_cover_art…`; independent ffprobe `v` = png attached_pic, `V` empty; `probe` `video.is_none()`, width 0) |
| 773-02 cover-art M4A and JPEG cover | pass (same test: `cover.m4a`, `cover-jpeg.flac`; independent ffprobe matches) |
| 773-03 Automatic/VaApi/Vulkan/Software cover-art path | pass (all four policies: audio header, width/height 0, `-map 0:0 -vn`, no `-hwaccel` / `0:v`. Not Opus WebM — current helper is raw PCM) |
| 773-04 movie+cover keeps motion video | pass (`movie_with_attached_cover…`; ffprobe `v` = 0+2, `V` = 0; map `-map 0:0` not `0:2` / `0:v:0`; decoded 64×48 with frames) |
| 773-05 audio-only without cover; real video without cover | pass (tone.flac / tone.ogg audio-only; `raw_software_video…`, `clip.mkv`, GIF/VFR tests in the same filter) |
| 773-06 hardware order / software last | pass (`hardware_order_and_commands_decode_only…`; decode-only argv, software last). Does **not** prove a live `h264_vaapi` SIGSEGV fallback |
| 773-07 targeted `sandbox_helper` | pass — **19 passed**, 0 failed, 0 ignored, 1576 filtered out, 3.98s |
| 773-08 Intel VAAPI SIGSEGV | **skipped** — no `/dev/dri`; `ffmpeg -init_hw_device vaapi=va` fails (`Device creation failed: -542398533`). FFmpeg here is 6.1.1, not reporter 9.0.1. Not a product fail |

Adjacent (same private Xvfb, already-built test bin): sandbox GPU/sysfs policy, bounded streaming vs `prlimit`, decoder failure, preview MIME classification, quick-preview offer — **8 passed**. Thumbnail path still `ffmpegthumbnailer` (unchanged; out of scope). Image/PDF helper tests in the `sandbox_helper` filter passed. Preferences / `video_preview_backend` not in the product diff. Two-window UI N/A (helper-level). Missing-path probe still fails closed.

## Gaps

- Intel encode SIGSEGV (773-08) cannot be confirmed here. Owner/manual only. Do not collect cores.
- Canonical `./scripts/e2e.sh` not run (bounded helper change; `test_quick_preview.py` is text-only).
- Full `cargo test --all-targets --all-features` and pre-push fmt/clippy not rerun (QA scoped to planned helper cases).
- Matroska attachment covers and genuine one-frame motion video remain out of scope.

## Commands / results

Private Xvfb (`xvfb-run -a`); never `DISPLAY=:1`. Outer `DISPLAY` unset for the test process.

```bash
xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
  GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  cargo test --all-targets --all-features sandbox_helper -- --nocapture
```

- `sandbox_helper`: **19 passed**, 0 failed
- adjacent filter: **8 passed**, 0 failed
- independent mux+ffprobe of cover FLAC/M4A/JPEG and movie+cover MP4 matches the plan table (`v` includes attached_pic; `V` does not)

Did not implement, undraft, merge, squash, rebase, amend, force-push, file GitHub issues, or run strata-cleanup. William will resign/sign himself.
