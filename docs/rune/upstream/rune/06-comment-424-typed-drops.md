# 06. Comment on #424: Wayland drops, and drops with any MIME type, the action and a position

| | |
| --- | --- |
| Kind | Comment on [#424](https://github.com/anomalyco/rune/issues/424) (Wayland: native file drops do not reach `on_drop_files`, filed 2026-11-09). S5 drafted U2 as a new issue before #424 existed. |
| Drafts | U2 (S5, from S2 R2) |
| Affects | P1, X2 |
| Retires | P1's `wl_data_device` drop target and its X11 `XdndProxy` (which turns off Rune's own X11 `on_drop_files`) |
| Checked | Re-checked at `2a44156` by source and target. winit 0.30.13 is still the backend (`Cargo.lock`). `on_drop_files`, `on_drop_hover` and `on_drop_move` (`el/drop.rs:63`, `:85`, `:97`) still take paths, a bool and a point only. The program compiled at `2a44156` and was not run: it needs a compositor. #424 is still open with no comments (checked live 2026-11-10), so the comment adds to an empty thread. P1's `wl_data_device` target and its `XdndProxy` are still there (`dnd/x11.rs`, `dnd/mod.rs:35–38`). Earlier: S2's sway measurement at `78d22ed`, and it compiled at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10). Post it as a comment on #424, which is still open. |

Paste from here.

---

We hit this in Strata (lgse/strata), a file manager on Rune, and add a measurement and what a file manager needs
beyond file paths.

**Measured** under headless sway at `78d22ed` (unchanged at `5d421fd`): a raw `wl_data_device`
client offering `text/uri-list`, `text/plain;charset=utf-8` and `image/png` got
`wl_data_source.target` with no type while over the Rune window, then `cancelled` at the drop.
The program below printed nothing (no hover, no move, no drop):

```rust
use rune::{AppSpec, Cx, El, col, el};
#[derive(Default)] struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(el().key("zone").size(300.0, 200.0).fill(rune::oklch(0.6, 0.15, 25.0))
        .on_drop_files(|_: &mut S, files, _| println!("on_drop_files {files:?}"))
        .on_drop_hover(|_: &mut S, over, _| println!("on_drop_hover {over}"))
        .on_drop_move(|_: &mut S, p, _| println!("on_drop_move {} {}", p.x, p.y)))
}
fn main() { rune::run(AppSpec::new("drops", (340.0, 240.0), S::default, view)); }
```

On X11, winit answers XDND itself (`XdndActionPrivate`), takes file URIs of existing files only,
and positions come from sampling `QueryPointer`.

**Beyond file paths**, a file manager takes `text/uri-list`, `text/plain`, `text/x-moz-url`,
`image/png` and HTML drops, and picks copy or move per target and per modifier. Could the fix
take the shape of `on_drop(|s, DropData, tx|)` with the offered types, a typed read, the
action, and a position on every motion, on Wayland and X11? Today we bind `wl_data_device`
ourselves on winit's display (asked in (link to 01)) and put an `XdndProxy` on the X11 window.

Related: #171, #357, (link to 14) for the file-drop hooks' own gaps.
