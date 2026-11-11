# 55. Linux: middle-click paste of the primary selection in text fields

| | |
| --- | --- |
| Kind | New issue (related #483) |
| Drafts | U-C24b-3 (and P2's "Rune has no primary selection") |
| Affects | C24b, C5 (rename field), C6, every text field; P2 |
| Retires | C24b's "no middle-click paste" deviation |
| Checked | At `2a44156`, by source: a middle press goes only to the innermost element with `pointer_buttons` and `on_press` or `on_drag` (`Runtime::button_down`, `runtime/input/gesture.rs:179`); fields have no middle-press path, and there's no primary-selection source or reader (no `primary_selection` or `zwp_primary_selection` in `crates/rune`). #483 (Shift+Insert) is open with no comments and not fixed: `KeyEvent::edit_command` still takes only `Key::Command` chords (`edit/keys.rs:146`), and a field ignores `Key::Insert` (`edit/input.rs:276`). #483 says its case needs no primary selection, so this isn't a duplicate. Gallery: C24b's deviation is still there (`scroll/mod.rs:122`, `:240`); the `a_middle_press` tests pass. Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Linux: text fields don't paste the primary selection on a middle click, and Rune has no primary selection

Context: Strata (lgse/strata), a file manager on Rune.

On Linux a middle click in a text field pastes the primary selection, and selecting text makes
it the primary selection. Rune's fields take no middle press (a middle tap on a `text_input`
goes to the nearest ancestor with `pointer_buttons([Middle])`), and there is no primary
selection source or reader.

**Ask.** Middle-click paste of the primary selection in text fields on Linux, selected text
offered as the primary selection (`zwp_primary_selection_v1` on Wayland, `PRIMARY` on X11), and
an app API to read it.

Related: #483 (Shift+Insert paste on Linux), (link to 07).
