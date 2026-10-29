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

That installs them into `.agents/skills/`, which is gitignored apart from the
project's own skills below. Keep
`skills-lock.json` in version control. Add or update skills with `npx skills add`
and `npx skills update`, then commit the lockfile. Do not vendor third-party
skill files under `.agents/`.

The project's own skills are the exception: `.agents/skills/qa-*` and
`.agents/skills/pr-review` are written here and committed, each un-ignored in
`.gitignore` and linked from `.claude/skills/`. Use `qa-scenarios` to write
and manage QA scenarios, `qa-drive` to verify a change by driving the
application in QA mode, `qa-quality` to score a scenario's value, and
`pr-review` to merge main into a pull request, prune tautological tests,
review and fix it, and watch CI until it is green.

## Git workflow

- **One conversation, one PR by default.** Treat follow-up requests in the same conversation as changes to the existing task and PR unless the owner explicitly requests a separate PR. Keep implementing, committing, and pushing to that PR's branch; do not create stacked PRs or split follow-ups onto separate task branches. If isolation requires a temporary worktree, integrate its changes into the existing PR rather than opening another one.
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

- **Scoped tests are the default, including after failures.** When a run has an
  isolated failure, reproduce that exact test first. After fixing it, rerun that
  test and the affected callers/regressions only. Do not rerun the entire Rust or
  QA suite to diagnose one failure, recover a green summary, or satisfy a generic
  review/push checklist. Preserve earlier results and report the scoped rerun.
  A full rerun requires concrete evidence of broader impact under the escalation
  rule below or an explicit owner request; state that reason before launching it.
  If the owner directs scoped validation, do not expand it without renewed consent.
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
- QA selections name scenarios, or `<scenario>/<run>` for one run, for
  example `./scripts/qa.sh copy-leaves-source` or
  `./scripts/qa.sh arrow-keys-move-focus/columns-arrows`; `qa.sh` rejects unknown
  names and fails a selection that runs no scenario. `scripts/qa.sh` is the
  canonical pinned-container GUI runner. `scripts/quality.sh` accepts only
  `all`, `fmt`, `clippy`, or `test` and does not forward test filters.
- Use full pinned `./scripts/quality.sh` phases and a full `./scripts/qa.sh`
  run when impact is broad or uncertain. Escalate to both for shared infrastructure, dependencies,
  build/CI/harness code, cross-cutting behavior, or uncertain coverage.
  Even after escalation, isolate individual failures with scoped tests rather
  than repeating the full run. Use `./scripts/quality.sh test` only when a full
  Rust run is justified; defer `fmt` and `clippy` to the pre-push checkpoint.
  Preserve pinned image provenance and the existing `target/quality-container`
  and `target/qa-container` caches.
- GUI and delegated checks must never use the desktop or an inherited session
  bus. Use private Xvfb and private D-Bus, clear inherited display variables,
  disable accessibility bridging, and require GTK tests to run (not silently
  skip). Stop if the isolated display or bus is unavailable; never fall back to
  the user's display. See `docs/testing.md`.
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

## QA scenarios

- Run the `--qa` scenarios only through `./scripts/qa.sh`, and drive the app by
  hand only through `./scripts/qa-drive.sh`, both inside the pinned container.
  `tests/qa.rs` refuses to start without `STRATA_QA_CONTAINER=1`.
- A host run (`STRATA_QA_HOST=1`) needs explicit owner consent for that task.
  Record it and the skipped container run in the handoff; it is never
  validation evidence. Do not wire the QA runner into
  `scripts/test-headless.py`. See `docs/testing.md`.
- `./scripts/qa.sh`, `./scripts/qa-drive.sh`, and `./scripts/qa-mutation.sh`
  share a build cache; run them one after another, never concurrently.
- A behaviour change updates the QA scenarios that cover it in the same change.
  When a scenario with a `mutation` changes, or its patch in
  `tests/qa/mutations/` stops applying, check it with
  `./scripts/qa-mutation.sh <scenario>`.
- Regenerate visual baselines only deliberately, with
  `STRATA_QA_UPDATE_BASELINES=1 ./scripts/qa.sh <baseline scenario>`, and
  review the images before committing them.

## Test image reuse

- `./scripts/qa.sh` and `./scripts/quality.sh` verify and reuse the local pinned
  base built from `tests/container/`, or pull the published base once if
  missing. They must not rebuild images or fetch distribution packages during
  ordinary test runs.
- Prefer rootless Podman and keep the task's isolated configuration, storage, and
  runtime directories across commands. Do not create a fresh container store for
  every invocation, prune other sessions' images, or bypass image-input checks.
- Only when intentionally changing environment inputs, or when an unpublished
  environment must be bootstrapped, run explicitly:
  `STRATA_CONTAINER_ENGINE=podman python3 scripts/image_base.py build`.
  Run this once, then return to `./scripts/qa.sh`. Do not habitually rebuild bases
  as a pre-test step. Preserve `target/qa-container` so Cargo can reuse its cache.
- Published bases are updated by the trusted-main publisher when environment
  inputs change. Application edits do not require rebuilding the base. Different
  local UID/GID values are handled by generated container account files.
