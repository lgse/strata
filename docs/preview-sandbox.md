# Preview sandbox

Strata treats files shown while browsing as untrusted. Native parsers do not receive the user's normal filesystem or network access.

## Sandboxed providers

The following providers run in a short-lived helper process:

- GDK Pixbuf image and camera RAW loaders;
- Poppler PDF thumbnail and page rendering;
- ImageMagick and `dcraw`/`simple_dcraw` RAW fallbacks; and
- `ffmpegthumbnailer` media thumbnails.

Image previews are normalized to PNG by the helper. Video previews are limited to the first 30 seconds of content, even for hour-long sources, and at most 30 frames per second. The requested video size follows the preview pane and display scale at load time, capped at 1280 pixels on either axis. Scaling fits that rectangle without enlarging smaller sources, subject to codec minimum dimensions and alignment. Resizing does not interrupt playback; the next load uses the new size. Recent media cache entries distinguish requested sizes and backend preferences.

Hardware acceleration is enabled by default except when an unset preference is paired with an AMD Polaris GPU; those systems start with software previews but can opt in from General settings. Automatic mode tries VA-API, then Vulkan, then software. A selected VA-API or Vulkan backend falls directly back to software if it fails. Hardware paths produce H.264/AAC MP4 with both dimensions aligned to 16 pixels. Software first tries H.264/AAC MP4 using libx264's ultrafast, zero-latency preset, then falls back to VP8/Opus WebM if unavailable or unsuccessful. Software dimensions are aligned to two pixels. Every path still decodes and re-encodes inside the sandbox; it never stream-copies the original compressed media. Playback currently waits for the complete normalized clip. A worker prepares that clip in a mode-0600 temporary file because GTK 4.14's GStreamer backend does not support input streams. The recent-preview cache and active player share ownership without copying the clip; the file is removed when its last owner is released. The cache's existing byte and entry limits include these files. This keeps GStreamer from parsing the selected untrusted file directly. Plain-text previews remain in-process and are limited to 1 MB; they do not invoke a native format parser.

Thumbnail rendering uses one helper at a time and queues at most 64 unique requests. Live rows deferred by a full queue are retried as capacity opens, duplicate requests share one render, rows leaving the view cancel work that has no remaining targets, and failed renders are cached for 30 seconds to prevent retry loops.

## Isolation and limits

Strata starts its own executable in a bubblewrap sandbox. The sandbox has:

- a new user, mount, PID, IPC, UTS, cgroup, and network namespace;
- read-only access to `/usr`, required runtime libraries and font/ImageMagick configuration, the Strata executable, and exactly one canonicalized input file;
- writable access only to private mode-0700 output and temporary directories;
- an empty environment with a nonexistent home directory;
- a 512 MB input limit for raster and PDF parsing, a 2 GB address-space limit allowing modern image loaders to start their isolated worker threads, a 512 MB sandbox file-size limit for decoder buffers, and a 32 MB parent-side output limit;
- a 12-second wall-clock limit for image, PDF, and thumbnail rendering, plus a 10-second CPU limit; and
- a 30-second wall-clock limit for media previews, which have no cumulative CPU limit because FFmpeg uses multiple threads. Hardware attempts are limited to 8 seconds each and 12 seconds collectively. The software H.264 attempt is limited to 8 seconds, leaving the remaining helper budget for VP8 fallback; adding that attempt does not extend the overall deadline.

Accelerated media previews receive only the devices required by their policy: VA-API gets safe `/dev/dri/renderD<digits>` nodes, while Vulkan and Automatic may also get `/dev/nvidia<digits>` and `/dev/nvidiactl`. They receive read-only `/sys` access for driver discovery. Software media previews, image, PDF, and thumbnail helpers receive no GPU devices or `/sys` mount.

Strata reads PCI vendor and device IDs from `/sys/class/drm/renderD*/device` to detect AMD Polaris 10, 11, and 12 devices (`0x67c0–0x67df`, `0x67e0–0x67ff`, and `0x6980–0x699f`). Because preview encoding can hang on them, an unset acceleration preference resolves to software when any Polaris render node is present. The settings remain available as an explicit opt-in, after which the selected hardware policy receives the render node normally. Nodes with unreadable metadata retain the non-Polaris default.

GPU acceleration expands the media helper's attack surface into the installed userspace and kernel GPU drivers; policy-specific device access keeps that exposure media-only and the existing namespaces and resource limits still apply.

External thumbnail providers have bounded stdout and discarded stderr. The parent accepts only a size- and dimension-bounded PNG, MP4 with an `ftyp` signature, or WebM with an EBML signature. Failed or unavailable hardware attempts advance to the next backend, while a failed final software attempt produces the normal unavailable-preview result. Cancellation or timeout kills the renderer process group and bubblewrap, whose PID namespace also tears down descendants that create a new process group. A missing bubblewrap installation, renderer crash, malformed result, timeout, or permission failure is fail-closed and produces the normal fallback icon or **Preview unavailable** message.
