# Media helper bundles

Strata ships `strata` and `strata-media-helper` together. The helper contains native
image/RAW/PDF parsing, FFmpeg orchestration and GStreamer PCM output; the UI owns
validated presentation and asynchronous supervision. There is no media daemon,
private GStreamer runtime, downloaded plugin or unsandboxed fallback. See the
[process/protocol boundary](preview-sandbox.md).

## Dependencies and failures

| Capability | Requirements / recovery |
| --- | --- |
| UI launch | GTK 4, GtkSourceView, GLib/GIO and the platform toolkit's transitive dependencies. Strata's UI crate no longer adds GStreamer/Poppler linkage. Some GTK packages themselves require GStreamer. |
| Any helper mode | Matching, executable, correct-architecture helper; GStreamer core/app/base, Poppler/Pixbuf and their runtime dependencies. Missing common libraries disables **all** helper modes, including images/PDF. |
| Original-file parsing | Bubblewrap and user namespaces; no fallback when isolation fails. |
| Video/audio/GIF decoding | FFmpeg and ffprobe. Video thumbnails additionally need ffmpegthumbnailer; RAW fallbacks remain capability-dependent. |
| PCM playback | GStreamer base raw-audio elements and `pulsesink` from the good plugins; PulseAudio or PipeWire with pipewire-pulse. No native PipeWire, ALSA-device or arbitrary sink fallback. |

Arch/Omarchy packages include `gstreamer`, `gst-plugins-base-libs`,
`gst-plugins-base`, `gst-plugins-good`, `poppler-glib`, `ffmpeg`, and
`ffmpegthumbnailer`. Debian/Ubuntu equivalents include `libgstreamer1.0-0`,
`libgstreamer-plugins-base1.0-0`, `gstreamer1.0-plugins-base`,
`gstreamer1.0-plugins-good`, the distribution's Poppler GLib runtime, `ffmpeg`,
and `ffmpegthumbnailer`. Installer/package dependencies intentionally cover the
normal media capabilities, not just the minimal dynamic-loader requirement.
Legacy codec plugin recommendations support older published players; the new
player does not decode compressed media in GStreamer.

Diagnostics distinguish missing helper, permissions/architecture/corruption,
version/protocol mismatch, loader libraries, tools, plugins, audio server,
sandbox startup and saturation. Startup stderr is bounded and sanitized; file
paths/contents and arbitrary suggested package commands are not forwarded.
Audio failure makes an audio-bearing preview unavailable even when muted; silent
video-only degradation is not implemented. Browsing and unrelated UI remain
available. Retry after installing dependencies; cached thumbnail failures can
last 30 seconds. Restart after replacing a mismatched package helper because a
running process deliberately retains its original helper inode.

## Standalone layout and activation

```text
<bin>/strata -> .strata-bundles/current/strata
<bin>/.strata-bundles/
  install.lock
  current  -> versions/<archive-sha256>
  previous -> versions/<previous-id>
  versions/<archive-sha256>/
    strata
    strata-media-helper
    bundle.json
    SOURCE_COMMIT, licenses, desktop/icon/portal templates, …
```

`bundle.json` format 1 identifies the exact release tag, GNU/Linux target,
40-hex source commit and media protocol 1. Its `files` object hashes every regular
archive member except the manifest. The archive retains the established
`strata-<version>-<target>/` root for old installers. Both executables have useful,
separate debug artifacts; checksums and archive provenance cover the pair.

The updater validates the GitHub release URL before downloading. Installer and
updater check bounded archive contents, path traversal, links, duplicate paths
and manifest keys, required files, identity, hashes and static ELF architecture.
Downloaded binaries are **not executed for validation**. Missing optional shared
libraries do not invalidate a correctly staged installation.

Staging and activation occur on the destination filesystem under a shared
cross-process `flock`. Parent directories must have trusted ownership and safe
permissions; unsafe ancestry is rejected before staging. The complete version is
synced before one pointer is atomically replaced. A regular launcher is preserved
before conversion, including a UI replaced by a legacy updater while an older
`current` pointer still exists. Verification/locking/pre-activation failures leave
the current installation usable. A post-activation directory-sync failure is
reported as a durability warning, not a false assertion that activation never
happened. Desktop/icon refresh is best-effort after activation; saved user opt-ins
are not created or reset by an update.

Restarts and portal/D-Bus launch paths use the stable launcher. Helper discovery
uses the running executable's immutable directory or pinned inode, **not** the
current pointer, PATH or cwd. Thus an old window can finish using its matching
helper after an update. Archive metadata remains with each version; rollback
through the updater refreshes existing desktop metadata from the selected archive.
A manually switched pointer alone does not refresh external desktop/icon caches.

AUR/Omarchy/pacman-owned installations retain `/usr/bin` paths and package-manager
ownership. Both executables are packaged without stripping/modification relative
to the attested archive. In-app standalone installation must not replace those
files. Prefix-relative ownership markers remain discoverable in versioned layouts.

