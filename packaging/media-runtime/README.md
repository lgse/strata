# Private media runtime patch kit (#779)

This directory preserves two experimental toolkit fixes, pinned source archives,
and standalone reproducers for [#779](https://github.com/lgse/strata/issues/779).
It is independent of the [diagnostics PR #782](https://github.com/lgse/strata/pull/782).
**It does not change Strata's binary, installer, or release workflow. Building
Strata alone does not apply either patch. This is not a shipping runtime yet.**

## What the patches fix

- **GTK 4.22.4:** `GtkGstSink` obtains an owned `gst_context` but omits its
  release in dispose. Release the buffer pool and created GL context before
  releasing the wrapped context/display. This stops the measured per-cycle
  GL-thread/FD/VRAM accumulation.
- **GStreamer 1.28.6:** an in-flight API message can release the last `GstPlay`
  reference on the playback worker. Original disposal skips joining itself,
  allowing finalization while `gst_play_main()` still accesses the object.
  Retain the object once until that worker finishes teardown, including repeated
  dispose calls. Other-thread disposal still joins the playback thread.

The second patch is a lifetime candidate, not a general concurrency audit.
Last-reference release on other streaming threads and explicit disposal racing
across multiple threads remain unverified. Neither patch claims to fix all RAM
retention. An independent review found no defect in the GTK patch and identified
three GstPlay issues subsequently addressed: repeat-dispose reference acquisition,
the probe's bus-flushing contract, and sleep-based completion. The amended version
has targeted test evidence, not a second independent review or upstream approval.

## Source and licenses

Both patched C files carry LGPL-2.0-or-later headers; retain their notices.
GTK's project license is LGPL-2.1-or-later. Copies of each source archive's
`COPYING` text are in [licenses/](licenses/). `GtkGstSink` credits Matthew Waters
(2015); `GstPlay` credits Sebastian Dröge (2014–2015), Brijesh Singh (2015),
Stephan Hesse (2019–2020), and Philippe Normand (2020).
The standalone probes are original MIT-licensed test code. Changes and probes
were developed with AI assistance and checked locally; a human must take
responsibility for any upstream submission. GStreamer prohibits automated
GitLab submissions; its maintainers must be contacted by a human. GTK requires
AI disclosure and merge requests rather than patch attachments on issues.

## Build the two libraries (explicit opt-in)

Run in a disposable build environment, not by installing over `/usr`. Requirements
include a C toolchain, Meson >= 1.5, Ninja, pkg-config, GTK/GStreamer development
dependencies, GLib development generators, Wayland/Vulkan headers and shaders.
GTK 4.22.4 requires **GLib >= 2.84, Pango >= 1.56 and Cairo >= 1.18.2**.
The current Ubuntu 24.04 release job's system dependencies do not satisfy this
runtime. The archive checksums pin sources, **not** the compiler/dependency graph;
a pinned release build environment is still required before distribution.

From the repository root (each attempt uses a new build directory):

```bash
set -eu
repo="$PWD"
kit="$repo/packaging/media-runtime"
mkdir -p "$repo/target"
build=$(mktemp -d "$repo/target/media-runtime.XXXXXX")
cd "$build"
curl --fail --location --output gtk-4.22.4.tar.xz \
  https://download.gnome.org/sources/gtk/4.22/gtk-4.22.4.tar.xz
curl --fail --location --output gst-plugins-bad-1.28.6.tar.xz \
  https://gstreamer.freedesktop.org/src/gst-plugins-bad/gst-plugins-bad-1.28.6.tar.xz
sha256sum --check "$kit/sources.sha256"
tar -xf gtk-4.22.4.tar.xz
tar -xf gst-plugins-bad-1.28.6.tar.xz
patch --batch --fuzz=0 -d gtk-4.22.4 -p1 < "$kit/patches/gtk-release-gl-context.patch"
patch --batch --fuzz=0 -d gst-plugins-bad-1.28.6 -p1 < "$kit/patches/gstreamer-defer-worker-finalize.patch"
meson setup gtk-build gtk-4.22.4 --wrap-mode=nofallback \
  -Dbuild-demos=false -Dbuild-tests=false -Dbuild-testsuite=false \
  -Dbuild-examples=false -Dintrospection=disabled -Dprint-cups=disabled \
  -Ddocumentation=false -Dman-pages=false
meson setup gst-build gst-plugins-bad-1.28.6 --wrap-mode=nofallback \
  -Dauto_features=disabled -Dintrospection=disabled -Dtests=disabled -Dexamples=disabled
ninja -C gtk-build -j 6 gtk/libgtk-4.so.1.2200.4
ninja -C gst-build -j 6 gst-libs/gst/play/libgstplay-1.0.so.0.2806.0
runtime="$build/gtk-build/gtk:$build/gst-build/gst-libs/gst/play"
```

These targets build only the affected libraries; disabling upstream suites here
is **not** a statement that those suites pass. Reuse the same source/configuration
without applying patches to obtain a meaningful baseline. Never substitute a
newer/different toolkit as the baseline.

## Regression probes

Use generated media, not private files. Continue in the build directory above:

```bash
cc -Wall -Wextra -Werror -g "$kit/probes/play-lifetime.c" \
  -o play-lifetime $(pkg-config --cflags --libs gstreamer-play-1.0)
cc -Wall -Wextra -Werror -g "$kit/probes/gtk-lifecycle.c" \
  -o gtk-lifecycle $(pkg-config --cflags --libs gtk4)
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc2=size=128x128:rate=10 \
  -t 2 -c:v libx264 -pix_fmt yuv420p fixture.mp4
```

`play-lifetime` is non-GUI. It blocks an already-in-flight API message, leaves
the API bus flushing as documented, then drops the application reference.
It joins the worker and asserts exactly one finalization; a subprocess timeout
fails incomplete teardown rather than accepting an early weak notification.
The repeat-dispose variant invokes disposal twice on the worker. The main-dispose
variant checks ordinary application-thread teardown without starting playback.

```bash
ulimit -c 0
for mode in main-dispose worker repeat-dispose; do
  for trial in $(seq 1 10); do
    env -u DISPLAY -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS -u AT_SPI_BUS_ADDRESS \
      LD_LIBRARY_PATH="$runtime" timeout --kill-after=5s 15s \
      ./play-lifetime "$build/fixture.mp4" "$mode"
  done
done
```

Expected: all 30 runs exit zero with one completion message and no diagnostics.
The unpatched worker case aborts with a mutex/finalization error on the tested
version. Test it separately; an expected baseline failure must not hide a failure
of the patched run.

The GTK probe requires successful preparation and all media finalizations. It
prints one checkpoint per completed cycle, with one second playing and one second
closed. FD counts include the descriptor used to enumerate `/proc/self/fd`.
It does not identify allocation owners or directly weak-track private GL contexts.
Use **private Xvfb and private D-Bus**; never fall back to the desktop:

```bash
env -u DISPLAY -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS -u AT_SPI_BUS_ADDRESS \
  GTK_A11Y=none NO_AT_BRIDGE=1 GIO_USE_VFS=local GDK_BACKEND=x11 \
  GSK_RENDERER=gl LIBGL_ALWAYS_SOFTWARE=1 LD_LIBRARY_PATH="$runtime" \
  timeout --kill-after=5s 130s dbus-run-session -- \
  xvfb-run -a ./gtk-lifecycle "$build/fixture.mp4" 50
```

Expected: 50 finalized media and bounded post-warm-up FDs/threads. Absolute counts
are environment-dependent: compare the trend with the same-environment baseline,
not hard-coded counts. Missing Xvfb, a failed backend/decoder, incomplete cycles,
or timeout is a failed/unavailable check, not a skip-success. GL initialization
failure, non-GL fallback, and a longer Wayland/NVIDIA regression remain required
coverage before general release.

## Evidence and limits

Local host: Arch/Omarchy, GTK 4.22.4, GStreamer 1.28.6, GLib 2.88.3; owner-operated
Wayland/NVIDIA RTX 5080, driver 610.57.04. Native builds and manual desktop
reproduction were explicitly authorized. Automated GTK probes used isolated
Xvfb/D-Bus and software GL. No installed libraries or user integration were changed.

| Check | Unpatched | Candidate |
| --- | --- | --- |
| Standalone GTK cycles 1 → 12 | 56 → 78 FDs; 86 → 108 threads | flat after warm-up |
| Combined 50-cycle GTK run | not run for 50 cycles | 50 finalized; FDs 54; threads settled at 85 |
| Contract-compliant worker last-ref probe | SIGABRT | 10/10 passed |
| Main-thread / repeated worker disposal | main-thread passed; repeated not measured | 10/10 each passed |
| Latest traced NVIDIA run | earlier original app crashed on fifth close | 14 prepared cycles; 42 objects finalized; exit 0 |
| Candidate NVIDIA closed state | not an identical-work comparison | 47 MiB VRAM; 88 FDs; 24 threads |

The latest candidate's closed RSS still grew **326 → 531 MiB** across 14 cycles;
final PSS was about 468 MiB. The requested 30–50-cycle / 30-second-final-idle manual
protocol was not completed. Several intervals were shorter than five seconds.
Do not claim an established RAM plateau or complete resolution of #779.

Sanitized discussion:
[initial candidate measurements](https://github.com/lgse/strata/pull/782#issuecomment-5623792996),
[review corrections and latest capture](https://github.com/lgse/strata/pull/782#issuecomment-5623949139).
Raw captures, profiles, core dumps, and host-built binaries are deliberately absent.

## Shipping requirements (not implemented by this patch kit)

1. Choose and pin the supported runtime baseline for both x86_64 and aarch64.
   A pair of Arch-built `.so` files is not a portable Ubuntu release artifact.
2. Build and bundle the dependency closure required by the chosen toolkit.
   Keep driver libraries supplied by the host; verify GStreamer plugin and
   GtkSourceView/Poppler compatibility rather than copying arbitrary host libraries.
3. Make runtime selection private to Strata, with no global loader configuration.
   A launcher's `LD_LIBRARY_PATH` is inherited by children: account for file-open
   commands, D-Bus activation, portal execution, sandbox helper loading, and the
   installer's current single-executable assumptions before choosing that design.
4. Ship license notices, corresponding patched source and build instructions;
   preserve dynamic relinking rights and source provenance for LGPL components.
5. Validate the **installed release artifact** on supported distributions, including
   real media, sandbox helpers, portal/file-manager integration, updates and rollback.
   Run the full pinned repository checks when changing release/build infrastructure.
6. Publish a prerelease before promoting it. Describe the measured crash/GL-resource
   fixes separately from unresolved RAM growth. Remove/backport patches deliberately
   after upstream versions incorporate equivalent fixes; do not apply by fuzzy match.
