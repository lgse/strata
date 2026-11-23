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
project's own skills below. Keep `skills-lock.json` in version control. Add or
update skills with `npx skills add` and `npx skills update`, then commit the
lockfile. Do not vendor third-party skill files under `.agents/`.

The project's own skills are the exception: `.agents/skills/qa-*` and
`.agents/skills/pr-review` are written here and committed, each un-ignored in
`.gitignore` and linked from `.claude/skills/`. Use `qa-scenarios` to write
and manage QA scenarios, `qa-drive` to verify a change by driving the
application in QA mode, `qa-quality` to score a scenario's value, and
`pr-review` to merge `main` into a pull request, prune tautological tests,
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
  `./scripts/quality.sh fmt` and `./scripts/quality.sh clippy` (or
  `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`)
  on the final code before pushing. If fixes change that code, rerun the affected
  checks before the push. CI continues to run its existing lint and formatting checks.
- Use `mise run test [filter]` or `./scripts/test-headless.py [filter]` for
  native targeted Rust tests across the unified workspace, for example
  `mise run test services::operations::tests` or
  `./scripts/test-headless.py services::operations::tests`. Both run headless
  with `--workspace --exclude strata-gallery --all-targets --all-features`
  (`mise run test` also runs `strata-gallery`'s unit tests; `mise run test-rune`
  is a compatibility alias). The selected filter must match at least one test,
  including relevant views, callers, or preferences coverage.
- Run compositor, D-Bus, and windowed tests with `mise run test-platform [filter]`
  (for example `mise run test-platform window::` for changes to the host, shell,
  key routing, or QA session host, or `mise run test-platform p2::` for a
  `strata-platform` shim). Each test runs in its own private headless sway
  session and D-Bus.
- QA selections name scenarios, or `<scenario>/<run>` for one run, for
  example `./scripts/qa.sh copy-leaves-source` or
  `./scripts/qa.sh arrow-keys-move-focus/columns-arrows`; `qa.sh` rejects unknown
  names and fails a selection that runs no scenario. `scripts/qa.sh` is the
  canonical pinned-container GUI runner. `scripts/quality.sh` accepts only
  `all`, `fmt`, `clippy`, or `test`, plus `--jobs N` and `--refresh-timings`
  for the test phase, and does not forward test filters.
- Use full pinned `./scripts/quality.sh` phases and a full `./scripts/qa.sh`
  run when impact is broad or uncertain. Escalate to both for shared infrastructure,
  dependencies, build/CI/harness code, cross-cutting behavior, or uncertain coverage.
  Even after escalation, isolate individual failures with scoped tests rather
  than repeating the full run. Use `./scripts/quality.sh test` only when a full
  Rust run is justified; defer `fmt` and `clippy` to the pre-push checkpoint.
  Preserve pinned image provenance and the existing `target/quality-container`
  and `target/qa-container` caches.
- GUI and delegated checks must never use the desktop or an inherited session
  bus. Clear inherited display variables, use a private display/compositor and
  private D-Bus, and disable accessibility bridging. Stop if the isolated display
  or bus is unavailable; never fall back to the user's display. See
  `docs/testing.md`.
- Documentation-only changes need no GUI/build tests: review the complete diff,
  validate links and example filters against the scripts and existing tests, and
  run `git diff --check`. These checks do not bypass required CI checks.
- The explicit owner consent policy above applies to all validation requirements,
  not just local tests. A generic request to push does not by itself authorize
  skipping checks; a direct request to push without specified checks does.
- Unless explicitly overridden, fix failures before pushing rather than
  relying on CI. Keep tests portable across supported environments and avoid
  assertions that depend on platform-specific URI normalization or other
  incidental system behavior.

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

## Archived media-runtime patch kit

`packaging/media-runtime/` holds an archived, opt-in GStreamer/GTK patch kit from
the 1.x release series (`#779`). Strata's Rune build does not link GTK or
GStreamer and does not use those patches. Do not modify or claim release coverage
for `packaging/media-runtime/` without reviewing `packaging/media-runtime/README.md`.

## Issues and pull requests

