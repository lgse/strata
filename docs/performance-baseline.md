# Performance Baseline

This document defines reproducible local performance checks for Strata. Results are machine-specific and should be compared against earlier runs on the same system rather than treated as universal benchmarks.

For the detailed Rune-era large-directory study (99,998 images across Columns, List, grouped List, and Icons modes, including windowed launch, offscreen phase breakdown, micro-benchmarks, and follow-up optimization measurements), see [performance/large-directory-open.md](performance/large-directory-open.md).

## Fixtures

Generate deterministic mixed directories containing 1,000, 10,000, and 100,000 entries, plus deep-tree and native-path edge cases:

```bash
./scripts/generate-fixture.sh target/fixtures --clean
```

Omit `--clean` to fill in missing fixture entries without deleting existing ones. On a modern desktop, clean generation of all fixtures takes approximately one second.

## Instrumented run

Build once so compilation is excluded from startup measurements:

```bash
cargo build --release
RUST_LOG=strata=debug target/release/strata target/fixtures/100000
```

Default logs contain request IDs, backend names, counts, and timings without browsed locations.
`RUST_LOG=strata=debug` explicitly enables diagnostic logging and may include full native paths.
Remote URI user-info, authentication parameters, queries, and fragments remain redacted at every
level. Review diagnostic logs before sharing them.

Strata accepts a startup directory on the command line. Structured logs report:

- Time until the application window is presented
- First provider batch latency
- Time until the first directory batch is rendered
- Complete enumeration time and entry count
- Complete UI publication
- Per-batch UI append duration
- Cancelled directory request IDs

Capture sampled RSS and proportional set size (PSS) with:

```bash
STRATA_BINARY=target/release/strata \
  ./scripts/profile-fixture.sh target/fixtures/100000
```

Close other Strata instances before profiling so single-instance D-Bus activation does not forward the request to an existing process. The script samples until the complete model has been published, not merely until enumeration finishes. PSS is included because RSS charges each process for shared graphics, GLib/GIO, and font pages in full.

## Offscreen and windowed Rune profiling

The root `Cargo.toml` defines a `perf` profile (`opt-level = 3`, `codegen-units = 16`, `lto = false`, no debug info) for fast iteration on offscreen phase benchmarks and micro-benchmarks via `strata_app::host::offscreen`:

```bash
cargo test --profile perf <benchmark_filter> -- --ignored
```

For end-to-end launch and memory measurements, run the `--release` binary (`lto = true`, `codegen-units = 1`) in a private headless `sway` session (`WLR_RENDERER=gles2`) with a private session D-Bus, measuring window map time, process-tree CPU settle time, and peak process-tree PSS from `/proc/<pid>/smaps_rollup`.

See [performance/large-directory-open.md](performance/large-directory-open.md) for the recorded Rune baseline and post-optimization numbers:

- **Empty directory (0 files, `--release`):** ~560–573 ms mapped, ~588–600 ms settled, ~145–147 MiB PSS.
- **99,998 images (`--release`, private headless sway):** ~877–954 ms settled across Columns, List, grouped List, and Icons modes, ~253–255 MiB peak process-tree PSS (~225 MiB main + 4 × ~6.8 MiB sandboxed `--browser-worker` processes).
- **Offscreen `LoadFinished` (`--profile perf`, 99,998 images):** ~170–183 ms total (~98–106 ms enumeration + sort, ~105–113 ms to first 128 visible rows).

## Regression budgets

Use these provisional guardrails:

- The first provider batch should remain below 100 ms on the target machine.
- A 100,000-entry directory must remain scrollable while enumeration continues.
- UI frame times during staged publication and scrolling should stay within one 16.7 ms frame budget.
- Navigation away from a loading fixture must cancel its request without stale rows appearing.
- Long-lived sessions visiting multiple large directories must keep entry-ID and view caches bounded (pruned between folder/tab transitions).

Record hardware, build profile, graphics driver/compositor, cache state, and notable environmental load whenever replacing baseline tables. Use multiple samples before treating small timing differences as regressions.
