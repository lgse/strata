# Preview sandbox

Strata treats files shown while browsing as untrusted. Native parsers do not receive the user's normal filesystem or network access.

## Sandboxed providers

The following providers run in a short-lived helper process:

- GDK Pixbuf image and camera RAW loaders;
- Poppler PDF thumbnail and page rendering;
- ImageMagick and `dcraw`/`dcraw_emu` RAW fallbacks; and
- `ffmpegthumbnailer` media thumbnails.

Image previews are normalized to PNG by the helper. Video previews are limited to the first 30 seconds, at most 1280 pixels on either axis, and at most 30 frames per second. They are normalized by VA-API first, Vulkan second, or the software VP8 fallback. Hardware paths produce H.264/AAC MP4; the software path produces VP8/Opus WebM. This keeps GStreamer from parsing the selected untrusted file directly. Plain-text previews remain in-process and are limited to 1 MB; they do not invoke a native format parser.

## Isolation and limits

Strata starts its own executable in a bubblewrap sandbox. The sandbox has:

- a new user, mount, PID, IPC, UTS, cgroup, and network namespace;
- read-only access to `/usr`, required runtime libraries and font/ImageMagick configuration, the Strata executable, and exactly one canonicalized input file;
- writable access only to private mode-0700 output and temporary directories;
- an empty environment with a nonexistent home directory;
- a 2 GB address-space limit, allowing modern image loaders to start their isolated worker threads, and a 32 MB file-size limit;
- a 12-second wall-clock limit for image, PDF, and thumbnail rendering, plus a 10-second CPU limit; and
- a 30-second wall-clock limit for media previews, which have no cumulative CPU limit because FFmpeg uses multiple threads. Hardware attempts are limited to 8 seconds each and 12 seconds collectively so the software fallback retains time to run.

Media previews additionally receive only discovered `/dev/dri/renderD<digits>`, `/dev/nvidia<digits>`, and `/dev/nvidiactl` devices plus read-only `/sys` access for driver discovery. Image, PDF, and thumbnail helpers receive none of these mounts. GPU acceleration expands the media helper's attack surface into the installed userspace and kernel GPU drivers; device access remains media-only and the existing namespaces and resource limits still apply.

The parent accepts only a bounded PNG, MP4 with an `ftyp` signature, or WebM with an EBML signature. Failed or unavailable hardware attempts advance to the next backend, while a failed final software attempt produces the normal unavailable-preview result. Cancellation or timeout kills bubblewrap, which is the sandbox PID-namespace init process, and therefore tears down all helper descendants. A missing bubblewrap installation, renderer crash, malformed result, timeout, or permission failure is fail-closed and produces the normal fallback icon or **Preview unavailable** message.
