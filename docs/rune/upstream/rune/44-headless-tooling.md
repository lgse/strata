# 44. Headless tooling: what scripts and `--tree` can't reach

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C6-7, U-X1-2, U-C25-3, U-C7-6, U-C1c-1, U-C1e-5 |
| Affects | C1c, C1e, C5, C6, C7, C17, C18, C25, X1 (tests that run in process instead of as scripts) |
| Retires | In-process tests for IME, titlebar actions, layout-dependent chords and image state; the gallery's 1.1 s warm-up for the commit-budget line |
| Checked | Re-checked at `2a44156` by source: the script parser (`tooling/script.rs`) still takes no input-method event, so a composition goes only through `Runtime::ime` (`runtime/input/keys.rs:279`); `--tree` (`tooling/trace.rs:293`) still carries no titlebar action, which only `Runtime::take_titlebar_action` (`runtime/input/titlebar.rs:83`) sees, and gives elements a video's state (`:360`) but no image's; `Chord::matches_with` is still private (`input/chord.rs:202`) and `Runtime::physical_key_input` `pub(crate)` (`runtime/input/physical.rs:54`); the window title is still the `pub(crate)` field `Runtime::window_title` (`runtime.rs:723`), read by `--tree` (`trace.rs:379`); and `RUNE_COMMIT_BUDGET_MS` still holds back lines within a real second and prints the count only on its next line (`tooling/perf_warn/budget.rs:1–4`, `:58–67`), with nothing at exit. The tooling commits since the pin add `{"open_url": …}` script events (`e970367b`, #428) and `--realtime` (`b726a4d1`, #423), which runs a script on the wall clock but still prints no held-back count at exit. #442 (open: file drops lost below 60 fps) and #432 (open: `elements()` lacks the description) cover none of the six. In the gallery, `mise run test composing` passed (7 in-process IME tests in C6, C17 and C18), and C6's, C7's and C1c's docs still cite U-C6-7, U-C7-6 and U-C1c-1 (`location_bar/mod.rs:232`, `tabs/mod.rs:265`, `collection/keyboard/mod.rs:132`), and the C1 page still waits 1.1 s before its budget lines (`strata-gallery/src/scenes/c1.rs:24`). Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Headless scripts and `--tree`: no IME event, no titlebar action, no image state, no layout-aware key, no window title reader

Context: Strata (lgse/strata), a file manager on Rune tests each behaviour as a headless script where it can, and
falls back to in-process tests where scripts can't reach.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
1. Scripts have no input-method event, so a composition is testable only through `Runtime::ime`.
2. What a press on a `.window_drag()` region did (a move, a double-click zoom) reaches neither a
   script's `expect` nor `--tree`; only `Runtime::take_titlebar_action` sees it.
3. `--tree`'s elements carry no image state, so a script can't check that an `El::pixels`
   image landed (decorative images are hidden from the accessibility tree).
4. A layout-aware chord (Ctrl+A on a Russian layout) can only be checked by a script:
   `Chord::matches_with(event, base, physical, platform)` is private and
   `Runtime::physical_key_input` is `pub(crate)`.
5. `Runtime` has no public reader for the window title (`--tree` shows it; tests read the
   window's accessible name instead).
6. `RUNE_COMMIT_BUDGET_MS` prints at most one line a real second and counts what it left out
   on its next line (`tooling/perf_warn/budget.rs:1–4`). A scripted run on the virtual clock
   ends inside that second, so after one slow first mount no next line comes, and every
   over-budget commit of the script goes unreported. `--realtime` runs on the wall clock, but
   the count held back at the end is still never printed.

**Ask.** An `{"ime": …}` script event; titlebar actions in `--tree` (or an `expect` on them); an
image's size (or "empty") in `--tree`'s elements; a public `Chord::matches_with` or
`Runtime::physical_key_input`; `Runtime::window_title()`; and the budget log's held-back count printed at exit (or its
one-a-second limit on the virtual clock in headless runs).

Related: #432 (`elements()` lacks the description), #442.
