# Preview sandbox

Strata treats files shown while browsing as untrusted. Original-file native
parsing and decoding run inside bubblewrap, never in the application.

## Providers

- GDK Pixbuf/camera RAW, Poppler PDF, ImageMagick, and dcraw fallbacks normalize
  images to dimension- and size-bounded PNG images.
- `ffmpegthumbnailer` produces bounded media thumbnails. The thumbnail coordinator
  queues at most 64 unique requests in request order; duplicates share work,
  obsolete targets cancel it, and failures are cached for 30 seconds. Each decode
  uses a fresh helper and sandbox, not a reusable native parser.
- Media previews use the incremental decoded-frame transport described below.
- Plain text stays in-process, invokes no native format parser, and is capped at
  1 MiB.

## Incremental media playback

```text
One original local file (read-only)
  -> bubblewrap: ffprobe + FFmpeg video/audio decoding and scaling
  -> bounded, validated RGBA frames and PCM blocks
  -> strata: validated GTK MemoryTexture presentation
           + bounded PCM/control pipe
             -> separate bubblewrap strata-media-helper: GStreamer raw-audio output
```

There is no compressed-video encoding, normalized MP4/WebM temporary file,
`GtkMediaFile`, `GstPlay`, compressed-media pipeline, or original-file URI in the
player. FFmpeg's `rawvideo` output packs RGBA pixels; `pcm_s16le` packs decoded
samples. GStreamer receives only application-constructed `audio/x-raw` caps and
validated samples through `appsrc ! audioconvert ! audioresample ! volume !
pulsesink` in the **separate PCM worker**. No decoder-provided path, pipeline,
caps string, or compressed packet crosses into an unsandboxed media parser.
The UI crate has no GStreamer or Poppler dependency; shared transport and media
selection types live in `crates/media-protocol`, native parsing/output in
`crates/media-helper`.

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

Both roles first exchange a bounded protocol-1 identity containing the operation
role, nonzero job/generation, exact release tag and source commit. Parser and PCM
identities cannot be interchanged. PCM control/data messages also validate job,
strict sequence, reserved bytes, fixed payload bounds, contiguous sample counts,
sample-derived timestamps and terminal EOS. Only fixed commands and 48-kHz stereo
PCM are accepted; there is no IPC pipeline string, URI or command execution.

After the parser handshake, the versioned `STRRAW01` little-endian protocol has a 40-byte header: magic,
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
The UI-to-audio queue holds at most three messages, each PCM payload at most 6,400
bytes; controls use atomic latest-value state so cancellation is not queued behind
audio. Pipe reads and writes have cancellation and whole-message deadlines.
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
page render with verified page count. File-list thumbnail storage/reuse is unchanged. See [preview sizing](evidence/885/README.md).

## Scheduling and deadlines

At most **four media sessions per application process** may own workers, across
browser windows and choosers sharing that process. Each owns one parent reader
thread, at most two FFmpeg decoding processes, and (when audio exists) one PCM
coordinator thread and one separate PCM worker. Both roles share **one** session
lease, retained until audio teardown finishes. A fifth request reports busy
instead of interrupting another player. The slot is retained through buffered
playback, so a fifth player cannot steal it between GIF loops. A short 250-ms
acquisition grace allows
an obsolete worker to finish cancellation. Separate application/portal processes
have separate limits, not a machine-global scheduler.

Raster operations share a separate four-worker fail-fast cap. At most twelve
helper processes can therefore belong to an application (four parsers, four PCM
workers, four raster workers), plus up to eight media FFmpeg children and the
bounded raster tools/bubblewrap supervision. Thumbnail order is FIFO within its
coordinator; independent raster callers race for the cap rather than reserving
an unbounded waiting queue. There is no cross-application fairness guarantee.
Cancellation is checked before admission and takes priority over transport.
This is not a new parser pool or the coordinator-reuse/RSS work in #841/#516.

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
plateau for every toolkit/driver. See [measurements and manual checks](evidence/824/README.md).

## Isolation and hardware policy

Bubblewrap retains the existing namespace/mount policy:

