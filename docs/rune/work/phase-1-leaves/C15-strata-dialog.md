# C15 StrataDialog

| | |
| --- | --- |
| Phase / lane | 1 · Chrome (first in the lane: five WPs wait on it) |
| Plan | §4 C15, `app/dialogs` (45 bullets) |
| Depends on | W2 |
| Unblocks | C12, C16, C17, C19, C26, X1. Strata's Properties, conflicts, Connect to server, passphrase, Format, Open With, leftovers and update dialogs. |
| Owns | `crates/strata-components/src/dialog/`, `scenes/c15.rs` |
| Branch | `feat/<issue>-c15-strata-dialog` |

## Goal

One dialog shell that every Strata dialog builds on, so all 45 `app/dialogs` bullets are met
in one place.

## Scope

- Wrap rune-kit `Dialog` with `.modal(..)` and a `backdrop_blur` that covers the header.
- Open and close motion, which respects the animations switch in `Look`.
- Clicking the backdrop dismisses, unless the dialog is marked as blocking.
- Initial focus goes to a named element, or else the first field. Focus is restored on close.
- Enter submits, unless the focus is in a multi-line field. Escape cancels.
- Sizing stays inside the window. Long content scrolls in the body.
- Slots for title, body, a footer of buttons (primary and secondary), and an optional
  "danger" style.

## Tests

- One script per Behavior bullet group: open and close, backdrop click, Enter and Escape,
  initial focus, focus restore, and a small window.
- A11y golden: the dialog role, its name from the title, modal, and focus trapped.
- Scenes: a basic dialog, a form, a danger confirm, and a small window. All in light and dark.

## Done when

The `featuremap.py qa --node app/dialogs` bullets are covered by scripts, or are listed in
the PR as Strata policy.
