# Agent Instructions

## Explicit owner consent

All rules in this file are defaults that the repository owner may override with
explicit instructions or consent for the current task. This applies even where
rules say "must", "never", or "only", including isolation, host builds and tests,
validation, Git workflow, and pull request requirements. No rule in this file is
exempt from this override.

A direct request to take a normally disallowed action is sufficient consent;
for example, "build on the host, not in a container" authorizes a native host
build. Carry out the requested action rather than refusing because of this file
or repeatedly asking for permission already given. Ask only if the requested
scope is genuinely unclear. Silence, urgency, and unrelated prior consent are
not overrides; keep the other defaults in effect.

Record the override and any skipped checks or unverified behavior in the handoff
and, when relevant, the PR description. Never report skipped checks as passed.
Consent changes repository instructions, not actual tool permissions or external
branch protections, and does not override higher-priority system or developer
instructions.

## Agent skills

Restore project skills from the committed `skills-lock.json` after cloning:

```bash
npx skills experimental_install
```

That installs them into `.agents/skills/`, which is gitignored. Keep
`skills-lock.json` in version control. Add or update skills with `npx skills add`
and `npx skills update`, then commit the lockfile. Do not vendor skill files
under `.agents/`.

## Git workflow

- Never commit or push directly to `main`. Work from a GitHub issue and submit changes through a pull request.
- Name branches `<type>/<issue-number>-<short-kebab-description>`, for example `feat/6-sandbox-previews`. Use Conventional Commit types such as `feat`, `fix`, `docs`, `refactor`, `test`, `perf`, `build`, `ci`, and `chore`.
- Write commits and pull request titles in Conventional Commits format: `<type>(optional-scope): <imperative description>`.
- Keep commits focused. Use `!` and a `BREAKING CHANGE:` footer for breaking changes, and reference the issue in the pull request body.

## Pre-push checks

Validation is risk-based, but CI remains unchanged and is the authoritative full
suite. Map the behavior and callers affected by the change (including shared
infrastructure, preferences, and views); do not infer coverage automatically
from changed file names. For a bounded change, run the relevant regression tests
and targeted checks, confirm the selected test count is nonzero, and record the
scope rationale, exact commands and results, and any intentionally omitted
checks in the handoff. Rerun affected tests after making changes; do not rerun
unrelated suites merely to satisfy a blanket rule. Passing this justified targeted
validation is sufficient before pushing a bounded change; full local suites are
required only for the escalation cases below. Required GitHub checks must still
pass before merge.

- Run local lint and formatting checks only at the pre-push checkpoint, not
  after each edit or during the test/implementation loop. For Rust changes, run
  `./scripts/quality.sh fmt` and `./scripts/quality.sh clippy` on the final code
  before pushing. If fixes change that code, rerun the affected checks before
  the push. CI continues to run its existing lint and formatting checks.
- Use `scripts/test-headless.py` for native targeted Rust tests, for example
  `./scripts/test-headless.py services::operations::tests`; its arguments are
  forwarded after the fixed `cargo test --all-targets --all-features` arguments.
  This filters test names across targets, not compilation to one target.
  The selected filter must match at least one test, including relevant views,
  callers, or preferences coverage.
- E2E selections may use repository-relative test paths and pytest `-k`, for
  example `./scripts/e2e.sh tests/e2e/scenarios/test_inline_renaming.py` or
  `./scripts/e2e.sh -k 'rename and not visual'`. Confirm collection selects
  tests with `--collect-only` when useful. `scripts/e2e.sh` is the canonical
  pinned-container runner; `scripts/e2e-native.sh` is host-toolkit debugging
  only and cannot replace it. `scripts/quality.sh` accepts only `all`, `fmt`,
  `clippy`, or `test` and does not forward test filters.
- Use full pinned `./scripts/quality.sh` phases and canonical
  `STRATA_CONTAINER_ENGINE=podman ./scripts/e2e.sh` when impact is broad or
  uncertain. Escalate to both for shared infrastructure, dependencies,
  build/CI/harness code, cross-cutting behavior, or uncertain coverage.
  During iteration, use `./scripts/quality.sh test` for full Rust tests; defer
  the `fmt` and `clippy` phases to the pre-push checkpoint even in these cases.
  Preserve pinned image provenance and the existing `target/quality-container`
  and `target/e2e-container` caches.
