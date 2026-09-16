# Preview sandbox

Strata treats files shown while browsing as untrusted. Original-file native
parsing and decoding run inside bubblewrap, never in the application.

## Providers

- GDK Pixbuf/camera RAW, Poppler PDF, ImageMagick, and dcraw fallbacks normalize
  images to dimension- and size-bounded PNG images.
- `ffmpegthumbnailer` produces bounded media thumbnails. One helper serves at most
  64 queued unique requests; duplicate requests share work, obsolete targets
  cancel it, and failures are cached for 30 seconds.
- Media previews use the incremental decoded-frame transport described below.
- Plain text stays in-process, invokes no native format parser, and is capped at
  1 MiB.
- Local XLS, XLSX, and ODS previews run Calamine inside the resource-limited
  helper, accepting at most 20 MiB of input. Only validated, bounded JSON cell
  values cross back into the application; macros and formulas are not executed.
- Local [Markdown, bounded HTML, CSV, and TSV previews](document-previews.md) are parsed
  in-process by pure-Rust parsers that receive only the bounded source string and
  cannot initiate filesystem access, network access, JavaScript execution, or
  subresource loading.

## Bundled interface icons

Strata's bundled Lucide SVGs are trusted application resources, not browser files.
They render directly to bounded in-memory pixels with `resvg`, avoiding synchronous
GdkPixbuf/Glycin loader startup on the GTK thread during row binding and live theme
changes. External and embedded image references are disabled; icon inputs and
output dimensions are bounded. SVG text, system-font lookup, and raster-image
features of this renderer are disabled. Emoji icons retain Pango/Cairo rendering
but pass raw pixels to GTK instead of encoding and decoding an intermediate PNG.

This renderer is not used for user SVGs, phone photos, or thumbnails of originals;
those keep their existing sandbox boundary. No toolkit libraries or private media
runtime patches are updated by this change.

## Remote still-image previews

Still images on GIO/GVfs locations (including phone camera, AFC, and MTP storage)
can use the preview pane and Space quick preview. The application streams the
selected original to a randomly named mode-0600 temporary file before invoking
the same image sandbox as local files. Only a short alphanumeric extension is
preserved; remote names never choose a local path. The displayed entry and its
actions retain the original URI.

Still-image transfers have a 64-MiB byte limit and a 30-second total deadline. Known
oversized files are rejected before opening; the stream limit also applies when
size metadata is missing or inaccurate. At most four transfers/staged originals
can be active per process, including ones waiting for cancellation or decoding.
A busy request fails with an explanatory message rather than starting another
unbounded transfer. Downloads run off the GTK thread.

Changing selection or closing the preview cancels GIO. A backend that is slow to
acknowledge cancellation retains its slot until its worker exits. Partial files
are removed on transfer failure; successful inputs remain owned by the decoder
worker and are removed when it exits, even if the UI has already cancelled the
request. Normal cleanup does not guarantee removal after a process crash or
forced termination. Remote previews do not populate the persistent thumbnail
cache or the in-memory rendered-preview cache, and originals are never modified.

MOV and MP4 previews use the same staging boundary, with a 256-MiB byte limit
and 60-second deadline, and share the four-input admission limit with images.
Playback starts only after download completes. The sandboxed media descriptor
retains the temporary file and its admission permit across player clones,
seeks, resizing, and paused-worker restart. The final player/worker owner removes
the input, including cancellation and decoder-error paths; no compressed input
is handed to an in-process media parser.

Remote PDFs, animated GIFs, audio, and other video formats remain unsupported:
copy them locally first. In particular, remote PDF rendering/printing needs a shared document
snapshot before it can safely request multiple pages without repeated downloads.
Supported still-image formats continue to depend on installed sandbox decoder tools;
HEIC/HEIF can use ImageMagick with libheif inside the image sandbox.

### Camera file-list thumbnails

Camera/PTP rows use the backend's `preview::icon` / `GLoadableIcon` interface.
GVfs's gphoto2 implementation requests `GP_FILE_TYPE_PREVIEW`, not the original.
Missing or failed previews leave the ordinary file icon; Strata does not fall
back to downloading full photos for thumbnails.

