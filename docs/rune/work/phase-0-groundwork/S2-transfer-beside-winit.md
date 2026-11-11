# S2 Data transfer beside winit (spike)

| | |
| --- | --- |
| Phase / lane | 0 · Transfer |
| Plan | P1, P2, §8 ("Wayland DnD inside winit … prototype P1 in phase 0") |
| Depends on | — (use W3's harness if it has landed) |
| Unblocks | P0, then P1 and P2 |
| Owns | `docs/decisions/S2-transfer.md`; the prototype stays on the spike branch or becomes P0's first commit |
| Branch | `docs/<issue>-s2-transfer-beside-winit` |

## Goal

Retire the biggest risk in the plan: whether `wl_data_device` and XDND can be bound beside
winit's own seat handling without breaking its pointer state.

## Questions

1. Can we reach winit's `wl_display` from Rune, and get a seat? Plan P1 points to Rune's
   `platform/layer_shell.rs` and `activation.rs` as the pattern. Can that be done without
   patching Rune?
2. Is there a second event queue on that display, and who dispatches it? Can it be woken
   from Rune's loop, or does it need its own thread?
3. Start a `wl_data_device` drag from a pointer press that Rune saw. Does winit still get the
   release, and does Rune's pointer capture stay sane?
4. Receive a drop with arbitrary MIME types, with a position on every motion event. Is that
   in surface coordinates that Rune's hit-test can use?
5. Does `zwlr_data_control_v1` or `ext_data_control_v1` exist on Hyprland, sway, GNOME and
   KDE? Where it is missing, what does the fallback to `wl_data_device` cost P2?
6. On X11, can XDND and selection ownership share winit's connection, or do they need a
   second `x11rb` connection?
7. Can a drag icon come from an element snapshot? Which Rune API gives one headless?

## Deliverable

A decision record with a yes or no per question, the binding approach chosen for P0, and
headless repros for S5's Rune issues (Linux `drag_out`, drops with MIME types and position,
clipboard MIME types and image writes).

## Done when

P0 has an approach with no Rune patch, or the record says what Rune must expose and S5 has
filed it.
