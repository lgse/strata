# 05. Linux `drag_out`, with several MIME types, an "ask" action and an element snapshot

| | |
| --- | --- |
| Kind | New issue, linking #171 (S5 owner question 5: new issue rather than a comment) |
| Drafts | U1 (S5, from S2 R1) |
| Affects | P1, P0, X2 |
| Retires | P1's `wl_data_device` and XDND drag source, its second-`Runtime` drag icon |
| Checked | Re-checked at `2a44156`. By source, `SUPPORTED = cfg!(target_os = "macos")` (`drag_out/mod.rs:29`), and `drag_out::snapshot` is still `pub(crate)` (`:320`). The re-read also found that `DragItem` (`:51`) already has files, PNG data and text, `DragOperation` Copy, Move and Link (`:175`), and `DragOutcome` Copied, Moved and Linked (`:275`), the same at `5d421fd`. The body's "no action" was wrong and is corrected, and the ask keeps "ask" and arbitrary MIME types. The headless half was re-run: `doc_05 --script drag.json --trace t.json` at `2a44156` records `drag-out #0/tile` and prints `on_drag_start`, then `on_drag_end Cancelled`, as before. The windowed half is S2's sway run at `78d22ed`, and it was not re-run. P1's workaround is still there: `dnd/wayland.rs:208` `start_drag`, and `dnd/icon.rs:196`'s second `Runtime`. P1's own module docs (`dnd/mod.rs:32`) repeat the "no actions" claim. Earlier: the same at `5d421fd` and `04b38ae`. |
| Status | Ready to file, trimmed: copy, move and link and the outcome's action are dropped from the ask, since `DragOperation` and `DragOutcome` already have them (at the pin too). Ask, arbitrary MIME types, the Linux backends and a public snapshot remain. Re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10. |

Paste from here.

---

**Title:** Linux: `drag_out` does nothing; support several MIME types, an "ask" action and an element snapshot

Related: #171 (`drag_out` on Linux, for file paths). This asks for the same drag source with typed
data: Strata (lgse/strata), a file manager on Rune, drags `text/uri-list` beside
`x-special/gnome-copied-files`, `application/x-kde-cutselection` and an in-process id, and needs
copy, move, link and ask.

**What's missing** (`5d421fd`, unchanged at `2a44156`): `drag_out::SUPPORTED` is
`cfg!(target_os = "macos")` (`drag_out/mod.rs:29`). `DragItem` carries files, PNG data and text
only, with copy, move and link (`DragOperation`) but no "ask". The live-frame snapshot that macOS
uses for the drag image (`drag_out::snapshot`, `:320`) is `pub(crate)`.

**Repro.** Headless, run with `--script drag.json --trace t.json`; windowed, press the tile and
drag it out over a file manager.

```json
{"events": [{"t": 0.1, "type": "move", "x": 70, "y": 45}, {"t": 0.15, "type": "down", "x": 70, "y": 45}, {"t": 0.2, "type": "move", "x": 90, "y": 45}, {"t": 0.3, "type": "move", "x": 200, "y": 80}, {"t": 0.4, "type": "up", "x": 200, "y": 80}]}
```

```rust
use rune::{AppSpec, Cx, DragItem, El, col, el};
#[derive(Default)] struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(el().key("tile").size(140.0, 90.0).fill(rune::oklch(0.55, 0.15, 260.0))
        .drag_out(|_: &S| Some(DragItem::new().text("hello from rune")))
        .on_drag_start(|_: &mut S, _| println!("on_drag_start"))
        .on_drag_end(|_: &mut S, outcome, _| println!("on_drag_end {outcome:?}"))
        .on_drag(|_: &mut S, d, _| println!("on_drag {:?}", d.phase)))
}
fn main() { rune::run(AppSpec::new("drag", (320.0, 200.0), S::default, view)); }
```

Headless (at `5d421fd` and `2a44156`), the trace records `drag-out #0/tile` and the program
prints `on_drag_start` then `on_drag_end Cancelled`. Windowed under sway it prints only `on_drag Down`, `Move`s and `Up`:
no system drag, no `on_drag_start` or `on_drag_end` (measured at `78d22ed`).

**Ask.**
- `drag_out` on Wayland (`wl_data_device`) and X11 (XDND).
- `DragItem` with arbitrary MIME types and lazily produced data per type.
- `DragOperation::Ask` beside copy, move and link (XDND's `XdndActionAsk`, `wl_data_device`'s
  `ask`), with the target's pick in the outcome.
- A public snapshot of an element from the live frame, for the drag icon.

**Why.** A downstream crate starts a `wl_data_device` drag from `on_drag` on winit's display
(which needs (link to 01)), and renders the drag icon from a second headless `Runtime`.

Related: #171, #357 (in-app drop targets), (link to 06).
