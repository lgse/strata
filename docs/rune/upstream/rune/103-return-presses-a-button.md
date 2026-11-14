# 103. Return can't press a focused button, and the only switch for it is `Cx::web_focus`

| | |
| --- | --- |
| Kind | New issue |
| Drafts | A5 of the parity plan (`docs/rune/parity/packages/A5.md`); finding [lgse/strata#2310](https://github.com/lgse/strata/issues/2310) |
| Affects | Every button of Strata's Rune interface: the pane header's, an unreadable folder's Retry, the window header's actions; strata-components' dialogs (C15) already route around it |
| Retires | `crates/strata-app/src/buttons.rs` (`on_return`), and C15's `Route::Press` for Return in `crates/strata-components/src/dialog/keys.rs` |
| Checked | By reading at `2a44156` (2026-11-13): `takes_return` in `runtime/input/focus.rs` (lines 121-128) and the key-routing table in `crates/rune/README.md`; and in `strata-app`'s hosted tests `buttons::tests`, which fail with the workaround switched off |
| Status | Drafted, not filed. Workaround in force: a focused button's `on_key` sends the Space that presses it for a plain Return |

Paste from here.

---

**Title:** Let a focused button take Return without `Cx::web_focus`

Context: Strata (lgse/strata), a file manager on Rune, is moving from GTK, where a focused
button presses on Return as well as Space.

Rune follows AppKit: Return presses the window's default button, and presses only a focused
row, link or `focus_highlight` element; Space presses a focused control (`takes_return` in
`runtime/input/focus.rs`, and the key-routing table in the README). `Cx::web_focus` makes
Return press any focused `on_press` element, but it also makes every such element a Tab stop
that takes focus on a click, which an app porting from GTK may not want everywhere.

**Repro** (headless, `2a44156`): a window with a focusable button whose `on_press` counts, no
default button; focus it and send Return: the count stays at 0. Send Space: it goes to 1.

**Ask.** A per-element opt-in, as `focus_highlight` is for rows (say `El::press_on_return`),
or a platform default that follows GTK and Windows, where a focused button takes Return.
