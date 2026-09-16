# Pipeline status

- issue: https://github.com/lgse/strata/issues/1045
- staging_pr: https://github.com/lgse/strata/pull/1055
- staging_branch: fix/1045-trusted-helper-paths
- folder: working-docs/1045
- round: 1
- stage: staging draft open; ready for code
- review_verdict: n/a
- qa_verdict: n/a
- head_sha: n/a (working-docs only)
- agent_id: bc-4766d826-7f2d-53dc-b627-f33ecdfdd8db
- recommended_branch: fix/1045-trusted-helper-paths
- notes: Staging draft only. No product/runtime code. Labels left `security`+`P1` on the issue; no P-band labels added to the PR. Assignee left `wmfeht`. Did not send to Origin. Fork head is `wmfeht/strata`; did not open a fork PR. Left draft. Implementation comes next. `working-docs/1045/` left for William.

## History

- plan: complete (bc-60f05de0-8e0c-55aa-9990-c5e5cb01a1fc, 2026-09-16)
- staging: https://github.com/lgse/strata/pull/1055 (bc-4766d826-7f2d-53dc-b627-f33ecdfdd8db, 2026-09-16)
- round 1 code: pending

## Code-stage validation (when implementing)

Bounded trusted-path module plus host exec call sites (`sandbox`, `update_install`, `settings` relaunch, `portal_setup`). Targeted:

```bash
./scripts/test-headless.py 'trusted_command::tests' 'sandbox::tests' 'services::update_install::tests' 'portal_setup::tests'
```

If relaunch construction is extracted into `ui::settings::tests`, add that filter. Confirm collection is nonzero (new resolver tests plus existing sandbox argv / checksum token tests). Do not run full `quality.sh` / `e2e.sh` for this spawn-path change. Pre-push later: `./scripts/quality.sh fmt` and `./scripts/quality.sh clippy`. Escalate to full suites only if `sandbox_command` argv or jail `PATH=/usr/bin` is accidentally rewritten.