- new user, mount, PID, IPC, UTS, cgroup, and network namespaces;
- read-only `/usr`, required runtime libraries and font/ImageMagick configuration,
  the pinned helper executable, and exactly one canonicalized regular input file;
- writable private mode-0700 output directories for image providers and a
  size-limited (512 MiB) private `/tmp`; media uses pipes, not output mounts;
- an empty environment, nonexistent home, and no desktop, session-bus, or
  audio-server sockets. The application alone handles texture presentation.

PCM workers use a different sandbox: no source file, GPU, desktop or session bus,
only the executable/runtime libraries and a bind of the real, user-owned
`pulse/native` Unix socket beneath a private user-owned `XDG_RUNTIME_DIR` (and an
optional local PulseAudio cookie). This supports PulseAudio and PipeWire through
`pipewire-pulse`, not a native PipeWire/ALSA device fallback. The sink is fixed;
environment-selected servers, sinks and plugin paths are not inherited. A fixed,
read-only client policy disables daemon autospawning and SHM/memfd transport.
PCM stays on the socket: PulseAudio's default 64-MiB shared-pool allocation would
otherwise exceed this role's 4-MiB file-size limit before its handshake. This
does not edit the user's audio configuration. Unsafe runtime paths fail closed. Temporary/install path ancestry is validated before
use, job storage is mode 0700, and only root or the effective user may own trusted
ancestors (writable ancestors require sticky-directory protection).

Both roles clear the environment, use absolute system tool paths, deny privilege
gain, disable core dumps, retain private PID/network namespaces and die with the
parent. The PCM sandbox additionally has a 2-GiB address-space limit. A helper
is pinned by open file descriptor; package replacement cannot redirect a running
UI to a different helper inode. Release payload verification also requires the
exact embedded helper digest. See [bundle installation](media-helper-bundles.md).

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

GStreamer app/base development libraries belong to the helper build, not the UI
crate. The common helper links these libraries at startup: missing them disables
**all helper modes**, including image/PDF thumbnails. The UI can show repair or
package guidance instead of loading Strata's media implementation at startup.
Some distributions' GTK builds themselves link GStreamer; removing libraries
required by the platform toolkit is not supported or claimed to work.

Audio-bearing previews require core/base raw-audio plugins, the `pulsesink` plugin
(`gst-plugins-good` / `gstreamer1.0-plugins-good`) and an available trusted audio
server. Unavailable audio fails that preview with actionable guidance, even if
muted; silent video-only degradation is not implemented. Video-only sources do
not need a PCM worker. Installing dependencies permits a new preview attempt;
thumbnail failures may remain cached for 30 seconds. Repairing a mismatched
package helper may require a restart because existing processes pin their helper.
The release's offline recovery payload can restore its exact helper, never an
arbitrary latest download. No private native runtime is bundled.

Installer/AUR metadata retains legacy codec-plugin recommendations for published
pre-helper binaries; the new player does not use those decoders.
Toolkit versions and the opt-in patches in
[`packaging/media-runtime`](../packaging/media-runtime/README.md) are unchanged.
The new player does not use the two patched `GtkGstSink`/`GstPlay` paths, but this
change neither applies nor retires that patch kit or claims to fix all RAM growth.

The split alone reproduced Ubuntu's runtime-library alias failure
[#806](https://github.com/lgse/strata/issues/806). A narrowly reviewed correction
binds only the fixed architecture-specific BLAS/LAPACK alias files read-only.
Their canonical ELF sources must reside under `/usr/lib`, with root-owned,
non-group/other-writable files and ancestry; the alias directory must also be
protected. No whole `/etc/alternatives` directory or media/environment-selected
alias is mounted. Invalid or missing candidates add no capability.

Real sandbox-to-GTK video, audio-only, A/V, GIF, seek/EOS, isolated worker-crash
and parent-death regressions now run in the pinned x86_64 Ubuntu environment.
Audio uses the isolated fake sink. This establishes that tested library-alias
path, not every Ubuntu installation, real speaker output, ARM64 behavior or a
memory plateau. Installed-artifact and platform evidence remain release gates.
