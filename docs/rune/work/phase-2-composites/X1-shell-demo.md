# X1 Shell demo

| | |
| --- | --- |
| Phase / lane | 2 · Integration |
| Plan | §7 phase 1 exit check |
| Depends on | P3, P5, P8, P9, K1–K4, C15 |
| Unblocks | M1, the first `strata-platform` release |
| Owns | `crates/strata-gallery/examples/shell.rs` (or a `shell` binary in the gallery) |
| Branch | `feat/<issue>-x1-shell-demo` |

## Goal

Show in this repository that the foundations fit together, before Strata moves onto them.

## Scope

- A themed window (K1, with K2 parameters and K4 overrides) with a Strata header: P3
  decorations, window buttons drawn with strata-kit, a drag region and the window menu.
- Desktop settings (P5) feed the text scale, the double-click interval and the animations
  switch into the `Look`.
- Single instance (P8): a second launch with paths forwards them, and the primary opens a
  window and focuses it.
- A GIO call through P9 (list the mounted volumes) that shows its result in a C15 dialog.
- Live reload of an overrides file.

## Tests

A headless script for the header, the dialog and an override reload. Hand tests on Hyprland,
GNOME and X11 for decorations and the second launch, recorded in the PR.

## Done when

M1: a themed empty window with the header, and a second launch forwarded. Tag the first
`strata-platform` release (S5's workflow) with the owner's go-ahead.
