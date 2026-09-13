# Pipeline status

- issue: https://github.com/lgse/strata/issues/812
- staging_pr: https://github.com/lgse/strata/pull/957 (draft)
- staging_branch: fix/812-x11-wm-class
- folder: working-docs/812
- round: 1
- stage: round 1 review complete
- review_verdict: approve-with-comments
- qa_verdict: n/a
- head_sha: 9553be04e418a0ecefff17cff44600b3305bfde6
- agent_id: bc-009b4098-4ea7-534b-b4d5-fbb908e67c94
- recommended_branch: fix/812-x11-wm-class
- notes: round 1 review complete on `fix/812-x11-wm-class`. Verdict `approve-with-comments` (nits: stale PR body; code-complete status listed `77be0a30`). No blockers. File-manager prgname and X11 program class are `io.github.lgse.Strata`. Portal FileChooser identity unchanged. E2E `APPLICATION_NAME` follows prgname. Left draft. Did not squash, undraft, drop working-docs, merge, or post a GitHub PR comment. Did not send anything to Origin.

## History

- plan: complete (bc-d2f12994-d476-5779-b4ec-fcd89b474ffb, 2026-09-13)
- staging PR: opened https://github.com/lgse/strata/pull/957 (draft) (bc-fcac889a-01d8-5d06-9c6a-c9638c2cfc26, 2026-09-13)
- round 1 code: complete (bc-1d72a758-95b9-586c-99da-6499ca2fc811, 2026-09-13)
- round 1 review: approve-with-comments (bc-009b4098-4ea7-534b-b4d5-fbb908e67c94, 2026-09-13)

## Pick rationale

Highest remaining unassigned `bug` by P-band after skip rules. P0 `#854` assigned. All open P1 bugs assigned and/or already have fixing PRs. Unassigned P2 `#597` skipped: List leftover fixed on main (`#786`/`#771`); Columns leftover shipped (`#795`/`#797`); issue still open but not a new pipeline. `#537` / `#851` skipped as in-flight. Next band is P3; lowest unassigned P3 bug with no closing PR is `#812` (ahead of `#857`). `#954` has no P-band and was not ranked above P3. Hint file `internal/pipeline-next-bug.md` still named `#537` and no longer matches pick rules.

## Code-stage validation (when implementing)

Bounded identity change in `src/main.rs` plus `gdk4-x11` and one E2E harness constant. Targeted:

    cargo fmt --all --check
    cargo clippy --all-targets --all-features -- -D warnings
    xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
      GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
      cargo test --all-targets --all-features -- \
      desktop_startup_wmclass application_identity x11_program_class portal::window_geometry::tests
    ./scripts/e2e.sh tests/e2e/scenarios/test_startup_arguments.py

See `code-notes.md` for host compiler flags, counts, and live `xprop`.