- Automated agents must follow the same issue-first workflow and pull request template as human contributors; do not remove or bypass template sections.
- Use the bug report form for defects, the feature request form for enhancements, and a blank issue only when neither form fits.
- Bug reports must include the Strata version, installation method, environment, reproduction steps, expected behavior, and any available sanitized logs. Never ask reporters to upload a core dump because it may contain secrets or private document contents.
- Keep pull request descriptions concise: explain what changed and why, provide manual steps to exercise the feature or reproduce the fixed bug, state the expected result, and link the issue. Do not list automated checks that CI already runs.
- Agents do not produce PR visual evidence: they cannot upload attachments, so screenshots and recordings taken for a PR are wasted work. Write `N/A` in the template's Visual evidence section and describe the change in words. Never commit screenshots, recordings, one-off capture scripts, logs, review reports, or PR-specific test plans to the source tree; maintained README media and executable visual-regression baselines are the only exceptions.
- Keep lasting behavior and test guidance in maintained documentation; put task checklists, test results, and review history in the issue or PR instead.
- Pull request titles must pass `.github/workflows/pr-title.yml`; do not bypass or weaken the Conventional Commit title check.

## Workspace crates

- The repository is a single unified Cargo workspace (`cargo check --workspace --all-targets --all-features --locked`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`).
- Follow `docs/workspace.md` for the crate layering, which `cargo deny --workspace check` (`deny.toml`) enforces, and for how visibility, translations, and log targets work across crates.
- `gtk4`, `gdk4`, `gdk4-wayland`, `gdk4-x11`, `sourceview5`, `gstreamer`, and `gstreamer-app` are banned workspace-wide in `deny.toml`. Never reintroduce them.
- Test-only code that another crate's tests use is gated with
  `#[cfg(any(test, feature = "test-support"))]`; a `tests` module is never compiled
  outside tests. Run tests with `--all-features`.

## The Rune interface

`crates/strata-platform`, `crates/strata-kit`, `crates/strata-components`, and
`crates/strata-gallery` hold the Rune UI stack; `crates/strata-app` joins them to
Strata's shared crates (`strata-core`, `strata-preview`, `strata-fs`,
`strata-desktop`, `strata-qa`), and the root `strata` package (`src/main.rs`) is
its thin binary entrypoint.

### Crate rules

- `strata-platform` depends on `rune` only: never on `rune-kit`, `strata-kit`, or
  a shared crate. Its API carries no Strata names. Each shim is a module behind
  its own cargo feature (`mise run compile-features` verifies each feature in
  isolation), and its docs link the upstream Rune issue and say what Rune lacks.
- Components in `strata-components` take state by accessor
  (`fn(&S) -> &X`, `fn(&mut S) -> &mut X`), as `rune-kit`'s Table and Tree do.
  They own no I/O: listings, thumbnails, and search results arrive as data.
- Components read only `Kit` tokens and `strata-kit` extensions, never a theme by
  name, and never hardcoded colours.
- Components carry no user-facing text. Labels and accessible names arrive as
  data, so the thirteen catalogs under `data/locales/` stay the only source.
- Strata's file-manager behaviour stays out of `strata-components`. A component
  holds the interaction model and placement, not the policy.
- Shared crates (`strata-core`, `strata-preview`, `strata-fs`, `strata-desktop`)
  never depend on Rune or on a Rune-side crate.
- `strata-app` is the only crate that depends on both sides, and only the root
  `strata` binary depends on `strata-app`.
- In `strata-app`, a `Browser` observer only queues: it runs inside GLib's
  dispatch, while Rune holds the application state. The state changes in `poll`
  or in an update.
- Reach Rune's loop from another thread with `AppHandle::update_if`, never with a
  `Waker` alone: under a covered Wayland window a `Waker` call does not run `poll`.

### Rune

- Rune is pre-release and takes no pull requests. When Rune core lacks something,
  file an issue upstream with a headless repro, or add a thin shim in
  `strata-platform` that names the issue. Never vendor or patch Rune.
- Every Rune crate pins the same `dev` rev in the root `Cargo.toml` (along with
  its `[patch.crates-io]` entries). Move the rev on purpose in its own change, and
  update the Rust version in `mise.toml` and `tests/container/Dockerfile` with it.
- Rune's API reference is `crates/rune/README.md` and `crates/rune-kit/README.md`
  at the pinned rev. Read them there, not from memory.

### Tests and goldens

- Workspace Rust tests run headless on Rune's virtual clock and never use the
  desktop display or session bus. Run them with `mise run test [filter]` and
  confirm the filter matched at least one test.
- Compositor, D-Bus, and windowed tests run with `mise run test-platform [filter]`,
  each in a private headless sway session; `strata`'s `tests/window.rs` is among
  them. Follow `docs/testing.md`.
- `strata-gallery`'s integration tests (its PNG/accessibility goldens and scripts
  under `tests/golden/`) are not part of the default `mise run test` run; only its
  unit tests run there. Run the gallery's integration suite on demand with
  `mise run test-gallery [filter]`, and update goldens only deliberately with
  `mise run bless <page>` followed by a full visual review of every changed PNG.
