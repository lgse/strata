# C5 InlineRenameEditor

| | |
| --- | --- |
| Phase / lane | 1 · Collection |
| Plan | §4 C5, `operations/rename`, `.../typed-paths` |
| Depends on | W1, W2 |
| Unblocks | C1e (rename slot), M2 |
| Owns | `crates/strata-components/src/rename/`, `scenes/c5.rs` |
| Branch | `feat/<issue>-c5-inline-rename` |

## Goal

A rename editor that sits over a row's label, in any view, and gives the same result
everywhere.

## Scope

- A rune-kit TextInput sized and placed over a given label rect, with the label's font.
- The first focus selects the stem, and the next select-all takes the whole name. Folders
  and dotfiles follow Strata's rules.
- Live validation through a caller-given function (`fn(&str) -> Result<(), Message>`), with
  the message drawn under the field. Policy stays in Strata.
- Enter commits, Escape cancels, and focus loss commits (or cancels, if the name is invalid).
  Tab and Shift+Tab commit and ask the host to move to the next or previous entry.
- Typed paths (`.../typed-paths`): a `/` in the name is reported to the host as a move
  request. The editor does not decide what that means.
- The host keeps the row in view. The editor reports its rect for `keep_on_screen`.

## Tests

- Scripts in `tests/scripts/c5/`, one per Behavior bullet group: stem selection, commit and
  cancel keys, focus loss, Tab to next, validation message.
- An a11y golden: the field's name says it is renaming, and the validation message is a live
  region.
- Gallery scenes, light and dark: a file, a folder, a dotfile and the invalid state.

## Done when

The rename scripts pass, which is part of M2.
