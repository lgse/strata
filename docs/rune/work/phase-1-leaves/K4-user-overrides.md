# K4 User overrides (and K5)

| | |
| --- | --- |
| Phase / lane | 1 · Look |
| Plan | §4 K4, K5, decision 3, §10 question 2 |
| Depends on | W2 (registry and slots). It builds against fallback values and needs no K1 or K2 code. |
| Unblocks | X1 |
| Owns | `crates/strata-kit/src/overrides/` |
| Branch | `feat/<issue>-k4-user-overrides` |

## Goal

Replace the GTK user stylesheet with `$XDG_CONFIG_HOME/strata/overrides.toml`, and migrate
what CSS can be migrated.

## Scope

- Parse TOML that sets any token in W2's registry by dotted name. It can scope a value to
  `light` or `dark`, or to a component slot (`[list.row]`, `[sidebar.row]`).
- Apply overrides after the theme, Omarchy, roundness and density, as the last step in
  building the `Look`.
- Errors: every unknown token or bad value, with its line. Rows that parse still apply.
- Live reload: the API takes the file's bytes, and the app watches the file, so this crate
  does no I/O. A reload goes through the same path as a theme switch.
- **CSS migration**, as a pure function from CSS text to (TOML text, unconverted rules). It
  converts plain colour, radius and padding declarations on Strata's documented style
  classes. Writing the file and the one-time notice stay in Strata.
- **K5:** a migration note for the dropped "interface renderer" and uncomposited-popup
  settings, for Strata's changelog.
- The settings row (open, error count, Reset) is a C20c row in Strata. Here, it is only the
  error count API.
- Ask the owner about §10 question 2 (icon overrides) before adding anything for it.

## Tests

Unit tests: a scoped override, an unknown token, a bad value, partial apply, and a CSS
migration fixture with converted and unconverted rules.

## Done when

The tests pass. The gallery shows a scene with an override applied. The token names in error
messages match the registry.
