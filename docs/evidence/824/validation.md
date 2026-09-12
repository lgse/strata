# Validation handoff

The change crosses protocol validation, process supervision, GTK presentation,
shared browser/chooser previews, saved preferences and runtime packaging. It
therefore received full pinned Rust and E2E runs, not just filename-selected unit
tests. Focused reruns covered later playback/lifecycle fixes. Toolkit versions,
GPU/mount policy and the private runtime patch kit were not updated.

## Commands and results

- `STRATA_CONTAINER_ENGINE=docker python3 scripts/e2e_base.py build` — explicitly
  rebuilt once for the added development packages. Subsequent runs reused the
  verified image and existing caches; ordinary tests did not rebuild/download
  Ubuntu packages. Final image:
  `sha256:9227efaa471492be5a546242aad02a86e17d5ba8a4ca418f1fc839a573f9c71e`.
- `STRATA_CONTAINER_ENGINE=docker ./scripts/quality.sh test` — final tree:
  **1467 passed, 20 existing ignored, 0 failed** (362.39 s). The initial full run's
  two failures were corrected; this is the complete successful rerun.
- `STRATA_CONTAINER_ENGINE=docker ./scripts/quality.sh fmt` and
  `STRATA_CONTAINER_ENGINE=docker ./scripts/quality.sh clippy` — passed on final
  Rust code at the pre-push checkpoint.
- `STRATA_CONTAINER_ENGINE=docker ./scripts/e2e.sh` — **802 passed** (161.99 s).
  This full run preceded the final sample-boundary EOS and window-lifecycle fixes.
- `STRATA_CONTAINER_ENGINE=docker ./scripts/e2e.sh tests/e2e/scenarios/test_quick_preview.py`
  — **31 passed** after those fixes. This covers the changed preview interaction
  path; unrelated E2E scenarios were not rerun.
- `python3 scripts/test-headless.py -- media preview` inside the verified quality
  container — **107 passed**, nonzero selection. This covers transport rejection,
  worker slots/cancellation/backpressure, real generated helper media, player
  clocks/seeks/GIF/resize/idle/end/teardown, caller integration and live preferences.
  Followed by `cargo build --locked --all-features --bin strata` in that container.
- `PYTHONPATH=scripts python3 -m unittest test_installer test_update_aur test_e2e_packages`
  — **58 passed**. `python3 scripts/update_aur.py --stable 0.9.0 --preview 0.9.0`
  regenerated the checked-in packages without changing their existing versions.
  Legacy codec-plugin recommendations remain for currently published binaries.
- `python3 docs/evidence/824/benchmark.py --before target/824-evidence/before-app/strata --after target/824-evidence/after-app/strata`
  and `python3 docs/evidence/824/cached_seek.py` — all six generated fixture classes
  completed; raw results and endpoint definitions are in this directory.
- Native private-display experiments: startup, paused seeking, 31-second idle
  cleanup/resume, 20 and 100 rooted cycles; a separate 40-cycle allocator diagnostic.
  The committed runner was smoke-tested on final code with
  `python3 docs/evidence/824/native_smoke.py after --cycles 3` — passed, settled at
  24 FDs/73 threads and no media-helper/FFmpeg descendants.
- Python evidence scripts compile; local Markdown links and generated-fixture
  command syntax checked; `git diff --check` passed.

Rust GUI tests require GTK and run on private Xvfb/D-Bus with accessibility
bridging disabled. E2E/native accessibility runs use their private AT-SPI bus and
isolated HOME. No inherited desktop or audio-server session was used. Docker
was used through the canonical pinned-container runners; this is not a claim
of rootless Podman execution. Native profiling is additional evidence, not a
replacement for the canonical pinned E2E suite.

## Limits and owner decisions

- The owner approved four simultaneous sessions per application process and
  releasing decoder/audio resources after 30 seconds paused, retaining the frame
  and position. Seeks/reopens and idle resume restart the original decoder; they
  require the original file to remain accessible and can be slower than cached
  normalized-file playback.
- By explicit owner direction, **#806 remains out of scope**. Native software
  playback works on the measured Arch environment; do not infer working playback
  on installations with the unresolved Ubuntu library-alias failure from an E2E
  pass. Sandbox mounts/permissions were not broadened to make that environment work.
- Actual speakers, hardware-accelerated decoding/presentation, ARM64, installed
  upgrade/rollback and an RSS/VRAM plateau were **not validated**. Private-runtime
  patch regressions were not rerun because that opt-in baseline was not changed
  or integrated. Required GitHub checks remain authoritative before merge.
- Resource evidence distinguishes tracked owners, allocator retention and the
  upstream legacy `GtkMediaFile` FD/thread bug. No process-wide allocator setting
  or claim of fixing every toolkit/driver memory issue is included.

Local full logs are `/tmp/strata-824-quality-test-final.log`,
`/tmp/strata-824-e2e.log`, `/tmp/strata-824-e2e-preview-final.log`,
`/tmp/strata-824-fmt.log`, `/tmp/strata-824-clippy.log`,
`/tmp/strata-824-targeted-final-5.log` and
`/tmp/strata-824-packaging-tests.log`. Native raw artifacts remain under
`target/824-evidence/`; committed evidence is synthetic and cropped/summarized.
