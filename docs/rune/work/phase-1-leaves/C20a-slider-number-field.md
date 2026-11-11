# C20a Slider and NumberField

| | |
| --- | --- |
| Phase / lane | 1 · Controls |
| Plan | §4 C20 (Slider, NumberField) |
| Depends on | W2 |
| Unblocks | C20c, Strata's settings pages (density, text size, thumbnail size, workers, roundness) |
| Owns | `crates/strata-components/src/controls/slider/`, `.../number_field/`, `scenes/c20a.rs` |
| Branch | `feat/<issue>-c20a-slider-number-field` |

## Goal

The two numeric controls that rune-kit lacks.

## Scope

- **Slider:** a horizontal track with snap points (labelled or not), dragging, a click on the
  track, and keys through Rune's slider role (`on_step`, `on_step_limit`: arrows, Page Up and
  Down, Home and End). It reports while dragging and on commit.
- **NumberField:** a TextInput with steppers, min, max and step, a unit suffix such as `%`,
  wheel and arrow steps, and invalid text that reverts on blur. It takes a value accessor.
- Both read Kit tokens only, and suit rune-kit's Field layout.

## Tests

Scripts: drag snaps, keys step and reach the limits, typing an out-of-range number clamps or
reverts. A11y golden: the slider role with value, min and max, and the spin button role.
Scenes: each control at its limits and disabled, in light and dark.

## Done when

The tests pass. Consider proposing both controls to rune-kit as issues, and record whether
that was done.
