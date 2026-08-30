# Performance Baseline

This document defines the reproducible local performance check for Strata. Results are machine-specific and should be compared against earlier runs on the same system rather than treated as universal benchmarks.

## Fixtures

Generate deterministic mixed directories containing 1,000, 10,000, and 100,000 entries, plus deep-tree and native-path edge cases:

```bash
./scripts/generate-fixture.sh target/fixtures --clean
```

Omit `--clean` to fill in missing fixture entries without deleting existing ones. On the baseline machine, clean generation of all fixtures takes approximately one second.

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
- Per-batch UI append duration
- Cancelled directory request IDs

Capture sampled RSS and proportional set size (PSS) with:

```bash
STRATA_BINARY=target/release/strata \
  ./scripts/profile-fixture.sh target/fixtures/100000
```

Close other Strata instances before profiling so GApplication does not forward the request to an existing process. PSS is included because RSS charges each process for shared GTK, graphics, and font pages in full.

## Initial baseline — 2026-08-29

Environment:

- AMD Ryzen 9 9950X3D, 60 GiB RAM
- Omarchy, Wayland/Hyprland
- GTK 4.22.4
- Rust 1.97.1
- Optimized `--release` build
- Warm filesystem cache

Each timing below is a single engineering sample. “First UI batch” includes process startup; the parenthesized value is the time spent appending that 128-entry batch.

| Fixture | Window presented | First provider batch | First UI batch | Complete enumeration | Peak RSS / PSS |
|---:|---:|---:|---:|---:|---:|
| 1,000 | 90 ms | 72 ms | 105 ms (2.5 ms) | 104 ms | 241.9 / 147.9 MiB |
| 10,000 | 89 ms | 67 ms | 103 ms (2.3 ms) | 287 ms | 247.4 / 153.2 MiB |
| 100,000 | 91 ms | 68 ms | 107 ms (2.3 ms) | 2,131 ms | 278.0 / 183.9 MiB |

### Findings

- Initial interaction is effectively independent of directory size because results arrive in bounded 128-entry batches.
- UI batch insertion is well below one 16.7 ms frame in these samples.
- Removing the UI’s duplicate `FileEntry` storage reduced the 100,000-entry sample from approximately 315 MiB RSS / 219 MiB PSS to 278 MiB RSS / 184 MiB PSS.
- The 100,000-entry memory result still exceeds the provisional 150 MiB target and remains an open optimization item. The current model retains one application entry and one GTK string object per result.
- Complete enumeration scales approximately linearly and remains asynchronous, but smooth scrolling must also be evaluated manually under sustained input.
- After globally stable incremental sorting was introduced, the 100,000-entry sample completed in 3,755 ms at 286.3 MiB RSS / 191.4 MiB PSS. Individual GTK insertion batches remained below 4 ms, but the application-side merge cost is a future optimization target.

## Regression budgets

Use these provisional guardrails:

- The first provider batch should remain below 100 ms on the target machine.
- A 100,000-entry directory must remain scrollable while enumeration continues.
- UI batch rendering should not routinely exceed one 16.7 ms frame.
- Navigation away from a loading fixture must cancel its request without stale rows appearing.
- Long-term peak memory remains targeted below 150 MiB for the 100,000-entry fixture.

Record hardware, build profile, GTK version, cache state, and notable environmental load whenever replacing the baseline table. Use multiple samples before treating small timing differences as regressions.
