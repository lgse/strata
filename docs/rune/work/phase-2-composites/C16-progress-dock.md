# C16 ProgressDock

| | |
| --- | --- |
| Phase / lane | 2 · Chrome |
| Plan | §4 C16, `operations/progress`, `.../dock`, `.../jobs` |
| Depends on | C15 (blocking progress dialog), W2 |
| Unblocks | M4 |
| Owns | `crates/strata-components/src/progress_dock/`, `scenes/c16.rs` |
| Branch | `feat/<issue>-c16-progress-dock` |

## Goal

Show running operations as cards, without the component owning any operation.

## Scope

- A bottom-right dock from rune-kit ToastRegion, Progress and Card. Each card shows the title,
  bytes done and total, speed and time left (formatted from data), and a Cancel action.
- Cancelling sends an intent. The card then turns into a notice that closes itself.
- Brief notices share the dock.
- A blocking progress dialog on C15, for operations that must hold the window.
- The Jobs dashboard as a footer element (it slots into C19). It shows a count and opens a
  list.
- Progress updates through `Live` values where Rune allows it, so a byte counter doesn't
  commit the view every frame.

## Tests

Scripts: a card appears, updates and completes; cancel turns it into a notice that closes; the
blocking dialog; stacking several cards. A11y golden: progress bars with values, and notices
as live regions.

## Done when

The progress, dock and jobs bullets pass headless.
