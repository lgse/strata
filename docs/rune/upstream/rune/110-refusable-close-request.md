# 110. A close the window manager asks for cannot be refused

| | |
| --- | --- |
| Kind | New issue |
| Drafts | A21 of the parity plan (`docs/rune/parity/packages/A21.md`), for GTK's close guards (finding [#2473](https://github.com/lgse/strata/issues/2473)) |
| Affects | `strata-app`'s windowed host (`crates/strata-app/src/host/windowed.rs`, "Refused closes"): GTK refuses a window's close while a tab has a file operation running, and the last window's while a custom action's job runs or waits (`shell::close_guard`). A close from the title bar, Alt+F4 or the compositor can't be refused on Rune |
| Retires | `crates/strata-app/src/host/windowed/refused.rs` (`keep_open`), which moves the closing window's state into a new window, opened from `on_close`, and leaves the closing one a stand-in state; and the `Start::Kept` start of `host::windowed::window` |
| Checked | By reading at `631e8a67` (2026-11-18): `WindowEvent::CloseRequested` goes straight to `close` (`window/events.rs`, line 244). `close` (`window/lifecycle.rs`, line 343) hides the window before it runs `close_hook`, and `close_hook` runs `App::on_close` (line 395) with no way to answer. The window goes after that unless the app has `keep_running`. By a window test in a private sway session (`strata-rune`'s `window::a_close_the_window_manager_asks_for_waits_for_a_running_job`): the window is hidden and its state dropped, and only the workaround's new window keeps the job's state |
| Status | Drafted, not filed: to file at the owner's filing pass |

Paste from here.

---

**Title:** Let an app refuse a close the window manager asks for

Context: Strata (lgse/strata), a file manager on Rune, is moving from GTK. GTK refuses a window's
close while a tab has a file operation running, and the last window's while background jobs
run. It says why in a dialog, and the window stays (`GtkWindow::close-request` returning
`true`). Rune lets the app refuse its own closes, since it decides whether to call
`Tx::close_window`. It can't refuse a close from the window manager: the title bar's button,
Alt+F4, or the compositor's close.

At `631e8a67`, `WindowEvent::CloseRequested` calls `close` (`window/events.rs`, line 244).
`close` hides the window first ("Off the screen first: the close looks done at once and
`on_close` runs unseen"). Then it runs `App::on_close`, which returns nothing, and the window and
its state go unless the app has `keep_running`. So `on_close` can only clean up.

**Repro sketch** (a window test, Wayland, `631e8a67`):
1. Build an `App` whose `on_close` wants to keep the window: it sets a flag, and nothing it can
   do keeps the window.
2. Open it, then close it from the compositor (`swaymsg '[con_id=…] kill'`).
3. Expected: the app can say no, and the window stays shown with its state. Actual: the window is
   hidden before `on_close` runs, and then it is destroyed.

**Ask.** A close request the app can answer before anything is hidden. For example,
`App::on_close_request(fn(&mut S, &mut Tx) -> bool)`, where `false` keeps the window as it is.
It would run for `WindowEvent::CloseRequested`, and on macOS for the window's close button and
⌘W. `on_close` would then run only for a close that goes ahead. A quit (`on_quit`) stays as it
is. Strata would show its dialog from the request and return `false`.
