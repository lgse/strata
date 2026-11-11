# 38. rune-kit menus and pop-up lists: submenu keys and hover, item keys, names, rich labels

| | |
| --- | --- |
| Kind | New issue (related #395) |
| Drafts | U-C21-4 (its keyboard and hover half), U-C21-5, U-C6-8, U-C6-4 |
| Affects | C21 (context menus), C6 (location bar's hierarchy menu and completion list), C17, C18 |
| Retires | C21's own Right/Left/Enter handling and the sent Tab into a submenu, its "rows keyed by position" deviation, C21's reliance on `Menu::build`'s layer order; C6's own completion list instead of `PopupList` |
| Checked | Re-checked at `2a44156` by source: `MenuItem::submenu(entries, open)` still opens only when the app sets it (`rune-kit/src/menu.rs:104`; the submenu mounts only `if open`, `:230`), and Rune core's focus routing gives a `menu` Up, Down and type-select only (`runtime/input/focus.rs:6–13`), no Right or Left. Items are still keyed `{key}-item{i}` by position (`menu.rs:358`), the item's accessible name is still its drawn label (`:169`), and `Menu` has no name setter. `Menu::build` still pushes `[scrim, point, surface, submenus…]` with no key to find them by (`:376–401`), and `El` still has no key getter. `PopupItem` has no rich label and `PopupList::build_list` is still `pub(crate)` (`rune-kit/src/popup_list.rs:233`). The commits to `menu.rs` and `popup_list.rs` since the pin (`cded95f0`, right-to-left mirroring; `1ea05bc7`, state machines) and the focus commits around menus (`22aa8cde`, `4d6c3206`, `2a44156b`: `Tx::return_focus`) add none of these, and #395 (open) asks only for a trailing element and a highlight callback. In the gallery, `mise run test context_menu` passed (50 tests, including C21's own Right/Left keys, its sent Tab and `layer_order_is_runekits`), and C6's docs still give its own list for U-C6-4 (`location_bar/mod.rs:166`, `:222`). Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** rune-kit `Menu` and `PopupList`: submenus have no keys or hover-open, items are keyed by position, names can't differ from the drawn text, and list items take no rich label

Context: Strata (lgse/strata), a file manager on Rune.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
- A submenu opens only when the app sets it open: no Right, Left, Enter or Space on a submenu
  row, the keyboard can't enter one, and no hover opens one.
- `MenuItem` is keyed by position, so a row inserted above the focused one moves the keyboard.
- `Menu` takes no accessible name, and a `MenuItem` can't give one apart from the text it draws,
  so an item shortened to fit reads shortened.
- `PopupItem` has no rich label (spans, or `label_with(El)`), so a completion list can't bold
  or colour the matched part, and `PopupList::build_list` is `pub(crate)`
  (`rune-kit/src/popup_list.rs:233`), so an app can't reuse the list inside its own popup.
- An El's key can't be read once built, so an app adjusting a menu's surfaces finds them by the
  order `Menu::build` pushes them (`[scrim, point, surface, submenus…]`).

**Ask.** Right, Left, Enter and Space on submenu rows with focus moving into the submenu, and
hover-open with a delay; `MenuItem::key`; `Menu::label` and `MenuItem::label` apart from the
drawn text; the built surfaces keyed (or returned) so an app can find them; `PopupItem::label_with(El)` (or
spans) and a public `PopupList::build_list`.

Related: #395 (trailing element and highlight callback), (link to 04).
