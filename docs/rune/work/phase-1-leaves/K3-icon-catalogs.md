# K3 Icon catalogs

| | |
| --- | --- |
| Phase / lane | 1 · Look |
| Plan | §4 K3, `settings/themes/icons`, `.../catppuccin` |
| Depends on | W2 (the semantic `IconName` set) |
| Unblocks | C20d. Every component's real icons, with no code change. |
| Owns | `crates/strata-kit/src/icons/`, `crates/strata-kit/build.rs`, `crates/strata-kit/icons/` (sources) |
| Branch | `feat/<issue>-k3-icon-catalogs` |

## Goal

Generate the Lucide and Catppuccin catalogs as `rune::Icon` sets at build time, behind W2's
semantic names.

## Scope

- A build step, like `themes/opencode/tools/icons.py`, that turns the SVG sources into Rune
  icon catalogs. Check the icon licenses (Lucide ISC, Catppuccin MIT) against `deny.toml` and
  note them.
- Lucide is single-colour and accent-tinted. Catppuccin is multi-colour, drawn in its own
  colours by Rune's rasterizer, with no special path.
- Map every W2 `IconName` in both catalogs. A missing mapping fails the build.
- Expose the full catalogs by name for C20d's picker, and for Strata's file-name → icon
  mapping, which stays in Strata.

## Tests

- A build test that every `IconName` resolves in each catalog.
- PNG goldens of an icon sheet per catalog, light and dark.

## Done when

The gallery's W2 token scene draws real icons in both catalogs.
