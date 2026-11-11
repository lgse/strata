# S5 Rune issues, crate name and release workflow

| | |
| --- | --- |
| Phase / lane | 0 · Transfer and Desktop |
| Plan | §0, §7 phase 0, §8 (published crate on a moving base), §10 question 1 |
| Depends on | S1, S2, S3 for repros. The name and workflow parts can start at once. |
| Unblocks | M0. The issue link in every P-module's docs. |
| Owns | `docs/decisions/S5-upstream.md`, `.github/workflows/release.yml`, `crates/strata-platform/README.md`, `CHANGELOG.md` |
| Branch | `ci/<issue>-s5-upstream-and-release` |

## Goal

Ask Rune for what this repository would otherwise have to shim, and set up the publishing
side of `strata-platform`.

## Scope

- **Rune issues**, each with a headless repro, for:
  - Linux `drag_out`;
  - drops with arbitrary MIME types and a position, on Wayland;
  - clipboard custom MIME types and image writes;
  - Linux client-side decorations and resize edges;
  - foreign parenting;
  - Settings portal keys beyond `color-scheme`;
  - the double-click interval;
  - popup surfaces;
  - horizontal or grid virtualization (C3 packs tiles per row meanwhile);
  - installed-font enumeration, if `system_fonts` lacks it (K2).
  Record each link in the decision record. The P-module docs cite them.
- **Crate name:** settle plan §10 question 1, with the owner deciding. Avoid a `rune-`
  prefix. Hold the name on crates.io with an empty placeholder only if the owner agrees,
  since publishing is outward-facing.
- **Release workflow:** git tags named `strata-platform-v<semver>+rune.<short-rev>` (or an
  equivalent the owner picks), a changelog, and a README that says which Rune rev each tag
  supports and how to pin both (plan §8). Each Rune rev bump is a minor release.
- **Shim lifecycle:** a deprecation checklist for when Rune ships a feature: deprecate for
  one release, then remove in the next minor.

## Done when

M0: each of P1–P5 says "Rune shipped" or "shim here", with an issue link. The name is
decided. A dry-run tag builds a release in CI.
