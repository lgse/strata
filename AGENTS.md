# Agent Instructions

## Git workflow

- Never commit or push directly to `main`. Work from a GitHub issue and submit changes through a pull request.
- Name branches `<type>/<issue-number>-<short-kebab-description>`, for example `feat/6-sandbox-previews`. Use Conventional Commit types such as `feat`, `fix`, `docs`, `refactor`, `test`, `perf`, `build`, `ci`, and `chore`.
- Write commits and pull request titles in Conventional Commits format: `<type>(optional-scope): <imperative description>`.
- Keep commits focused. Use `!` and a `BREAKING CHANGE:` footer for breaking changes, and reference the issue in the pull request body.

## Pre-push checks

- Do not push until the full local CI suite passes: `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features`.
- Agents must never run GTK tests against the user's active Wayland or X11 display. Run the suite under a private Xvfb display with accessibility bridging disabled:

  ```bash
  xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
    GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
    cargo test --all-targets --all-features
  ```

  If `xvfb-run` is unavailable, use a non-root portable extraction of the distribution's Xvfb package or another isolated display server. Do not fall back to the active desktop display, and do not use a backend that causes GTK tests to skip because initialization failed.
- Fix failures before pushing rather than relying on CI for feedback. Keep tests portable across supported environments and avoid assertions that depend on platform-specific URI normalization or other incidental system behavior.

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
