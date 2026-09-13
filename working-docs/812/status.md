# Pipeline status

- issue: https://github.com/lgse/strata/issues/812
- staging_pr: https://github.com/lgse/strata/pull/957 (draft)
- staging_branch: fix/812-x11-wm-class
- folder: working-docs/812
- round: 1
- stage: round 1 QA complete
- review_verdict: approve-with-comments
- qa_verdict: pass-with-nits
- head_sha: 9553be04e418a0ecefff17cff44600b3305bfde6
- agent_id: bc-4e962a5b-dc53-558c-ab3a-d774e1adeb34
- recommended_branch: fix/812-x11-wm-class
- notes: round 1 QA complete on product SHA `9553be04`. Verdict `pass-with-nits` (no product non-nits). Mapped FM `WM_CLASS` is `io.github.lgse.Strata`/`io.github.lgse.Strata`. Portal FileChooser identity still split. E2E `test_startup_arguments.py` 2 passed after `APPLICATION_NAME` follows prgname. Docs tip at QA start was `3733e428`. Left draft. Did not squash, undraft, merge, file issues, post a cleanup comment, or send anything to Origin.

## History

- plan: complete (bc-d2f12994-d476-5779-b4ec-fcd89b474ffb, 2026-09-13)
- staging PR: opened https://github.com/lgse/strata/pull/957 (draft) (bc-fcac889a-01d8-5d06-9c6a-c9638c2cfc26, 2026-09-13)
- round 1 code: complete (bc-1d72a758-95b9-586c-99da-6499ca2fc811, 2026-09-13)
- round 1 review: approve-with-comments (bc-009b4098-4ea7-534b-b4d5-fbb908e67c94, 2026-09-13)
- round 1 QA: pass-with-nits (bc-4e962a5b-dc53-558c-ab3a-d774e1adeb34, 2026-09-13)

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

See `code-notes.md` for host compiler flags, counts, and live `xprop`. QA re-ran the Rust filter (**12 passed**) and the startup E2E file (**2 passed**) plus mapped-window `xprop` on private `:90`.
