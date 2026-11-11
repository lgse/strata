# C9 PreviewDrawer

| | |
| --- | --- |
| Phase / lane | 1 · Preview |
| Plan | §4 C9, `preview/preview-panel`, `.../layout` |
| Depends on | W2 |
| Unblocks | Hosts C10–C14. C2 reserves its slot. M5. |
| Owns | `crates/strata-components/src/preview_drawer/`, `scenes/c9.rs` |
| Branch | `feat/<issue>-c9-preview-drawer` |

## Goal

The resizable drawer that hosts every preview, in the browser and in the chooser.

## Scope

- SplitView's inspector pane, or a PaneState-driven column. Pick one and say why in the
  module docs. The width is held per window.
- A header with the Rendered/Source switch, Find, Open, Wrap and Close. Each control is shown
  only when the hosted view says it supports it, through a small trait that C10–C14 implement.
- The narrow-window rules, and the reserved slot when C2 Columns is active. Take both rules
  from the layout node's bullets.
- The content area is a slot. Gallery scenes use a placeholder view.

## Tests

Scripts: resize, close and reopen keeps the width, the narrow window collapses per the rules,
and keyboard focus moves into the drawer and back out. A11y golden: a region with a name, and
the header controls named.

## Done when

The preview-panel layout bullets pass headless.