Retrieval is asynchronous, limited to 1 MiB and 15 seconds. These jobs share the
existing four-worker, 64-waiting-job thumbnail queue and row-binding cancellation.
The compressed preview is written to a random mode-0600 temporary file, decoded
by the existing image-thumbnail sandbox, and removed after the decoder exits.
Only the normalized PNG reaches GTK. Generated camera thumbnails use the bounded
in-memory cache, never the persistent thumbnail cache. AFC and MTP file-list
thumbnails are not enabled by this path.

Complete Photos scans yield between batches to give visible thumbnails a bounded
turn on the camera connection. If the viewport is at the top, incoming batches
leave it there instead of following the old first photo down the sorted list.
After the user scrolls, normal viewport anchoring applies; background selection updates do
not reveal the selected photo. The initial folder listing still depends on GVfs
finishing that folder's enumeration before it publishes entries.

## Media metadata

File Properties shows available source-media details: image
resolution; audio/video duration and overall bitrate; video codec and frame rate;
and audio codec, sample rate, and channel count. These describe the original file,
not the preview's scaled frames or resampled audio. Attached album artwork is not
reported as a video track, and still images do not show synthetic video timing.
Missing individual fields are omitted; an unsuccessful inspection shows
`Media: Unavailable` without blocking the other file information.

Properties uses an asynchronous inspector. Only regular files with a
local source are inspected; remote files are not downloaded for metadata. The
inspector runs `ffprobe` inside the existing software-only bubblewrap sandbox,
with a four-second probe timeout and a 64 KiB JSON limit. Image information can
fall back to GDK Pixbuf inside that same sandbox. The enclosing helper retains
the existing memory, CPU, and wall-time limits and receives no GPU access. Media
sandboxes expose only the optional BLAS/LAPACK runtime alternatives for supported
x86-64 and ARM64 Debian-family installations, not all of `/etc/alternatives`. Only
validated numeric fields and bounded codec identifiers reach the UI, not arbitrary
embedded tags. Closing Properties cancels its work and prevents stale results
from appearing. The preview pane retains only its normal size, modified date,
and type information; it does not run this metadata inspector.

## Incremental media playback

```text
One original local file (read-only)
  -> bubblewrap: ffprobe + FFmpeg video/audio decoding and scaling
  -> bounded, validated RGBA frames and PCM blocks
  -> Strata GtkMediaStream: GTK MemoryTexture presentation
                         + GStreamer appsrc raw-audio output
```

There is no compressed-video encoding, normalized MP4/WebM temporary file,
`GtkMediaFile`, `GstPlay`, compressed-media pipeline, or original-file URI in the
player. FFmpeg's `rawvideo` output packs RGBA pixels; `pcm_s16le` packs decoded
samples. GStreamer receives only application-constructed `audio/x-raw` caps and
validated samples through `appsrc ! audioconvert ! audioresample ! volume !
autoaudiosink`. No decoder-provided path, pipeline, caps string, or compressed
packet crosses into an unsandboxed media parser.

A media worker uses separate FFmpeg video and audio processes when both tracks
exist. Each decodes only its selected track. This avoids cross-output pipe
deadlocks with sparse/VFR video or attached cover art. Both processes remain in
one sandbox, with access to the same single input. Cover art is decoded once in
software and retained alongside audio; audio-only inputs need no video decoder.
Video timing is normalized **inside the sandbox** to 30 fps, including holding
VFR/GIF frames. Audio is 48 kHz, stereo, interleaved signed 16-bit little-endian
PCM. Resampling preserves gaps/offsets relative to the common source timeline.

Previews play **the entire source**, without a 30-second playback cap. Seeking
restarts the sandbox at the requested source position, rounded down to the 30-fps
grid. Video retains decoder preroll so a seek into a VFR gap can show the frame
covering that point. Short GIFs still batch loops into a 30-second generation,
with seeks mapped to their animation phase to avoid restarting a process every
cycle; this does not truncate the animation. Longer GIFs play their complete
animation before looping.

Files without a reported duration play until decoded EOF. Their timeline remains
unknown and seeking is disabled until the actual end is established; pause/idle
resume still retains the playback position. The wire's representable terminal
tick (about 4.5 years at 30 fps) is an arithmetic ceiling and unknown-duration
sentinel, not a practical preview-length policy.