## Published binary-only clients and rollback

All published tags audited through `v0.17.0-nightly.20260912` predate this boundary.
The old updater implementations install only `strata`: the single `find_binary`
contract in v0.4–v0.7 and `find_binaries(..., &["strata"])` in subsequent versions.
v0.2/v0.3 have no `services/update_install.rs`. Merely adding a helper archive
member or publishing a transition release cannot handle clients skipping it.

Every future **release build** therefore embeds a compressed copy and SHA-256 of
its exact final helper, after helper stripping/debug-link creation. Release builds
fail without `STRATA_MEDIA_HELPER_BUNDLE`. If a legacy updater discards the sibling,
the new UI can recover that exact helper offline before a preview job. It never
fetches a latest helper, and never executes an archive member to discover a version.

Recovery uses mode-0700, digest-keyed user cache storage, a process lock,
decompression/size/hash checks, staged rename and fsync. A failed recovery leaves
the UI usable and reports repair/reinstall guidance. A later standalone update
migrates the original launcher into the complete versioned layout. To explicitly
repair/retry the installed release's helper:

```sh
strata --repair-media-helper
```

The recovery cache is under `$XDG_CACHE_HOME/strata-media-recovery`, or
`$HOME/.cache/strata-media-recovery`. Corrupt cache entries fail closed: close all
Strata/portal instances, remove only the reported release's cache entry, then
retry or reinstall the complete bundle. Source builds without an embedded payload
must build/install the sibling helper; they cannot perform release recovery.

Rollback to old binary-only archives is allowed only for the frozen
[audited published tags](../src/services/update_install/bundle/legacy-releases.txt).
Both installers enforce this compatibility set; unknown/new tags require a
complete manifest and helper. A cached legacy archive is re-extracted and compared
before reuse, since those releases have no per-member manifest. Do not expand this
list automatically when publishing new releases.

Legacy releases run from a separate **regular `<bin>/strata` file**, not from the
cached version directory. Their published updaters replace `current_exe()` and
would otherwise overwrite the immutable rollback copy. Rollback atomically
replaces the launcher with a verified flat copy **before** recording the legacy
`current` pointer. That pointer records the selected archive; while the launcher
is regular, the launcher itself is authoritative and may be replaced by an old
updater. A post-commit pointer/durability failure is reported as a warning, not as
an uncommitted installation.

An old client can then upgrade normally, recovering the exact embedded helper
offline. Its next complete-bundle update preserves the actual flat executable and
restores the modern symlink layout. Desktop/portal entry points remain
`<bin>/strata`, and older modern windows can restart through either launcher
form. Cached legacy archives stay untouched, allowing another rollback after
**new → old → old updater → new**. Do not manually launch or activate a cached
legacy executable through `current`: that bypasses this compatibility boundary.

No version is automatically garbage-collected: there is no reliable proof that
another application/portal process stopped using it. Eight retained versions
(including legacy backups) cap normal bundle storage; recovery has a similar cap.
For cleanup, close **all** application and portal instances, retain `current` and
`previous`, and remove only unused versions. Interrupted private staging directories
may be removed only after confirming no installer is active. Do not delete active
versions to make room. Uninstall must remove the launcher and its associated bundle
store, plus only the integrations the user installed; see [README](../README.md).

## Rollout and validation scope

Release publishes matching x86_64/aarch64 archives without running test, E2E,
quality/lint or runtime-validation jobs. Normal pull-request CI remains separate.
Keep old links immutable and publish package metadata only after matching release
artifacts exist. Every later release must continue carrying the recovery payload
while binary-only clients remain supported, including alpha/beta/RC/nightly builds.
Document the `current`/`previous` layout, repair command and common-helper capability
loss in release notes.

Unit/synthetic-ELF tests are not proof of installed release behavior. Pinned GTK
regressions and E2E use private displays/buses and fake audio, not host speakers.
The helper split initially reproduced Ubuntu's alternatives-library issue
[#806](https://github.com/lgse/strata/issues/806): FFmpeg could not resolve
`libblas.so.3` under the original mounts. The corrected policy binds only fixed
BLAS/LAPACK alias files, validated as protected root-owned `/usr/lib` ELF files.
It never mounts all of `/etc/alternatives`. Real sandbox/player regressions now
exercise video/audio/A/V/GIF, worker crashes and parent death in pinned x86_64
Ubuntu with fake audio. Other installations and real installed artifacts still
need their own evidence.

Optional, non-publishing diagnostics exercise real installed artifacts, historical
extraction routines, rollback, offline recovery, a monitored private PulseAudio
sink and dependency restoration. These are not release dependencies; the owner
explicitly removed the added native release-test job and cancelled its remaining
validation. See the [commands, actual evidence and scope limits](evidence/850/README.md).

Keep claims bounded to measured environments. ARM64 runtime validation was not
completed, and existing tests establish neither a universal memory/VRAM plateau
nor physical speaker audibility. The private media-runtime patch kit is neither
shipped nor retired by this change.
