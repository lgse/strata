# #850 release-boundary evidence

## Repeatable, non-publishing gate

```sh
STRATA_CONTAINER_ENGINE=podman ./scripts/release-gate.sh --build-environment
```

The explicit build flag bootstraps an unpublished environment from the pinned E2E
recipe. Subsequent runs reuse its input/architecture/UID-keyed image. The runner
uses native x86_64 or AArch64, isolated container storage supplied by the caller,
private Xvfb/D-Bus, and no network during installed-artifact checks. It never
publishes a tag, release, package, or artifact to a release channel. CI runs both
native architectures. The normal canonical E2E runner remains `scripts/e2e.sh`.
Use a fresh `STRATA_RELEASE_GATE_OUTPUT` directory for each run; Cargo caches stay
under `target/release-gate`.

The gate builds two real release generations through the same producer as the
release workflow, then checks:

- Manifest hashes, static ELF architecture, useful separate debug line tables,
  exact debug-link CRCs, and distributed license bytes.
- Frozen v0.4.0/v0.16.0 single-file installation routines against the two-binary
  archive, followed by actual UI startup/playback with offline helper recovery.
  `tests/fixtures/legacy_update/provenance.json` identifies the immutable sources;
  `scripts/check-legacy-fixtures.py` checks their bodies against Git history.
  These fixtures substitute network transport and desktop/portal callbacks, not
  extraction, executable selection, staging, permissions or replacement. They
  are not a claim that every historical GUI client contacted a published release.
- The current Rust updater's real-archive legacy migration, rollback and forward
  update; the shell installer's real-bundle activation, concurrent lock exclusion,
  permission failure and actual ENOSPC on a private size-limited tmpfs.
- An old running UI starting a new matching decode after activation; installed
  video playback/seeking, 100 open/close cycles, and paused-worker release/resume.
- Missing/mismatched sidecar recovery; a private PulseAudio null sink and monitor
  checking actual `pulsesink` output, live mute, volume changes, EOS and cleanup.
- Disposable-container removal/restoration of common GStreamer libraries,
  FFmpeg/ffprobe, bubblewrap and base/good plugins. The UI must show guidance,
  retain text previews, and recover without restarting after restoration. An
  absent audio server has a separate unavailable-capability case.

The two native-archive Rust tests are explicitly ignored in ordinary unit runs
because they need freshly produced archives. The release gate selects and runs
both; an ordinary suite's ignore count is not evidence that they passed.

## Local observations

Native x86_64 Ubuntu release artifacts at source `a26f165b1555470a6a92832d421c915d2525c93f`
passed installed shell upgrade/rollback, live-old-instance playback, ENOSPC,
permission/lock failures, exact offline recovery and mismatched-sidecar recovery.
The subsequent current-Rust-updater and frozen historical-routine checks also
passed with those artifacts. These are deliberately executed, locally built test
programs; installation validation itself never executes downloaded binaries.

One 100-cycle installed video run retained 15 application FDs and 70 threads, with
zero remaining helper/FFmpeg processes after every close. Closed-cycle application
RSS was 156,316 KiB and tree PSS 101,161 KiB throughout that run; no smaps reads
failed. A 31-second pause reduced the process tree from one helper plus one
FFmpeg process to neither, and playback resumed afterward. Paused seeks in that
run took approximately 91 ms. These are one generated fixture/environment, not
startup speedup, GPU-memory, universal memory-plateau or long-duration claims.

Real PulseAudio transport exposed `SIGXFSZ`: a 64-MiB shared-memory allocation
exceeded the worker's 4-MiB file-size budget. The fix stages a worker-private
client configuration disabling SHM, memfd and autospawn; resource limits remain
unchanged. Private-monitor peaks were 435 initially, 0 while muted and 724 after
raising volume from 0.15 to 0.25. A consented generated-tone A/V test also traversed
the normal PipeWire/PulseAudio service, reached EOS and released all workers;
physical speaker audibility was not independently confirmed.

The standalone evidence driver records raw JSON, logs and screenshots under its
requested output directory. `one_second_label_wall_ms` is a playback-label timing,
not application startup or direct first-frame latency. GUI evidence uses no
inherited desktop/bus; private PulseAudio records only its generated null-sink
stream. `--audio-runtime` additionally requires `--allow-audible` and operator
consent. No private media-runtime patch is shipped or retired by this work.
