# 12. Comment on #310: minimise a borderless window, and its maximised state, on Linux

| | |
| --- | --- |
| Kind | Comment on [#310](https://github.com/anomalyco/rune/issues/310) (maximise, minimise and the maximised state) |
| Drafts | U7 (S5, from S1 R2); U6, its maximised-state half |
| Affects | P3, X1 (header) |
| Retires | X1's hidden Minimise button on Wayland, the header's guessed maximise/restore icon |
| Checked | Re-checked at `2a44156` by source. `window/shell_requests.rs:20` (`self.options.borderless \|\| …`) still refuses the minimise, and `Tx::minimize`'s docs (`el/tx.rs:265–270`) still say it does nothing for a borderless window. Only `Tx::toggle_maximized` (`el/tx.rs:276`) exists: there is no `Cx::window_maximized` or `window_state`. The program compiled at `2a44156` and was not run: it needs a compositor. #310 is still open with no comments (checked live 2026-11-10). P3's `Gap::RuneMinimize` on Wayland and its own X11 `WM_CHANGE_STATE` (`csd/mod.rs:163–164`) are still there, as is the header's `maximized: Option<bool>` that is unknown on Wayland (`header/chrome.rs:36–40`). `mise run test header::` passed (5 tests). Earlier: the same at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10). Post it as a comment on #310, which is still open. |

Paste from here.

---

The same need from Strata (lgse/strata), a file manager on Rune, on Linux. Running this as a debug
build and clicking Minimise logs `rune: minimize ignored: a window without a minimize button can't
minimize`, and no request reaches the compositor (`window/shell_requests.rs:20` at `5d421fd`):

```rust
use rune::*;
struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full()
        .child(row().h(40.0).w_full().window_drag().child(text("borderless")))
        .child(row().key("min").w(120.0).h(32.0).role("button").label("Minimise").child(text("Minimise"))
            .on_press_tx(|_: &mut S, tx: &mut Tx| tx.minimize()))
}
fn main() {
    App::new(AppSpec::new("minimise", (400.0, 240.0), || S, view))
        .window_options(WindowOptions { borderless: true, ..Default::default() })
        .run();
}
```

Borderless windows are exactly the ones that draw their own minimise button, so the check
refuses the case that needs it. GNOME, KDE and X11 window managers honour
`xdg_toplevel.set_minimized` and `_NET_WM_STATE_HIDDEN`.

For the maximised state: `Cx::chrome_insets()` is 0 on Linux, and only `Tx::toggle_maximized`
exists, so a header can't choose between Maximise and Restore, or drop its resize bands when
maximised. `Cx::window_maximized()` (or `window_state()` with #313's tiled edges) would cover
both.
