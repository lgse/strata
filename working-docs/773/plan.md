# Plan: skip VAAPI encode for cover-art audio previews

Issue: [#773](https://github.com/lgse/strata/issues/773)
Labels: `bug`, `P2` (unchanged)
Assignee: `wmfeht`. No linked PR. One comment: Cloud Agent **partial repro** (cover-art becomes `-map 0:v:0`; Intel `h264_vaapi` SIGSEGV not reachable without `/dev/dri`).

Do not request or attach a core dump. Cores can contain secrets or private document bytes.

## Goal

Quick-previewing a FLAC/M4A (and similar) file whose only video stream is embedded cover art must not spawn `h264_vaapi`. Use the existing audio-only preview path from [#817](https://github.com/lgse/strata/pull/817): Opus WebM, no hardware encoder, no `Process crashed: ffmpeg`, no core. Real motion video keeps the current hardware → software fallback.

## Research

### Reported failure

- Strata 0.14.0 release tarball, Omarchy/Hyprland, Intel UHD 770 (`renderD129`) + NVIDIA, FFmpeg **9.0.1** / `libavcodec.so.63`.
- Selecting cover-art audio with `video_preview_backend = "automatic"` or `"vaapi"` runs the sandboxed helper; **ffmpeg SIGSEGV** in `ff_hw_base_encode_receive_packet` (`h264_vaapi`). Strata stays up. Each attempt dumps a core and a desktop “Process crashed: ffmpeg” notice.
- Command line in the core matches `media_command(VaApi)` in `src/sandbox_helper.rs`: `-hwaccel vaapi -hwaccel_device /dev/dri/renderD129 -map 0:v:0 -map 0:a:0? … -c:v h264_vaapi … pipe:1`.
- Fault is a NULL deref in FFmpeg’s hardware-encode timestamp bookkeeping (`si_addr = 0x28`), not a GPU hang (#127) and not `SIGXFSZ` (#128). Turning off hardware acceleration is a working workaround.
- Reporter’s expected product behavior: treat a crashing encoder as a backend failure, fall back to software, and stop dumping a core per attempt. The **smallest** way to get that outcome for this input is to never send a still attached picture into VAAPI.

### Why cover-art audio takes the video path

[#817](https://github.com/lgse/strata/pull/817) added `input_has_video` / an audio-only cmdline (`-map 0:a:0? -vn`, SoftwareVp8 only). The probe is:

```text
ffprobe -select_streams v -show_entries stream=index -of csv=p=0
```

FFmpeg stream specifiers: `v` matches **all** video streams, including attached pictures / cover art; `V` matches video that is **not** an attached picture, thumbnail, or cover. Cover-art FLAC/M4A therefore look like video, `render_media_preview` builds VAAPI/Vulkan/software-H264 backends, and `-map 0:v:0` encodes the still.

Verified on this Cloud VM’s FFmpeg **6.1.1** with muxed fixtures (no GPU required):

| File | `select_streams v` | `select_streams V` |
| --- | --- | --- |
| sine + PNG `attached_pic` FLAC | stream `1` (png) | empty |
| sine + PNG `attached_pic` M4A | stream `1` (png) | empty |
| H.264 movie + PNG cover MP4 | `0` and `2` | `0` only |

`-map 0:v:0` on cover-art FLAC succeeds (maps the still). `-map 0:V:0` fails with “matches no streams” — so the probe change and the map change must stay paired.

### Fallback today (do not rely on it for this bug)

`run_media_backends` already skips a backend whose ffmpeg child is non-success or empty stdout and continues to software. Media helpers **do not** use `prlimit` (no `RLIMIT_CORE=0`); that was removed so Vulkan would not die with `SIGXFSZ` (#128 / `media_previews_use_bounded_streaming_instead_of_driver_wide_resource_limits`). ffmpeg stderr is discarded. On a machine where `h264_vaapi` actually SIGSEGVs:

- systemd/kernel can still write a core and show “Process crashed: ffmpeg”;
- software fallback is **untested** on Intel VAAPI (Cloud/CI never enter that encoder).

Avoiding the encoder for attached-pic-only inputs removes the crash instead of racing it.

### Hardware vs CI

| Environment | VAAPI / `/dev/dri` | FFmpeg | What we can prove |
| --- | --- | --- | --- |
| Reporter (Intel i915 `renderD129`) | yes | 9.0.1 | Real SIGSEGV; owner/manual QA only |
| Cursor Cloud Agent / this VM | **no** (`ffmpeg -init_hw_device vaapi` fails) | 6.1.1 | Probe + software transcode only |
| Canonical `./scripts/e2e.sh` image | software raster, no Intel encode | pinned 1.98.1 + distro ffmpeg | Same; **do not** add an E2E that claims to hit `h264_vaapi` |
| GitHub Actions | no GPU | CI ffmpeg | Same as targeted Rust tests |

Cloud GUI note (issue comment): sandboxed ffmpeg on Ubuntu can fail on `libblas.so.3` because bwrap does not bind `/etc/alternatives`. That is a **distinct** Cloud/Ubuntu gap, not this bug, and not this fix.

## Approach

Smallest safe fix: **treat attached-picture-only files as audio-only**, same contract as #817.

1. In `input_has_video`, change `-select_streams v` → `-select_streams V`. Successful empty stdout stays audio-only. Inconclusive probe still returns true (keep the existing video fallback comment).
2. In `media_command`’s video branch, change `-map 0:v:0` → `-map 0:V:0` so a file that has both a movie and a cover encodes the movie, not the still (the same crash trigger if stream 0 is the attached pic).
3. Leave `media_backends`, VAAPI/Vulkan flags, timeouts, `run_media_backends`, sandbox `prlimit` policy, and preferences unchanged.
4. Docs: one sentence in `docs/preview-sandbox.md` that cover art / attached pictures do not count as preview video (they follow the audio-only path).

No new preference, no crash blacklist, no `RLIMIT_CORE`, no FFmpeg upgrade, no sandbox bind for `/etc/alternatives`.

## Touch points

- `src/sandbox_helper.rs` — `input_has_video`, video `-map` in `media_command`
- `src/sandbox_helper/tests.rs` — cmdline assertions (`0:V:0`), probe specifier if asserted
- `src/sandbox_helper/tests/media.rs` — mux cover-art FLAC/M4A (+ JPEG cover if cheap); `input_has_video` false; `render_media_preview` for Automatic/VaApi/Software → Opus WebM; MP4 movie+cover still has video and maps `0:V:0`
- `docs/preview-sandbox.md` — attached-pic sentence

Tests stay in the adjacent `src/sandbox_helper/tests*.rs` modules (not inline in production). No new E2E file: `tests/e2e/scenarios/test_quick_preview.py` is text-only and cannot exercise Intel VAAPI.

## Constraints

- GTK 4 baseline unchanged; no UI widgets, icons, or theme tokens.
- Private Xvfb for any GTK tests; never `DISPLAY=:1`. This change is helper/cmdline, so targeted `cargo test` filters on `sandbox_helper` are enough.
- Do not restore media `prlimit --fsize/--as` (reopens #128).
- Do not ask anyone for a core dump.

## Risks

- If probe is **inconclusive** on a cover-art-only file, we still take the video path; `-map 0:V:0` then fails every backend and the preview is unavailable. Cover-art FLAC/M4A probe **successfully** with empty `V` on FFmpeg 6.1.1; missing files already fail. Do not weaken the inconclusive→video invariant.
- Genuine one-frame **motion** video (not `attached_pic`) can still hit the upstream encoder NULL deref. Out of scope; do not special-case `nb_frames=1`.
- Matroska often stores covers as attachments, not `attached_pic` video; FLAC/M4A/MP4 are the reported and verified muxers. Do not chase MKV attachment mapping here.
- Software encode of a still attached pic **works** today. After this fix we **stop** showing that still as a 30s video and play audio-only instead. That matches #817 and is the desired preview for an audio file.

## Non-goals

- Patching FFmpeg / shipping a private libavcodec.
- `RLIMIT_CORE=0`, crash-count backend pinning, or skipping VAAPI after N SIGSEGVs.
- Hardware fallback proof on CI/Cloud (impossible without Intel VAAPI + FFmpeg 9).
- Ubuntu bwrap `/etc/alternatives` / `libblas.so.3`.
- Changing `video_preview_backend` defaults, Polaris software default, or Settings copy.
- Thumbnail (`ffmpegthumbnailer`) path.

## Recommended verification (code stage)

Bounded change: probe specifier + map specifier + fixtures. Not full `./scripts/quality.sh` / full `./scripts/e2e.sh` unless the edit grows into sandbox `prlimit` or GPU device policy.

- Targeted: `./scripts/test-headless.py sandbox_helper` (or Cloud equivalent `xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 cargo test --all-targets --all-features sandbox_helper`). Confirm a **nonzero** test count including the new cover-art cases.
- Pre-push (later): `cargo fmt --all --check` and `cargo clippy --all-targets --all-features -- -D warnings`.
- Manual (Intel VAAPI host only): cases in `test-cases.md` 773-08. Do not collect cores.

## Status (from working-docs/773/status.md)

- stage: `plan complete; ready for code`
- staging_pr: none
- recommended_branch: `fix/773-cover-art-vaapi-sigsegv`
- head_sha: n/a
- agent_id: `bc-f270fc50-b7c9-51b5-8feb-079578e82bc0`

Coordinator next: draft staging PR on `fix/773-cover-art-vaapi-sigsegv` from latest `lgse/strata` `main`, then `strata-code`. Do not implement in this step. P2 unchanged.
