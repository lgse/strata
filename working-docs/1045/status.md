# Pipeline status

- issue: https://github.com/lgse/strata/issues/1045
- staging_pr: https://github.com/lgse/strata/pull/1055
- staging_branch: fix/1045-trusted-helper-paths
- folder: working-docs/1045
- round: 1
- stage: round 1 code complete
- review_verdict: n/a
- qa_verdict: n/a
- head_sha: 1f2cf0d812d693c13982915efaa37bf23eedd0e4
- agent_id: bc-91ed4417-8d89-5141-91b4-addf8f99e9e0
- recommended_branch: fix/1045-trusted-helper-paths
- notes: Round 1 implementation on the existing draft. Host helpers resolve only under `/usr/bin`, `/usr/sbin`, `/bin`, `/sbin`. In-process SHA-256 for update archives. Jail-internal helpers unchanged. Left draft. Did not send to Origin. Did not squash. `working-docs/1045/` left in place.

## History

- plan: complete (bc-60f05de0-8e0c-55aa-9990-c5e5cb01a1fc, 2026-09-16)
- staging: https://github.com/lgse/strata/pull/1055 (bc-4766d826-7f2d-53dc-b627-f33ecdfdd8db, 2026-09-16)
- round 1 code: complete (bc-91ed4417-8d89-5141-91b4-addf8f99e9e0, 2026-09-16)

## Code-stage validation

Targeted (nonzero collection; 82 passed, 0 failed):

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  cargo test --all-targets --all-features trusted_command::tests
# plus sandbox::tests, services::update_install::tests, portal_setup::tests, restart_waiter, omarchy::tests
```

Did not run full `quality.sh test` or `e2e.sh`. Isolation argv and jail `PATH=/usr/bin` were not rewritten. `cargo-deny` / `typos` not installed locally.
