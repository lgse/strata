# 96. Comment on #354: a text-field menu's Undo and Redo

| | |
| --- | --- |
| Kind | Comment on [#354](https://github.com/anomalyco/rune/issues/354) (text fields: expose whether undo and redo are available) |
| Drafts | U-C21-3 |
| Affects | C21 (field menu) |
| Retires | C21's undo of a cloned `TextEdit` when the menu opens, to learn whether Undo would do anything |
| Checked | Re-checked at `2a44156` by source: `TextEdit` still has only `undo_group`, `undo` and `redo` (`edit/history.rs:259`, `:286`, `:298`), no `can_undo` or `can_redo`. #354 is open with no comments (`gh issue view 354`, 2026-11-10), so nothing in the thread says this yet. Gallery: C21's cloned-`TextEdit` check is still there (`crates/strata-components/src/context_menu/field.rs:506–509`) and its test passes (`mise run test undo_and_redo`, 1 test). Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

The same need from Strata (lgse/strata), a file manager on Rune, for an app-drawn text-field context
menu (Linux has no native one). Today we clone the field's `TextEdit` when the menu opens and undo
the clone to see whether Undo would change anything. `TextEdit::can_undo()` and `can_redo()` would
replace that.
