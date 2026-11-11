# 48. No "press outside" for a field

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C5-2 |
| Affects | C5 (inline rename), C6 (path bar: a press outside cancels a frame later) |
| Retires | C5's D4 list of six presses that keep the field focused, each turned into a host commit |
| Checked | Re-checked at `2a44156`: the body's repro, run as a probe (`repros-head/src/bin/rv_48.rs`: a focused `text_input` with its own `on_press`, beside a `preserve_focus_on_press(true)` button), prints `press on the preserve-focus button: field focused true; heard ["button on_press"]` and `right-click beside the field: field focused true; heard []`. The other four presses of the list (a link, a `keeps_text_selection` element, part of another text box, a press that dismisses a modal) weren't re-probed. By source, Rune still has no `on_press_outside` or capture-phase press (no match in `crates/rune/src`; `preserve_focus_on_press` at `el.rs:1055`). In the gallery, `mise run test rename::` passed (31 tests), and C5's D4 and U-C5-2 are still there (`rename/mod.rs:130`, `:247`). Earlier: by source at `5d421fd` and `04b38ae`, and C5's probe at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** A field can't hear a press outside it that leaves it focused

Context: Strata's (lgse/strata, a file manager on Rune) inline rename ends when the user clicks
elsewhere, as a GTK capture-phase click or a web `pointerdown` listener on the document can.

**What's missing** (`5d421fd`, unchanged at `2a44156`). A press that leaves a field focused
tells it nothing: a `preserve_focus_on_press` control, a link, a `keeps_text_selection` element,
part of another text box, a press that dismisses a modal, or any press with another button.
Repro: a focused `text_input` beside a `preserve_focus_on_press(true)` button; press the button,
or right-click beside the field: no handler of the field's runs, and it stays focused.

**Ask.** `El::on_press_outside(|s, press, tx|)` for an element and its subtree, any button.

Related: #106 ((link to 09)).
