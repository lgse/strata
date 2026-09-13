# Round 1 review: #773 skip cover art as preview video

- **verdict:** `approve-with-comments`
- **staging_pr:** https://github.com/lgse/strata/pull/934 (draft, left draft)
- **head_sha reviewed:** `fe16981ba1de0ba2f62af96235fa61640618fb27`
- **n:** 773 (`working-docs/773/`)
- **agent_id:** `bc-7e858fdd-59b9-51da-8b89-159837d6c668`

## Blockers

None.

## Nits / suggestions

- `Input.cover` is now unreachable. Video is chosen with `!is_cover`, so `cover: video.is_some_and(is_cover)` is always false, `|| input_info.cover` never fires, and the one-frame still-decode branch in `command()` cannot run for `attached_pic`. Harmless leftover, not a behavior bug.
- Cover-art tests assert `!info.cover` on files whose only video is cover art. True under the new meaning (“selected preview video is a cover”), but easy to misread. `video.is_none()` plus width 0 is the real contract.
- `working-docs/773/test-cases.md` still describes `input_has_video`, Opus WebM, and `-map 0:V:0`. The code adapted those cases to `probe` / raw PCM / `-map 0:{index}` correctly; the case file was not retargeted.
- Cover-art cmdline assertions inspect the audio command (`h264_vaapi` was never on that argv). `stream()` width 0 is the useful check; a `backends(...)` assertion on a cover-only `Input` would make the software-only policy explicit.

## Notes

The plan targeted 0.14.0’s encode helper (`input_has_video`, `-select_streams v`, `-map 0:v:0`, `-c:v h264_vaapi`). That encoder is already gone on current `main` (`preview-media` writes `rawvideo` / PCM; hardware is `-hwaccel vaapi|vulkan` decode only). The reported `ff_hw_base_encode_receive_packet` SIGSEGV cannot run on this tree even without this diff.

On current `main` before this change, a successful `attached_pic` probe already set `cover` and forced `Backend::Software`, so cover-art FLAC/M4A also would not enter VAAPI decode. The remaining bug was product selection: metadata **fell back** to the still as preview video (software-decoded frozen frames). Dropping that fallback is the correct analogue of the plan’s `v` → `V` probe/map pair:

- Cover-only files: `video: None` → audio-only decoder, no video child, no hwaccel.
- Movie + cover: first non-cover stream by index (`-map 0:0` on the fixture), not the still.

That matches the plan’s intended contract (#817-style audio-only for attached-pic-only files; motion video kept). It does **not** reintroduce or patch `h264_vaapi`. Closing #773 on this branch is still right: the encoder path is already retired, and cover art no longer becomes a video track.

Residual risk (plan already allowed): Intel `h264_vaapi` SIGSEGV is owner/manual (773-08); Cloud/CI have no `/dev/dri` encode. Genuine one-frame motion video and Matroska attachment covers stay out of scope. Inconclusive `ffprobe` fails closed (preview error), which is pre-existing on the decode helper, not a weakening of the old inconclusive→video invariant.

## CI / process

- GitHub format/lint/test and E2E skipped (draft). Metadata policy failed: agent attribution. William will resign/sign; history was not rewritten.
- `mergeable_state: blocked`. Did not rebase, amend, undraft, merge, or run cleanup.
- Did not re-run full local CI. Code stage recorded `sandbox_helper` **19 passed** plus fmt/clippy; that scoped set covers the change.

Did not implement a fix.
