# C6 LocationBar

| | |
| --- | --- |
| Phase / lane | 1 · Chrome |
| Plan | §4 C6, `browser/navigation/location-bar`, `.../completion` |
| Depends on | W1 (drop zones, matched text), W2 |
| Unblocks | X2 (crumb drops), M3 |
| Owns | `crates/strata-components/src/location_bar/`, `scenes/c6.rs` |
| Branch | `feat/<issue>-c6-location-bar` |

## Goal

Breadcrumbs that switch to a text entry with completion.

## Scope

- Crumbs from a path's segments (data). Overflow scrolls on `scroll_x` and keeps the last
  crumb in view.
- Each crumb has a hierarchy menu (rune-kit Menu, with its items as data from the host),
  opens in a new tab on middle-click, and is a W1 `DropZone`.
- Toggle to an entry: Ctrl+L, or a click on empty space. Escape goes back to crumbs.
- Completion: a PopupList fed by the host through `rune::Latest`. Stale results are dropped.
  Tab completes the common prefix, then cycles. Matches draw with W1's matched text.
- The host gets intents: navigate, open in new tab, open the menu, and drop.

## Tests

- Scripts per Behavior group: crumb click, overflow keeps the last crumb, middle-click,
  toggle and Escape, Tab completion, and a stale result dropped (two queries, slow first).
- A11y golden: crumbs read as a navigation list, and the entry is a combobox.
- Scenes: short path, overflowing path, entry with completion open, and a drop hover. All in
  light and dark.

## Done when

The location-bar and completion scripts pass headless.