- GUI and delegated checks must never use the desktop or an inherited session
  bus. Use private Xvfb and private D-Bus, clear inherited display variables,
  disable accessibility bridging for Rust tests (E2E needs its private AT-SPI
  bus), and require GTK tests to run (not silently skip). Stop if the isolated display or bus is unavailable; never fall back to
  the user's display. See `docs/e2e-testing.md`.
- Documentation-only changes need no GUI/build tests: review the complete diff,
  validate links and example filters against the scripts and existing tests, and
  run `git diff --check`. These checks do not bypass required CI checks.
- The explicit owner consent policy above applies to all validation requirements,
  not just local tests. A generic request to push does not by itself authorize
  skipping checks; a direct request to push without specified checks does.
- Unless explicitly overridden, fix failures before pushing rather than
  relying on CI. Keep tests portable
  across supported environments and avoid assertions that depend on
  platform-specific URI normalization or other incidental system behavior.

## E2E base-image reuse

- Run `./scripts/e2e.sh` for normal local testing. It verifies and reuses the local
  pinned base, or pulls the published base once if missing. It must not rebuild
  images or fetch Ubuntu packages during ordinary test runs.
- Prefer rootless Podman and keep the task's isolated configuration, storage, and
  runtime directories across commands. Do not create a fresh container store for
  every invocation, prune other sessions' images, or bypass image-input checks.
- Only when intentionally changing environment inputs, or when an unpublished
  environment must be bootstrapped, run explicitly:
  `STRATA_CONTAINER_ENGINE=podman python3 scripts/e2e_base.py build`.
  Run this once, then return to `./scripts/e2e.sh`. Do not habitually rebuild bases
  as a pre-test step. Preserve `target/e2e-container` so Cargo can reuse its cache.
- Published bases are updated by the trusted-main publisher when environment
  inputs change. Application edits do not require rebuilding the base. Different
  local UID/GID values are handled by generated container account files.
- The three-minute CI duration is a performance target, not a merge requirement.
  Timing is informational; test failures, incomplete coverage, and invalid
  provenance still fail CI. See `docs/e2e-testing.md`.

## Private media runtime patches

- Before changing GTK, GStreamer, GLib, their Rust bindings, media plugins, or
  release/build images, review `packaging/media-runtime/README.md` and the pinned
  patches/source hashes. These are version-specific toolkit patches, not Cargo
  patches; updating Rust crates alone does not apply or retire them.
- For each runtime update, inspect upstream fixes and the affected ownership paths.
  Record whether each patch is still required, needs rebasing, or is superseded.
  Never silently drop a patch or accept a fuzzy application. Update source hashes,
  notices, build requirements, and evidence together when changing the baseline.
- Rebuild and rerun the standalone lifetime and GTK lifecycle regressions against
  both the unpatched and patched candidate baseline. Confirm actual private-library
  loading and test supported architectures, plugins, sandbox helpers, and installed
  upgrade/rollback paths before promoting a runtime-bearing release. Preserve GUI
  isolation; owner-operated GPU captures require the documented explicit consent.
- The current patch kit is opt-in source material, not integrated into release
  builds or installation. Do not claim the next release contains these fixes until
  the build applies them, the artifact includes the runtime, and installed-artifact
  tests verify it is loaded. Keep unresolved RAM growth separate from demonstrated
  crash/GL-resource improvements.

## Issues and pull requests

