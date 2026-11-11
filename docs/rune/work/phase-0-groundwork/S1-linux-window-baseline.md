# S1 Linux window baseline (spike)

| | |
| --- | --- |
| Phase / lane | 0 · Desktop |
| Plan | §7 phase 0 ("Rune window on Hyprland, GNOME and X11"), P3, P5, the popup-surface note |
| Depends on | — |
| Unblocks | P3, S5 (repros) |
| Owns | `docs/decisions/S1-linux-window.md`; any prototype stays on the spike branch |
| Branch | `docs/<issue>-s1-linux-window-baseline` |

## Goal

Learn what a plain Rune window does on Hyprland, GNOME (Mutter) and X11 at the pinned rev,
before anyone builds on it.

## Questions

1. Does the window open, resize, take focus and draw at fractional scales on each
   compositor? Note any wgpu fallback, such as lavapipe on the QEMU images (plan §8).
2. With decorations off, what does each compositor do? Can winit's `drag_window` and
   `drag_resize_window` work from Rune handlers? This is P3's premise.
3. Which compositors offer server-side decorations (`xdg-decoration`), and which refuse them?
4. What does Rune read from the Settings portal today, and how are changes delivered? This is
   P5's starting point.
5. How do in-window menus clip against a small window? This decides the chooser's minimum
   size until popup surfaces exist.
6. Is the double-click interval fixed at 0.5 s on each platform, as the README says?

## Deliverable

A decision record: one table per question with each compositor's result and a screenshot or
log, plus a headless repro for each finding that becomes a Rune issue (S5 files them).

## Done when

P3's owner can start without re-asking any question above. Prototype code is not merged.