- PNG goldens and screenshots cover the look. Do not assert layout geometry,
  spacing, or pixel placement.

## Test organization

- Do not place test implementations inline with production code.
- Put module unit tests in an adjacent test module, such as `crates/strata-fs/src/app/navigation/tests.rs`, and declare it from the implementation with `#[cfg(test)] mod tests;`.
- Use the top-level `tests/` directory for integration tests that exercise the crate through its public API.

### Test value

- Test observable behavior, not implementation echoes. Do not add tests whose
  only purpose is to repeat constants or setter assignments, match style text,
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

## Code health

CodeScene scores every file; `scripts/code-health.sh <files>` prints the score
and the function-level findings, and `scripts/code-health.sh --changed` reviews
the files changed against `origin/main`. It needs `CS_ACCESS_TOKEN`.

- A change must not lower the code health of the files it touches. Run the
  script on them before pushing and fix new findings in the same change.
- Put new behavior in a new submodule instead of growing a file past about
  1,500 lines. Keep functions under about 70 lines and a cyclomatic complexity
  under 9; extract handlers and builder stages into named functions.
- Pass more than four values through a context, options, or request struct
  instead of positional arguments.
- Name complex conditions as predicates, and keep nesting under four levels.
- Share one implementation between the list and icons modes and between
  parallel copy/move/restore paths instead of duplicating a function per mode.
- In tests, share fixtures and assertion helpers and use table-driven cases
  instead of repeating large assertion blocks.

## Saved preferences

- Follow `docs/preferences.md` when adding or changing application-wide settings.
  A setting's schema field and default live in `strata_desktop::settings`; its
  live application, Settings UI, and host/app wiring live in `crates/strata-app/src/settings/`
  and `crates/strata-app/src/settings_dialog/`.
- Apply preferences at startup before Settings opens and propagate changes live
  across open windows. Settings pages must only edit preferences, never initialize
  browser behavior. Preserve documented chooser and window-local exceptions.
- Extend the exhaustive saved-preferences fixture and behavioral coverage for
  startup before Settings opens, changes across windows, and relevant view rebuilds.
  Serialization-only tests are not sufficient.

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
  regressions with `mise run test i18n::`, plus affected behavioral tests as
  required by the validation policy above.

## Keyboard shortcuts

- Define every shortcut in `strata_desktop::keys` (`crates/strata-desktop/src/keys/`),
  and match key presses through `KeyPress::is(…)` or `KeyPress::digit_in(…)`.
  Never compare raw key codes or typed text to recognize a command.
- Missing key constants belong in `crates/strata-desktop/src/keys/key_constants.rs`.
- Never type a key label by hand. F1 rows, hints, tooltips, and menus render
  catalog bindings with `strata_desktop::keys::label`. Follow `docs/keyboard-shortcuts.md`.

## UI controls

- Reuse existing `strata-components` and `rune-kit` controls and `strata-kit` look
  tokens for dropdowns, buttons, toggles, and segmented controls before creating
  new ones. Match their hover, active, focus, disabled, and theme states.

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

- Bundled icons come from two sources, compiled into `strata-kit` (`crates/strata-kit/icons/README.md`):
  - **Lucide** for interface icons and generic file categories (documents, pictures, archives, keys, folders). Add new interface icons only from Lucide, keeping geometry intact.
  - **Catppuccin VSCode Icons** for code, language, and config file types, with filename and extension associations in `crates/strata-app/src/entry_icon/catppuccin.rs`.
- Namespace every bundled asset with `strata-` (`strata-ctp-` for Catppuccin) and keep the Lucide ISC and Catppuccin MIT attributions in `THIRD_PARTY_LICENSES.md` and `crates/strata-kit/icons/` current, including the upstream revision.
- Lucide icons are tinted with semantic theme tokens. Catppuccin icons keep their own palette by default (Mocha on dark backgrounds, Latte on light ones); the **File icon colors** preference switches them to theme colors. Route both behaviors through `strata-kit::icons`; never hardcode a flavor or bypass the preference.

## Theming

- Apply semantic `Kit` / `strata-kit::Look` tokens to every visual state of new interface elements, including icons, text, backgrounds, borders, focus rings, selections, hover/active states, menus, and dialogs.
- Never use static hex/RGB colors for themeable interface elements. Built-in, custom, and Omarchy themes, as well as live appearance and transparency changes (and kit-layer `strata-kit::overrides` tokens), must remain visually consistent and update live.