- Test failures, incomplete coverage, and invalid image provenance fail CI.
  See `docs/testing.md`.

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
- Agents do not produce PR visual evidence: they cannot upload attachments, so screenshots and recordings taken for a PR are wasted work. Write `N/A` in the template's Visual evidence section and describe the change in words. Never commit screenshots, recordings, one-off capture scripts, logs, review reports, or PR-specific test plans to the source tree; maintained README media and executable visual-regression baselines are the only exceptions.
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
- Never write layout tests. Do not add assertions for widget geometry, alignment,
  spacing, dimensions, wrapping, responsive arrangement, or pixel placement,
  including tests framed as clipping or layout regressions. Verify layout changes
  manually with screenshots instead.
- Keep behavioral tests for actions, input routing, lifecycle, filesystem safety,
  error handling, and live preferences. Do not turn visual adjustments into
  widget-tree or layout assertions.
- Before adding a test, identify the existing coverage owner. Extend a matching
  setup or use table-driven inputs instead of duplicating default/round-trip
  assertions or adding another QA smoke launch. Preserve separate cases where
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

## Internationalization

- Every new or changed application-owned, user-facing string must be localized
  with `rust-i18n` and added or updated in **all thirteen supported languages in the
  same change**: English, French, Hungarian, German, Spanish, Japanese, Brazilian
  Portuguese, Korean, Vietnamese, Italian, Russian, Simplified Chinese, and
  Traditional Chinese. This includes labels, buttons, menus, placeholders, tooltips,
  accessibility names/descriptions, dialogs, help text, errors, notifications,
  and status/progress messages.
- Follow `docs/internationalization.md` and update the matching catalogs under
  `data/locales/`. English fallback is a runtime safeguard, not permission to leave
  translations missing or copy English into other languages as a placeholder.
- Translate complete messages with named placeholders and the appropriate plural
  forms. Preserve placeholder names, markup, paths, commands, and keybindings;
  do not assemble sentences from English-only fragments or plural suffixes.
- Keep translation at presentation boundaries. Never translate filesystem paths,
  filenames, protocol/action identifiers, user-authored content, or labels and
  values supplied by another application.
- When changing UI text or catalogs, run the catalog coverage and interpolation
  regressions with `./scripts/test-headless.py i18n::`, plus affected behavioral
  tests as required by the validation policy above.

## Keyboard shortcuts

- Read key input only through `ui::keys` (`keys::on_press`, `keys::on_release`,
  `keys::read_event`); `clippy.toml` rejects raw key signals elsewhere.
- Define every shortcut as a constant in the `ui::keys::bindings` catalog and
  match it with `press.is(…)` or `press.digit_in(…)`. Never compare raw keyvals or
  typed text to recognize a command.
- Never type a key label by hand. F1 rows, hints, tooltips, and menus render
  catalog bindings with `keys::label`. Follow `docs/keyboard-shortcuts.md`.

## UI controls

- Reuse existing control builders, shared bindings, and CSS classes for dropdowns, buttons, toggles, and segmented controls before creating new ones. Match their hover, active, focus, disabled, and theme states; do not style a new control as a generic form field when an existing button style applies.

## Tooltips

- Only icon-only buttons may have tooltips. Do not add them to text buttons,
  menu items, labels, entries, file rows, status indicators, or other widgets,
  even when their text is truncated.
- For controls that switch between icon-only and labelled modes, enable tooltips
  only in icon-only mode and clear them when the label is shown.
- Keep important help, errors, and status information visible; use explicit
  accessible names/descriptions for assistive technology. Never use tooltip
  text as internal data for filtering, identification, or application behavior.

## Comments

- Prefer self-explanatory names and structure. Do not add comments that narrate obvious code or restate a test's setup, actions, or assertions.
- Use concise comments for non-obvious intent, invariants, safety requirements, external constraints, workarounds, or surprising behavior.

## Icons

- Bundled icons come from two sources, each with its own job:
  - **Lucide** for interface icons and generic file categories (documents, pictures, archives, keys, folders). Add new interface icons only from Lucide, keep its geometry intact, and store them under `data/icons/scalable/actions/`.
  - **Catppuccin VSCode Icons** for code, language, and config file types. Store them under `data/icons/catppuccin/`, copied unchanged from upstream's `icons/css-variables/` set, and add their filename and extension associations to the generated `src/ui/browser/entry/catppuccin.rs` by rerunning `scripts/catppuccin_icons.py <upstream checkout>` rather than editing it by hand.
- Namespace every bundled asset with `strata-` (`strata-ctp-` for Catppuccin) and keep the Lucide ISC and Catppuccin MIT attributions in `THIRD_PARTY_LICENSES.md` current, including the upstream revision.
- Render theme-colored bundled icons through `assets::primary_icon` / `assets::set_primary_icon`; direct icon-theme loading preserves the SVG's fallback color and will not follow live theme changes.
- Lucide icons are tinted with the theme accent. Catppuccin icons keep their own palette by default, resolved at render time to Mocha on dark backgrounds and Latte on light ones; the **File icon colors** preference switches them to the accent. Route both behaviors through `assets`; never hardcode a flavor or bypass the preference.

## Theming

- Apply semantic `@strata_*` colors to every visual state of new interface elements, including icons, text, backgrounds, borders, focus rings, selections, hover/active states, menus, and dialogs.
- Never use static hex/RGB colors for themeable interface elements. Built-in, custom, and Omarchy themes must remain visually consistent and update live.
