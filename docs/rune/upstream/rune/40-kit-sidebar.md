# 40. rune-kit `Sidebar` and `Reorder`: a rail form, focus without pressing, sections, keyboard moves

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U18, U19, U20, U21, U22 (C8) |
| Affects | C8 (places sidebar) |
| Retires | C8's own list, headings, section and icon-only rail on `SidebarRow`, its parked reorder move |
| Checked | Re-checked at `2a44156` by source: `sidebar-list` still has `focus_follows()` (`rune-kit/src/shell/sidebar.rs:133`), `SidebarHeader` is still `focusable(false)` (`:239`), and `Reorder::follow` is still `pub(super)` (`shell/reorder.rs:72`). `SidebarRow::reorder` (`sidebar.rs:475`) still takes a plain `fn` accessor and a drop with no `Tx`, and the row inset is still a private 8 (`:22`), the icon box 16 pt (`:589–590`), the selected fill `c.pressed` (`:586`) and the busy spinner `icon_muted` (`:640`). `SidebarRow::select_only` (`:415`, at the pin too) makes rows unfocusable, which isn't focus without a press. The only change since the pin, `cbc93701`, makes a reorder hold one grip and let go of a lost release (the row unmounted mid-drag); it adds no keyboard move or cancel. In the gallery, `mise run test sidebar::` passed (15 tests) and `mise run test c8` (13 gallery tests and 3 more), and C8's docs still list U18 to U22 (`sidebar/mod.rs:106–131`). Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** rune-kit `Sidebar`: no icon-only rail or compact step, headers that can't take focus, collapsed sections that drop the open row, and `Reorder` without keys or cancel

Context: Strata's (lgse/strata, a file manager on Rune) places sidebar (bookmarks, devices, a
pending-unmount spinner) with a compact icon-only rail and drag-to-reorder bookmarks.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
1. `SidebarRow` has no icon-only form and no tooltip: with an empty label it keeps the label slot
   and gap, and draws an ellipsis. `SplitView`'s sidebar knows only open and closed.
2. The list presses the row focus reaches (`focus_follows`, `sidebar.rs:133`), and
   `SidebarHeader` takes no focus (`focusable(false)`, `:239`). A file manager opens a place
   only on Enter or a click, and steps onto headings.
3. A collapsed `SidebarSection` drops every row, with no way to keep the open item's row in view.
4. `Reorder` has no keyboard move; a drag past the end clamps with no way to cancel; a middle
   button reorders too; `Reorder::follow` is private; `SidebarRow::reorder` takes a plain `fn`
   accessor; its drop runs with no `Tx`; and a drag released off the row ends with no press.
5. `SidebarRow`'s icon box is fixed at 16 pt and its row inset at a private 8; the selected fill
   is fixed to `c.pressed`; the busy spinner is `icon_muted`; it names the row by its label only;
   and `over` isn't public.

**Ask.** An icon-only rail form with tooltips and a compact `SplitView` step; focusable headers
and an option not to press on focus; a collapsed section that can keep one row; keyboard moves,
cancel, primary-only drags, a public `follow`, closures and a `Tx` for `Reorder`; the icon box,
inset, selected fill, spinner colour and accessible name as options.

Related: #396.
