# Investigate preview resource retention

Follow-up [#779](https://github.com/lgse/strata/issues/779) tracks residual RAM,
file-descriptor, and small VRAM growth after the large lifecycle leak addressed by
[#765](https://github.com/lgse/strata/pull/765). Start with a build containing that
fix. See the [measured before/after evidence](evidence/779/README.md) before choosing
another fix: still-image phases were flat, cached media reopenings did not launch
FFmpeg, and all fixed-build GTK media hierarchies finalized.

These diagnostics observe behavior; they do not fix the residual issue. Trace
output is disabled unless `STRATA_PREVIEW_TRACE=1`. No NVIDIA dependency or GPU
polling is added to Strata. The external Python sampler uses installed
`nvidia-smi` (including graphics clients) and Linux `/proc`, with no extra Python
packages, root privileges, or changes to preview sandbox permissions. Media-helper
traces use a pipe capped at 64 KiB per job, not a writable host log descriptor;
only valid structured trace lines are relayed.

## Manual capture

Build against the native toolkit/driver used for reproduction:

```bash
cargo build --locked
python3 scripts/preview-memory.py --resources
```

Run the second command yourself in a desktop terminal. It opens a separate debug
instance on a **private D-Bus session**, using your desktop display/GPU. It does not
drive input or start automated GUI tests. Automated checks must still use private
Xvfb and D-Bus per [the testing guide](e2e-testing.md).

The first launch copies saved Strata settings/custom themes into
`target/preview-memory-profile`. Subsequent launches reuse that private profile;
changes there do not modify the installed application's preferences. Its thumbnail
cache starts empty. Dismiss any first-run integration prompt rather than installing
a portal for this experiment.

1. Keep one window, one media file, and the acceleration preference constant.
   Start with **software conversion**, matching the existing evidence.
2. Wait ten seconds before the first preview for baseline resource snapshots.
3. Open/play/close the same GIF or MP4 repeatedly. Wait at least five seconds with
   the preview closed between cycles so periodic resource samples can capture
   settled state. Extend to **30–50 cycles** to distinguish warm-up from ongoing
   growth; do not assume either a leak or an allocator plateau from RSS alone.
4. Use repeated JPEG/WebP previews as a separate control. Leave the preview closed
   for ten seconds, then **close the debug window**. A five-second sampling tail
   is recorded before the sampler exits.
5. Compare resource categories and PSS/RSS after close, aligned to finalization
   events. Identify still-live owners before changing cache limits or backends.

Restart between acceleration-setting comparisons: the encoded cache key does not
include backend policy. The trace records the effective conversion policy and
actual backend attempts. This preference does not disable all GTK playback/display
GPU use.

## Output and selection

Each run creates a new directory under `target/preview-memory/`:

- `app.log`: `STRATA_PREVIEW_TRACE` JSON records plus ordinary application logs.
- `samples.jsonl`: main/descendant process identities, RSS, thread/fd counts,
  namespace PIDs, graphics-inclusive per-PID VRAM, and GPU totals.
- With `--resources`, each process also has periodic `resources` snapshots:
  FD categories (GPU devices, sockets, pipes, eventfds, memfds, etc.), allowlisted
  thread-name categories, and selected numeric `smaps_rollup` fields including PSS
  and private/shared pages. These are sampled approximately every five seconds,
  not every 500 ms memory sample. Unknown thread names are grouped as `other`;
  categories are diagnostic hints, not proof of ownership.
- `metadata.json`: driver/GPU, binary SHA256, sampling intervals, profile location,
  and selected renderer environment overrides.
- `finished.json`: capture completion/application exit status when available.

Unreadable/racing FD and thread entries have explicit counts; unavailable resource
collections and PSS have error markers, not successful empty/zero values. Resource
snapshots whose process exits or changes identity are discarded. Snapshots are not
atomic: threads and descriptors can change while `/proc` is read. Resource
collection overhead appears in the overall sample duration.

The main Strata PID is in the `startup` trace. Do not confuse it with the
`dbus-run-session` launcher or sum the entire process tree to infer application
memory. GPU totals include unrelated desktop applications.

Select a preserved comparison binary, an existing process, or a different profile:

```bash
python3 scripts/preview-memory.py --binary /path/to/strata --resources
python3 scripts/preview-memory.py --pid 12345 --resources --seconds 60
python3 scripts/preview-memory.py --profile target/another-preview-profile
```

`--binary` defaults to `target/debug/strata`; no old build is selected implicitly.
Attach mode cannot enable traces in an already-running process. `--output` must
name a new directory; existing captures are never overwritten. Ctrl+C or
`--seconds 60` stops sampling **without killing the application**. Close its window
separately; it can continue writing `app.log` until it exits. Capture files grow
until recording stops.

## Trace interpretation

- `preview_load` / `preview_close` identify actions. Request IDs are drawer-local;
  use one window initially to avoid ambiguous provider IDs across drawers.
- `cache_hit` reuses encoded bytes without another conversion. `cache_store`
  reports encoded cache bytes, not decoded RAM/VRAM or allocator reservations.
- `sandbox_started` maps a job token to a host child PID. Events with `sandbox_job`
  report helper/FFmpeg PIDs **inside that sandbox**; sampled `namespace_pids` can
  map surviving processes to their host PIDs. Brief processes can be missed.
- `media_open`, `media_created`, `media_prepared`, and `media_playing` identify
  player creation/state. Backend and GTK renderer types are included. Prepared/
  playing notifications do not prove the first frame was painted.
- `content_clear_begin/end` bracket cleanup. `object_finalized` separately reports
  destruction. Observers hold no strong references to observed objects.
- `load_handle_dropped` requests cancellation but can occur after successful
  completion; it does not by itself prove a running helper was killed.

Match `unix_ms` timestamps. The default memory sampling interval is 500 ms, but
queries take time (`sample_end_unix_ms`); exclude samples overlapping the next open
when reporting post-close levels. Unavailable NVIDIA values and absent GPU process
records are **not zero**. Neither GPU accounting nor PSS provides an allocation
backtrace. Compare bounded playback peaks separately from growth in the settled
baseline, and compare like-for-like builds/toolkits.

## Privacy

New traces use randomized per-process file tokens, not filenames. Resource
snapshots emit categories instead of raw FD targets or arbitrary thread names;
no file contents, stack traces, full memory maps, or core dumps are collected.
Ordinary GTK/application log lines and process names may still contain private
information. Review captures before sharing; keep the private profile/settings
local. Nothing is uploaded automatically. The committed evidence deliberately
excludes raw application logs and user settings.
