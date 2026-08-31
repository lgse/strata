# Agent Instructions

## Git workflow

- Never commit or push directly to `main`. Work from a GitHub issue and submit changes through a pull request.
- Name branches `<type>/<issue-number>-<short-kebab-description>`, for example `feat/6-sandbox-previews`. Use Conventional Commit types such as `feat`, `fix`, `docs`, `refactor`, `test`, `perf`, `build`, `ci`, and `chore`.
- Write commits and pull request titles in Conventional Commits format: `<type>(optional-scope): <imperative description>`.
- Keep commits focused. Use `!` and a `BREAKING CHANGE:` footer for breaking changes, and reference the issue in the pull request body.

## Issues and pull requests

- Automated agents must follow the same issue-first workflow and pull request template as human contributors; do not remove or bypass template sections.
- Use the bug report form for defects, the feature request form for enhancements, and a blank issue only when neither form fits.
- Bug reports must include the Strata version, installation method, environment, reproduction steps, expected behavior, and any available sanitized logs. Never ask reporters to upload a core dump because it may contain secrets or private document contents.
- Keep pull request descriptions concise: explain what changed and why, provide manual steps to exercise the feature or reproduce the fixed bug, state the expected result, and link the issue. Do not list automated checks that CI already runs.
- Attach before/after screenshots or a short video for user-visible changes. Write `N/A` with a brief reason for non-visual changes.
- Pull request titles must pass `.github/workflows/pr-title.yml`; do not bypass or weaken the Conventional Commit title check.

## Test organization

- Do not place test implementations inline with production code.
- Put module unit tests in an adjacent test module, such as `src/app/navigation/tests.rs`, and declare it from the implementation with `#[cfg(test)] mod tests;`.
- Use the top-level `tests/` directory for integration tests that exercise the crate through its public API.

## Icons

- Add new interface icons only from the Lucide icon set.
- Keep Lucide geometry intact, namespace bundled assets with `strata-`, and preserve the ISC attribution in `THIRD_PARTY_LICENSES.md`.
- Render theme-colored bundled icons through `assets::primary_icon` / `assets::set_primary_icon`; direct icon-theme loading preserves the SVG's fallback color and will not follow live theme changes.

## Theming

- Apply semantic `@theme_*` colors to every visual state of new interface elements, including icons, text, backgrounds, borders, focus rings, selections, hover/active states, menus, and dialogs.
- Never use static hex/RGB colors for themeable interface elements. Built-in, custom, and Omarchy themes must remain visually consistent and update live.