The decode rectangle follows the pane's logical size times display scale, capped
at 1280 pixels on either axis. Frames preserve display aspect ratio, including
sample aspect ratio/right-angle rotation, without unnecessarily enlarging small
sources. Resizes settle for 250 ms before restarting at the current playback
position; the previous texture remains visible. Changes that would not materially
change the fitted frame size do not restart decoding. A paused resize/seek stays
paused. Mute/volume preferences initialize and update every player's raw-audio
output live; backend preference changes apply on the next preview request.

## Wire validation and budgets

The versioned `STRRAW01` little-endian protocol has a 40-byte header: magic,
width, height, exact RGBA stride, audio-present flag, duration in microseconds,
starting tick, and a zero reserved field. Each 24-byte record contains its type,
tick, timestamp, and video/audio lengths. Frame records are followed by exactly
those payloads; an explicit end record has no payload. EOF alone is not success.

The parent independently checks:

- version/reserved fields, flags, requested dimensions, stride, and arithmetic;
- positive duration within the `u32` terminal-tick range and the requested start
  tick; the maximum representable duration denotes an unknown source duration;
- strictly consecutive ticks below the declared duration, exact
  `floor(tick * 1,000,000 / 30)` timestamps, and an end tick that cannot overflow;
- video length exactly `width * height * 4`, at most 6,553,600 bytes;
- PCM length exactly 6,400 bytes per tick (1,600 stereo samples), with the last
  block truncated to the advertised content duration before audio output;
- end timestamp/lengths, no trailing output, and successful helper exit.

Lengths are checked **before allocation**. Helper buffers cannot mutate textures:
pipe reads create owned buffers, and the texture owns its `glib::Bytes` until its
last reference is released. There is no shared writable memory.

The worker-to-GTK queue holds three records, the presentation queue three frames,
and a worker may hold one pending frame. Including the displayed CPU texture,
that is at most eight directly application-held frames (50 MiB at the maximum
square size), plus small audio buffers and bounded kernel pipes. FFmpeg's input
and output packet queues are limited to two packets each. GStreamer's appsrc
queue is capped at 38,400 bytes / 200 ms; no unbounded queue element is inserted.
GTK/driver rendering caches and codec working memory are additional, not part of
that application-buffer claim. Total decoded bytes scale with playback duration,
but queued memory does not: no complete clip is accumulated. Audio timestamp
conversion uses wide intermediates and rejects values outside GStreamer's clock
range rather than overflowing for long playback. Seeking/looping begins a new
memory-bounded generation.

There is **no whole-clip media cache**. Closing or revisiting a file requires a new
decode. The existing byte/entry-bounded image/PDF preview cache is unchanged.
Image renders preserve small source dimensions so the UI can enforce its 2×
upscaling limit. Image previews do not use normalized shared-thumbnail
placeholders, which can already be enlarged and lack reliable native dimensions;
they request the bounded full render immediately. PDFs likewise wait for a bounded
page render with verified page count. File-list thumbnail storage/reuse is unchanged.

## Scheduling and deadlines

At most **four media sessions per application process** may own workers, across
browser windows and choosers sharing that process. Each owns one parent reader
thread and at most two FFmpeg decoding processes. A fifth request reports busy
instead of interrupting another player. The slot is retained through buffered
playback, so a fifth player cannot steal it between GIF loops. A short 250-ms
acquisition grace allows
an obsolete worker to finish cancellation. Separate application/portal processes
have separate limits, not a machine-global scheduler.

