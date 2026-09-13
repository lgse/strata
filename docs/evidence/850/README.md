# #850 release-boundary evidence

## Optional local diagnostics

```sh
STRATA_CONTAINER_ENGINE=podman ./scripts/release-gate.sh --build-environment
```

The explicit build flag bootstraps an unpublished environment from the pinned E2E
recipe. Subsequent runs reuse its input/architecture/UID-keyed image. The runner
uses native x86_64 or AArch64, isolated container storage supplied by the caller,
private Xvfb/D-Bus, and no network during installed-artifact checks. It never
publishes a tag, release, package, or artifact to a release channel. This is an
explicitly opt-in manual tool, not called by CI or the Release workflow. At the
owner's request, the added native release-test job was removed and its active
validation run cancelled; Release performs build/package/publish only. Normal
pull-request CI remains separate; its canonical E2E runner is `scripts/e2e.sh`.
Use a fresh `STRATA_RELEASE_GATE_OUTPUT` directory for each run; Cargo caches stay
under `target/release-gate`.

The Ubuntu bootstrap is the immutable `noble-20260810` multiarchitecture index
`sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517`.
It selects the unchanged x86_64 image
`sha256:1e0a86e57d247923571b75e0aaf48a1449cf8c543d51fb3e07a4a7d7bfa79316`
and native ARM64 image
`sha256:95fa486768020359141f1318720f43e7982ef926c792891d984aef9aaf05e7ea`.
The previous architecture-specific pin failed on ARM64 before compilation with
`Exec format error`. Lifting that pin to its containing index changes neither
the x86_64 filesystem nor the authenticated Ubuntu package snapshot. It does not
update toolkit versions or apply/remove any private-runtime patch.

When explicitly invoked, the diagnostic tool builds two real release generations
through the same producer as the release workflow, then checks:

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

The ordinary `legacy_updaters_round_trip_without_mutating_cached_releases`
regression runs both frozen updater families through the shell installer's
modern → legacy → old-updater → modern → legacy path. It uses the test runner's
ELF as an inert archive payload, not as a real Strata release, and never executes
that payload. The adjacent Rust bundle regression covers the in-app installer's
flat-launcher restoration and cache integrity; portal/restart regressions cover
old windows following a flat rollback launcher.

The two native-archive Rust tests are explicitly ignored in ordinary unit runs
because they need freshly produced archives. The optional diagnostic tool selects
and runs both; an ordinary suite's ignore count is not evidence that they passed.

## Local observations

The full x86_64 installed-artifact diagnostic run passed at `ef81ea29f1c2fbf83fc116e7841b90fa7f6b03d0`.
ARM64 runtime validation was not completed before the owner-directed cancellation;
architecture-specific release builds remain configured, not runtime proof. No
additional release-validation run is required or dispatched by the release workflow.

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
