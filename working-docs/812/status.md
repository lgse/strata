# Pipeline status

- issue: https://github.com/lgse/strata/issues/812
- staging_pr: https://github.com/lgse/strata/pull/957 (draft)
- staging_branch: fix/812-x11-wm-class
- folder: working-docs/812
- round: 1
- stage: plan complete; ready for code
- review_verdict: n/a
- qa_verdict: n/a
- head_sha: 9737471da73a4f4b52640dfcc9e73ab03dd6b0bc
- agent_id: bc-fcac889a-01d8-5d06-9c6a-c9638c2cfc26
- recommended_branch: fix/812-x11-wm-class
- notes: staging PR https://github.com/lgse/strata/pull/957 (draft) on `fix/812-x11-wm-class` from latest `lgse/strata` `main`. Working-docs only; no product/runtime code. Closed fork PR wmfeht/strata#31. Did not assign the issue or change P-band labels. Did not send anything to Origin.

## History

- plan: complete (bc-d2f12994-d476-5779-b4ec-fcd89b474ffb, 2026-09-13)
- staging PR: opened https://github.com/lgse/strata/pull/957 (draft) (bc-fcac889a-01d8-5d06-9c6a-c9638c2cfc26, 2026-09-13)
- round 1 code: pending

## Pick rationale

Highest remaining unassigned `bug` by P-band after skip rules. P0 `#854` assigned. All open P1 bugs assigned and/or already have fixing PRs. Unassigned P2 `#597` skipped: List leftover fixed on main (`#786`/`#771`); Columns leftover shipped (`#795`/`#797`); issue still open but not a new pipeline. `#537` / `#851` skipped as in-flight. Next band is P3; lowest unassigned P3 bug with no closing PR is `#812` (ahead of `#857`). `#954` has no P-band and was not ranked above P3. Hint file `internal/pipeline-next-bug.md` still named `#537` and no longer matches pick rules.

## Code-stage validation (when implementing)

Bounded identity change in `src/main.rs` plus `gdk4-x11` and maybe one E2E harness constant. Targeted:

    ./scripts/test-headless.py desktop_startup_wmclass application_identity x11_program_class portal::window_geometry::tests

Rename the filter to the actual test names after they exist. It must collect a nonzero set (new `src/tests.rs` cases plus existing portal centering). If AT-SPI name changes, also:

    ./scripts/e2e.sh tests/e2e/scenarios/test_startup_arguments.py

Pre-push (later): `./scripts/quality.sh fmt` and `./scripts/quality.sh clippy`. Full `quality.sh` / full `e2e.sh` only if the harness change proves broader than one constant. Record live `xprop` from case 4 in `code-notes.md`.