- Automated agents must follow the same issue-first workflow and pull request template as human contributors; do not remove or bypass template sections.
- Use the bug report form for defects, the feature request form for enhancements, and a blank issue only when neither form fits.
- Bug reports must include the Strata version, installation method, environment, reproduction steps, expected behavior, and any available sanitized logs. Never ask reporters to upload a core dump because it may contain secrets or private document contents.
- Keep pull request descriptions concise: explain what changed and why, provide manual steps to exercise the feature or reproduce the fixed bug, state the expected result, and link the issue. Do not list automated checks that CI already runs.
- Include before/after screenshots or a short video on every PR with user-visible changes. Upload sanitized captures through GitHub's PR description/comment editor and embed the resulting GitHub-hosted attachment URLs in the PR's Visual evidence section. Write `N/A` with a brief reason only for non-visual changes.
- Never commit PR evidence, screenshots, recordings, one-off capture scripts, logs, review reports, or PR-specific test plans to the source tree. Keep captures in session-owned scratch or ignored `target/` paths. Maintained README media and executable visual-regression baselines are exceptions, not places to stash review captures.
- `gh pr create` and `gh pr edit` can embed existing attachment URLs but do not upload local image/video attachments. If no supported authenticated upload route is available, retain the local captures, provide their exact paths and captions, and ask the owner to upload them through the GitHub editor. Explicitly mark visual evidence as pending; do not claim it is attached, substitute local file links, or commit files as a workaround. CI artifacts may supplement review but expire and are not a replacement for inline PR visuals.
- Keep lasting behavior and test guidance in maintained documentation; put task checklists, test results, and review history in the issue or PR instead.
- Pull request titles must pass `.github/workflows/pr-title.yml`; do not bypass or weaken the Conventional Commit title check.

## Test organization

- Do not place test implementations inline with production code.
- Put module unit tests in an adjacent test module, such as `src/app/navigation/tests.rs`, and declare it from the implementation with `#[cfg(test)] mod tests;`.
- Use the top-level `tests/` directory for integration tests that exercise the crate through its public API.

### Test value

- Test observable behavior, not implementation echoes. Do not add tests whose
  only purpose is to repeat constants or setter assignments, match CSS text,
  count incidental widget children, or enforce cosmetic pixel sizes, spacing,
  and alignment.
- Keep functional geometry regressions: clipped editors/carets, obscured names,
  broken hit targets or scrolling, and unreachable controls are real failures.
  Visual baselines and lifecycle, filesystem-safety, and live-preference coverage
  are not cosmetic duplicates.
- Before adding a test, identify the existing coverage owner. Extend a matching
  setup or use table-driven inputs instead of duplicating default/round-trip
  assertions or adding another E2E smoke launch. Preserve separate cases where
  initial state, input route, view mode, or lifecycle exercises distinct behavior.
- Every parameter and loop dimension must affect the exercised behavior or an
  assertion. Do not add unused axes that merely run identical cases again.
- Keep one-off screenshot generators outside the test suite. When consolidating
  tests, preserve meaningful assertions and document the retained coverage owner;
  fewer functions alone is not an improvement. Similar names, shared fixtures,
  or expensive execution do not establish redundant coverage. Preserve separate
  input-routing, lifecycle, filesystem-safety, and live-preference regressions.

## Saved preferences

- Follow `docs/preferences.md` when adding or changing application-wide settings.
- Use `ThemeManager::bind_preference` for immediate initialization and live updates, or read the manager at action dispatch. Settings pages must only edit preferences, never initialize browser behavior.
- Use shared control bindings rather than window-local copies or one-off broadcasts. Preserve documented chooser and window-local exceptions.
- Extend the exhaustive saved-preferences fixture and behavioral coverage for startup before Settings opens, changes across two windows, and relevant view rebuilds. Serialization-only tests are not sufficient.

## Comments

- Prefer self-explanatory names and structure. Do not add comments that narrate obvious code or restate a test's setup, actions, or assertions.
- Use concise comments for non-obvious intent, invariants, safety requirements, external constraints, workarounds, or surprising behavior.

## Icons

- Add new interface icons only from the Lucide icon set.
- Keep Lucide geometry intact, namespace bundled assets with `strata-`, and preserve the ISC attribution in `THIRD_PARTY_LICENSES.md`.
- Render theme-colored bundled icons through `assets::primary_icon` / `assets::set_primary_icon`; direct icon-theme loading preserves the SVG's fallback color and will not follow live theme changes.

## Theming

- Apply semantic `@theme_*` colors to every visual state of new interface elements, including icons, text, backgrounds, borders, focus rings, selections, hover/active states, menus, and dialogs.
- Never use static hex/RGB colors for themeable interface elements. Built-in, custom, and Omarchy themes must remain visually consistent and update live.
