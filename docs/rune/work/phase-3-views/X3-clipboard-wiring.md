# X3 Clipboard wiring

| | |
| --- | --- |
| Phase / lane | 3 · Integration |
| Plan | §7 phase 4 exit check, `operations/clipboard/*` |
| Depends on | P2, C19 |
| Unblocks | M4 |
| Owns | `crates/strata-gallery/examples/clipboard.rs` (or a gallery demo module) |
| Branch | `feat/<issue>-x3-clipboard-wiring` |

## Goal

Show copy, cut and paste of entries working end to end, from a component through P2.

## Scope

- A demo app with a C1e list over a temp folder. Ctrl+C, Ctrl+X and Ctrl+V go through P2 with
  the file-manager formats.
- C19's "Files on clipboard" badge follows P2's ownership changes.
- Since strata-components does not depend on strata-platform, this glue lives in the demo,
  and is written so that Strata can copy it.

## Tests

Under W3, with the peer: copy in the demo, then the peer reads all three formats; the peer
sets the clipboard, then the badge clears. Hand tests with Nautilus and Dolphin, on Wayland
and X11, are recorded in the PR.

## Done when

The clipboard half of M4.