| State | Bound / behavior |
| --- | --- |
| Startup or seek | 22 seconds from request to first frame; includes a 4-second probe, hardware attempts of at most 4 seconds each / 8 seconds combined, and up to 8 seconds for software. |
| Active decoding | 8 seconds for a complete next record, not a deadline restarted by each byte. A stalled audio playback clock also fails after 8 seconds. |
| Backpressure | A full queue stops consumption and propagates pressure through bounded pipes; it does not accumulate a whole clip. Waiting for the consumer is not charged as decoder progress time. |
| Paused | Keep position, frame and bounded queues for 30 seconds, then cancel the worker, drop PCM output/queues, and stop the polling timer. The displayed frame and position remain. Resume or a paused seek starts a new bounded decode. |
| Close / selection change / destruction | Cancel promptly; pipe/queue waits check cancellation at 10–20-ms intervals. Kill/reap the renderer and its sandbox descendants. No join of a blocked pipe reader on the GTK thread. |
| End / malformed output / failure | Release the worker; malformed/truncated output, unavailable decoding and unsuccessful exit fail closed. No unsandboxed fallback. End-of-audio allows only sample-grid rounding (at most 22 µs), not a stalled clock. |

The old batch-conversion wall timeout does not govern a paused real-time player.
The limits do not promise instant startup, zero-latency seeking, or a total RAM
plateau for every toolkit/driver.

## Isolation and hardware policy

Bubblewrap retains the existing namespace/mount policy:

- new user, mount, PID, IPC, UTS, cgroup, and network namespaces;
- read-only `/usr`, required runtime libraries and font/ImageMagick configuration,
  the Strata executable, and exactly one canonicalized regular input file;
- writable private mode-0700 output directories for image providers and a
  size-limited (512 MiB) private `/tmp`; media uses pipes, not output mounts;
- an empty environment, nonexistent home, and no desktop, session-bus, or
  audio-server sockets. Only the application handles presentation/audio output.

Image/PDF parsing has a 512-MiB input cap, 2-GiB address-space cap, 512-MiB file
cap, 32-MiB parent output cap, 12-second wall limit and 10-second CPU limit.
Media has no input-file size cap, playback-duration policy cap, or cumulative CPU
limit; per-record sizes, queue limits, and progress deadlines bound active work. Each software FFmpeg process has a 2-GiB
address-space limit; all decoding processes disable core dumps and cap files and
individual media allocations at 512 MiB and source frames at 50 million pixels.
Accelerated decoders retain the existing exemption from the address-space limit
because drivers reserve large virtual ranges; this is not a hardware RSS cap.

Automatic tries VA-API, then Vulkan, then software. Explicit VA-API/Vulkan falls
directly back to software when its hardware attempts fail before output begins.
A failure after playback starts fails closed rather than silently splicing a new
backend into the stream. VA-API receives only safe `/dev/dri/renderD<digits>`
nodes; Vulkan/Automatic may also receive `/dev/nvidia<digits>` and `/dev/nvidiactl`.
Accelerated helpers receive read-only `/sys` for discovery. Software/image/PDF/
thumbnail helpers receive no GPU devices or `/sys` mount.

The conservative AMD Polaris default remains unchanged: an unset acceleration
preference resolves to software for PCI IDs `0x67c0–0x67df`, `0x67e0–0x67ff`, or
`0x6980–0x699f`. Explicit opt-in remains available. Removing the old encoder path
is not evidence for changing device defaults or declaring driver issues fixed.
GPU access still expands the helper's attack surface into driver code.

## Compatibility scope

GStreamer app/base development libraries are now direct build dependencies;
installed systems need their runtime libraries and raw-audio/output plugins.
In particular, the core and app/base libraries are now required to **launch**
Strata, even when no preview is open. Minimal installations that self-update only
the executable must install these packages before updating. Installer/AUR metadata
retains the legacy codec-plugin recommendation for currently published binaries;
the new player does not use those decoders.
Toolkit versions and the opt-in patches in
[`packaging/media-runtime`](../packaging/media-runtime/README.md) are unchanged.
The new player does not use the two patched `GtkGstSink`/`GstPlay` paths, but this
change neither applies nor retires that patch kit or claims to fix all RAM growth.

The Ubuntu runtime-library alias problem tracked in
[#806](https://github.com/lgse/strata/issues/806) was initially deferred. The sandbox
now includes optional read-only binds of the BLAS/LAPACK alternatives used by
media helpers on x86-64 and ARM64 Debian-family installations. This resolves their
runtime links without exposing the whole `/etc/alternatives` directory or the
rest of `/etc`. Canonical pinned-container HEIC, MOV,
and MP4 preview tests exercise actual sandbox startup and decoding; installed
systems still depend on their available codec libraries.
