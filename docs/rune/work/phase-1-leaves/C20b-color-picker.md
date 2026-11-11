# C20b ColorPicker

| | |
| --- | --- |
| Phase / lane | 1 · Controls |
| Plan | §4 C20 (ColorPicker), `browser/folder-customization`, `settings/themes` |
| Depends on | W2 |
| Unblocks | Strata's folder colours and theme editor |
| Owns | `crates/strata-components/src/controls/color_picker/`, `scenes/c20b.rs` |
| Branch | `feat/<issue>-c20b-color-picker` |

## Goal

Replace GTK's `ColorDialogButton` with a picker that serves folder colours and the theme
editor.

## Scope

- A saturation and value plane, a hue strip and an alpha strip, drawn with `canvas` shapes or
  `Pixels`.
- A hex entry (with `#RGB`, `#RRGGBB` and `#RRGGBBAA`) that stays in sync.
- Swatches as data: theme colours and recent colours.
- A button that opens the picker in a rune-kit Popover, plus an inline mode.
- Keyboard: arrows move in the plane and the strips. Tab moves between parts.

## Tests

Unit: hex parsing and HSV round-trips. Scripts: drag in the plane, hex entry, swatch click
and keyboard moves. A11y golden: each part named, and values as text. Scenes: opaque,
translucent and the popover, in light and dark.

## Done when

The tests pass, and the colour value type converts to and from `rune::Color`.
