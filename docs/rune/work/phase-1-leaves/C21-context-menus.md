# C21 Context menus

| | |
| --- | --- |
| Phase / lane | 1 · Chrome |
| Plan | §4 C21, `browser/context-menu` (100 bullets) |
| Depends on | W1 (geometry for the keyboard anchor), W2 |
| Unblocks | M3 |
| Owns | `crates/strata-components/src/context_menu/`, `scenes/c21.rs` |
| Branch | `feat/<issue>-c21-context-menus` |

## Goal

The anchor and placement rules for Strata's menus, on rune-kit `context_menu`. Which items
appear is Strata's policy, and arrives as data.

## Scope

- Open from the pointer (secondary click), or from the keyboard (the Menu key, Shift+F10)
  at the cursor item's rect, taken through W1 `CollectionGeometry`.
- A background menu when the press is on empty space.
- Grouped sections with separators, a custom actions section, and Send to submenus. The
  menu model is a plain data type.
- The row-hover look from `Kit::interactive`.
- Placement that stays inside the window. Menus clip to the window until Rune has popup
  surfaces (plan §4). Note the chooser's minimum size.

## Tests

Scripts: open by pointer, Menu key and Shift+F10 at the cursor item, a background menu,
submenu keyboard navigation, and placement at window edges. A11y golden: menu roles, groups
and submenu state.

## Done when

The bullets that cover placement and keyboard opening pass. The rest are listed as Strata
policy in the PR.
