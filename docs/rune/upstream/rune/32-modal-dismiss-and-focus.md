# 32. `El::modal`: say why it was dismissed, and a capture layer that leaves focus where it is

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C20b-2 (the reason half; its `Tx` half is fixed, see the README), U-C24b-4 |
| Affects | C20b (colour picker), C24b (autoscroll shield), C5 (rename commits on blur), C17 |
| Retires | C20b's Escape taken in an ancestor's `on_key` first, and its outside-press report a frame late; C5's "autoscroll commits the rename" note |
| Checked | Re-checked at `2a44156`. By source, `El::modal` and `El::modal_tx` (`el.rs:1175–1185`) still store one dismissal, a `PressFn` (`el.rs:182`, `ModalSpec` at `:232–235`), for Escape and a press outside alike, with no reason. The focus half was re-run with a new headless probe, `rv_32`: a focused `text_input`, then a full-window `.modal(false, ..)` with no focusable child mounted: the field's `ElementInfo::focused` goes false, and true again once the modal is gone. The modal focus machine (`22aa8cde`) still focuses "its `autofocus` control, else its field, else itself" on opening (`runtime/input/modal_focus.rs:52–54`). `4d6c3206` (#498) and `2a44156b` (#510) change only where `Tx::return_focus` sends focus as a menu closes, not a modal taking focus as it opens. In the gallery, `mise run test a_press_outside_keeps_the_colour_and_remembers_it` and `the_popover_opens_on_the_plane_and_escape_puts_the_colour_back` passed (1 test each: C20b's colour picker). C20b's notes (`controls/color_picker/mod.rs:153`, `:173`, `view/mod.rs:302`) and C24b's (`scroll/mod.rs:121`, `:245–252`) are still there. Earlier: by source at `04b38ae` (`modal_tx` added in `22aa8cde`); U-C24b-4 from C24's probe at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `El::modal`: tell the dismissal why (Escape or a press outside), and offer a modal that doesn't take focus

Context: Strata (lgse/strata), a file manager on Rune.

**What's missing** (`2a44156`):
- `El::modal` and `El::modal_tx` (new in `22aa8cde`) call the same dismissal for Escape and for
  a press outside, with no reason. A colour picker that cancels on Escape and keeps the colour on
  a press outside has to catch Escape in an ancestor's `on_key` first.
- `El::modal` moves focus into itself, so a text field that commits on blur commits when any
  modal opens, even one with nothing to type into (an autoscroll shield that only catches
  presses and Escape). Repro: focus a `text_input`, mount a full-window `.modal(false, ..)` with
  no focusable child: the field's `ElementInfo::focused` goes false until the modal closes
  (ran headless at `2a44156`). The modal focuses its `autofocus` control, else its field, else
  itself (`runtime/input/modal_focus.rs:52–54`).

**Ask.**
- `modal_with(|s, why: Dismiss, tx| ..)`, with `Dismiss::Escape` and `Dismiss::Outside`.
- An input-capture layer (presses, Escape) that doesn't take focus, or a modal option that
  leaves focus where it is.

Related: #291 (rune-kit `Dialog` drops a dialog without `on_close`).
