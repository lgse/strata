# C7 BrowserTabs

| | |
| --- | --- |
| Phase / lane | 1 · Chrome |
| Plan | §4 C7, `browser/tabs`, `.../session-restore` |
| Depends on | W1 (drop zones), W2 |
| Unblocks | X2, M3 |
| Owns | `crates/strata-components/src/tabs/`, `scenes/c7.rs`, `strata-kit/src/slots/tabs.rs` |
| Branch | `feat/<issue>-c7-browser-tabs` |

## Goal

rune-kit `TabStrip` with what a file manager adds: tabs as drop targets, and reopening closed
tabs.

## Scope

- Wrap `TabStrip`, keeping reorder, tear-out and middle-click close.
- Each tab and the New tab button is a W1 `DropZone`. The intents: transfer into the tab's
  folder, open a dropped folder as a new tab, and save dropped content. The host decides what
  happens.
- Hovering a drag over a tab for a short delay activates it.
- A closed-tabs stack (data), with Ctrl+Shift+T to reopen.
- The window title follows the active tab: an intent the host applies with Rune's title API.
- Session restore is Strata's. The component takes its tab list as data.

## Tests

Scripts: middle-click close, reopen-closed, drop intents on a tab and on New tab, and a hover
that activates. A11y golden: tab roles and names, and the selected state. Scenes: one tab,
many tabs overflowing, and a drop hover. All in light and dark.

## Done when

The tabs scripts pass headless.
