# C23 Motion effects

| | |
| --- | --- |
| Phase / lane | 1 · Chrome |
| Plan | §4 C23, `operations/trash/animation`, `operations/delete/dissolve` |
| Depends on | W2 (animation switch) |
| Unblocks | M4 |
| Owns | `crates/strata-components/src/motion/`, `scenes/c23.rs` |
| Branch | `feat/<issue>-c23-motion-effects` |

## Goal

The two operation effects, built against motion keys, so that they work with any row and any
target once C1e and C8 publish their keys.

## Scope

- **Trash flight:** icons fly from a set of source keys to a target key with Rune's
  shared-element motion. Several icons are staggered, then the target pulses.
- **Dissolve:** fragments drawn as `canvas` shapes, or in a `rune-fx` compositor slot.
  Measure both and keep the cheaper one.
- Both are skipped when animations are off. The host plays them only when the operation
  finishes before its progress card would show. That timing is Strata's.

## Tests

- Headless scripts on the virtual clock: the flight lands and cleans up its overlay
  elements, and the dissolve removes the row at the end.
- Animations off skips straight to the end state.
- Scenes: mid-flight and mid-dissolve at fixed times (PNG).

## Done when

The gallery demo flies from a list row to a sidebar-like target. C8 and C1e only need to
publish keys.
