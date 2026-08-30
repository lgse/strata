# Preview sandbox

Strata treats files shown while browsing as untrusted. Native parsers do not receive the user's normal filesystem or network access.

## Sandboxed providers

The following providers run in a short-lived helper process:

- GDK Pixbuf image and camera RAW loaders;
- Poppler PDF thumbnail and page rendering;
- ImageMagick and `dcraw`/`dcraw_emu` RAW fallbacks; and
- `ffmpegthumbnailer` media thumbnails.

Image and media previews are normalized to PNG by the helper. Media previews are intentionally static so GStreamer does not parse an untrusted file in the Strata process. Plain-text previews remain in-process and are limited to 1 MB; they do not invoke a native format parser.

## Isolation and limits

Strata starts its own executable in a bubblewrap sandbox. The sandbox has:

- a new user, mount, PID, IPC, UTS, cgroup, and network namespace;
- read-only access to `/usr`, required runtime libraries and font/ImageMagick configuration, the Strata executable, and exactly one canonicalized input file;
- writable access only to private mode-0700 output and temporary directories;
- an empty environment with a nonexistent home directory;
- 512 MB address-space, 10-second CPU, 32 MB file-size, and 12-second wall-clock limits.

The parent accepts only a bounded PNG result. Cancellation or timeout kills bubblewrap, which is the sandbox PID-namespace init process, and therefore tears down all helper descendants. A missing bubblewrap installation, renderer crash, malformed result, timeout, or permission failure is fail-closed and produces the normal fallback icon or **Preview unavailable** message.
