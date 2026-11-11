# W2 Look contract

| | |
| --- | --- |
| Phase / lane | 0 · Look |
| Plan | §3 rules (components read only Kit and strata-kit tokens), K1–K4 |
| Depends on | W0 |
| Unblocks | Every component, K1–K4 |
| Owns | `crates/strata-kit/src/look.rs`, `tokens.rs`, `icons.rs` (the semantic set), `slots/` |
| Branch | `feat/<issue>-w2-look-contract` |

## Goal

Fix the surface that components read their look from, before any theme exists. Components
built against W2's fallback values must then look right when K1–K4 land, with no code change.
Only the goldens should move.

## Scope

- **`Look`:** a `rune_kit::Kit` plus Strata's extensions, built from `Look::fallback(dark)`.
  Components get it through an accessor, the same way they get state.
- **Extension tokens:**
  - frame and content density as spacing steps;
  - row height per view;
  - the tile size ladder (C3) and the thumbnail size steps;
  - a type ramp scaled by text size;
  - roundness as a factor on `Kit.radius`;
  - glow.
  K2 decides the values. W2 decides the names and types.
- **Semantic icons:** an `IconName` set covering what the components draw (folder, file,
  chevrons, eject, lock, pin, trash, close, search and so on), backed by a placeholder
  catalog. K3 replaces the catalog behind the same names. Components never name a catalog.
- **Component slots:** a `slots/<id>.rs` file per component that has one (`list.row`,
  `sidebar.row`, `tabs.tab`, …). Each holds that component's named tokens, with defaults
  taken from `Look`. Slots are API (plan K4). Each WP fills in its own file.
- **Token registry:** `tokens::get` and `tokens::set`, by dotted name, over the `Kit` tokens,
  the extensions and the slots, so K4 can apply overrides without knowing each component.
  Registering a slot is one line, in that slot's own file.
- **Animation input:** `Look` carries the "animations off" switch (from P5) as plain data, for
  the motion WPs.

## Out of scope

Theme files and their mapping (K1), real density and type values (K2), real icons (K3), and
reading override files (K4).

## Tests

- The registry round-trips every registered name, and an unknown name is an error.
- A gallery scene, `w2-tokens`, shows the fallback swatches, spacing and type ramp in light
  and dark, with a PNG golden.

## Done when

The K1, K2 and C1e owners review the token names. `strata_kit::fallback` is removed or
forwards to `Look::fallback`.
