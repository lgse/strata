# Calibration anchors

Ten real scenarios, scored against the rubric in [SKILL.md](SKILL.md) at
`feat/qa-harness` dbda0ed4 (2026-10-19). They are **calibration anchors**: use
them to align judgement before and during a batch. They are not recorded in the
scenario files; whoever applies the rubric writes those fields.

Re-check an anchor's evidence before relying on it after the code or the
corpus changes. The re-score triggers in SKILL.md apply to anchors too.

Anchors 9 and 10, and `permanent-delete-confirmation`, were later removed from
the suite for their low score per unit of cost; they stay here as scoring
examples at that revision.

| # | Scenario | I | C | K | D | Score | Note |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | `copy_conflicts/dismissing-conflict-preserves-both` | 5 | 5 | 4 | 4 | 84 | paste-conflict dismissal by Escape/Cancel/close/move keeps both files; only scenario checking dismissal outcomes |
| 2 | `copy_conflicts/keep-both-then-undo-remaining` | 4 | 4 | 4 | 3 | 52 | Keep Both via keyboard from default Replace focus, then undo; superset of keep-both-then-undo, only keyboard route through the conflict dialog |
| 3 | `entry_management/permanent-delete-symlinked-parent` | 5 | 3 | 4 | 4 | 42 | permanent delete inside a symlinked folder keeps link and target; fs logic unit-tested, alias pane path is not |
| 4 | `dialogs_and_menus/modal-dialog-blocks-window-shortcuts` | 3 | 4 | 4 | 5 | 54 | window shortcuts stay blocked over a modal; added by a fix, no unit or other scenario covers it |
| 5 | `copy_conflicts/keep-both-then-undo` | 4 | 2 | 4 | 3 | 17 | Keep Both by click, then undo; every assertion repeated by keep-both-then-undo-remaining |
| 6 | `accessibility/toolbar-controls-named` | 3 | 3 | 3 | 5 | 31 | toolbar buttons expose names; Search and Toggle sidebar names checked nowhere else |
| 7 | `view_switching/appearance-menu-switches-view` | 3 | 2 | 3 | 4 | 14 | Icons switch via Appearance menu persists; appearance-menu-switches-view-list covers the same path for List |
| 8 | `visual_baselines/icons-hover-baseline` | 2 | 2 | 3 | 1 | 7 | hovered Icons tile rendering; only hover capture, fixture shared with icons-view-baseline |
| 9 | `filter_results/filter-selection-uses-theme` | 2 | 2 | 3 | 3 | 10 | filter text selection uses the theme accent; unique but cosmetic, added with fix #778 |
| 10 | `column_headers/column-header-titles-bounded` | 2 | 1 | 2 | 2 | 0 | column title bounds and centring; geometry-only assertions |

I = impact, C = coverage, K = criticality, D = diagnostic. Scores follow
`(50*(C-1)*P + 32) // 64` with `P = 3*(I-1) + 3*(K-1) + 2*(D-1)`.

By score: #1 84, #4 54, #2 52, #3 42, #6 31, #5 17, #7 14, #9 10, #8 7,
#10 0.

## 1. dismissing-conflict-preserves-both: 5 / 5 / 4 / 4 → 84

- **Behaviour:** pasting onto an existing `archive/todo.txt` and dismissing the
  conflict dialog by Cancel, Escape, or the dialog's close button (copy), or
  Cancel (move), leaves both files untouched and creates no numbered copy.
  It also asserts that a move offers no Keep Both and that a file collision
  offers no Merge.
- **Impact 5:** a dismissal that applies a choice silently overwrites or
  moves user data.
- **Coverage 5:** four dismissal routes that reach different handlers (key,
  button, window close, move instead of copy). `similar` ranks
  `replace-restores-original-on-undo` and `keep-both-then-undo` closest, and
  they share only setup. `src/ui/browser/transfer/tests.rs` checks which
  choices the dialog offers (`conflict_dialog_offers_skip_for_a_multi_item_paste`)
  but not the outcome of dismissing it. `archive-conflict-keyboard-choices`
  dismisses a different dialog (an archive name collision).
- **Criticality 4:** `src/adapters/local_operations` has ~96 commits and ~55
  `fix` commits, `src/ui/browser/transfer*` ~30 and ~15; the outcome spans
  dialog, operation and listing. Not 5: no race or external process.
- **Diagnostic 4:** each variant takes one route, and the failing file check
  names the file that changed. Not 5: it does not say whether the dialog or
  the operation applied the choice.

## 2. keep-both-then-undo-remaining: 4 / 4 / 4 / 3 → 52

- **Behaviour:** the paste-conflict dialog opens with Replace focused;
  Shift+Tab then Return chooses Keep Both; `todo (2).txt` is created (skipping
  the existing `todo (1).txt`), selected and focused; Ctrl+Z removes only the
  copy.
- **Impact 4:** a wrong default or a wrong undo trashes or keeps the wrong
  file. It is recoverable, so not 5.
- **Coverage 4:** the only keyboard route through the paste-conflict dialog,
  and the only check of its default focus. Numbering is unit-tested
  (`duplicating_an_existing_numbered_name_advances_its_index`,
  `keeping_both_in_a_cross_folder_paste_generates_a_unique_name` in
  `src/adapters/local_operations/tests/`); selection and focus after the dialog
  are not.
- **Criticality 4:** same area as #1, plus focus restoration after an
  asynchronous operation completes.
- **Diagnostic 3:** six phases are chained: default focus, keyboard choice,
  numbering, selection, focus, undo. The message names the phase, but
  transfer, operations and focus code could all be behind it.

## 3. permanent-delete-symlinked-parent: 5 / 3 / 4 / 4 → 42

- **Behaviour:** open `documents-alias` (a symlink to `documents`) in Columns
  (select) or List (double-click), Shift+Delete `notes.txt`, confirm; the file
  is gone from the target, the entry leaves the alias listing, and the link
  and target folder survive.
- **Impact 5:** permanent deletion; following or removing the wrong path is
  unrecoverable.
- **Coverage 3:** `permanent_delete_accepts_a_symlink_in_the_parent_path` and
  its neighbours in `src/adapters/local_operations/tests/deletion.rs` pin the
  filesystem logic. `permanent-delete-confirmation` covers the dialog. What is
  unique is the GUI resolving an alias pane and refreshing it. The two modes
  count, because they open the alias by different routes.
- **Criticality 4:** symlink fixes are recent and repeated: #1627 (symlinked
  folder contents counted as deleted) and #1537 (self-nesting moves through
  aliases).
- **Diagnostic 4:** short and focused. A failure says that the link, the
  target or the alias listing went wrong.

## 4. modal-dialog-blocks-window-shortcuts: 3 / 4 / 4 / 5 → 54

- **Behaviour:** with the permanent-delete confirmation open, Ctrl+K,
  Ctrl+Shift+K and Ctrl+\ open no palette, keep focus on the default button and
  leave `arrow_navigation_scoped` unchanged; Escape then closes the dialog and
  focus returns to the file.
- **Impact 3:** a keyboard user loses the dialog's focus, or a preference
  flips silently. No data is at risk: the file survives.
- **Coverage 4:** no other scenario sends window shortcuts over a modal.
  `src/ui/modal/tests.rs` covers Escape pass-through and focus restoration, not
  window accelerators. Not 5: three shortcuts take one route, through the
  window's accelerators.
- **Criticality 4:** `git log -S` on the assertion's name finds its origin in
  fix c4d858af (triage batch 5, "shortcuts"); `src/ui/modal.rs` has 18
  commits, 11 of them fixes. Only real key events through the window show it.
- **Diagnostic 5:** every check names its shortcut and what leaked
  (`ctrl+k opened a palette`).

## 5. keep-both-then-undo: 4 / 2 / 4 / 3 → 17

- **Behaviour:** as #2, but Keep Both is clicked.
- **Impact, criticality and diagnostic:** the same as #2. Duplication does not
  lower them.
- **Coverage 2:** `similar` reports 20 of its 21 steps in
  `keep-both-then-undo-remaining`, which also adds the keyboard route and
  default focus. Clicking a dialog button is an equivalent route that dozens
  of scenarios take. It is the subset of the pair, so it takes the low score
  and #2 keeps the shared coverage. Not 1: it is the only click on this
  dialog's Keep Both. Had it been coverage 1, its score would be 0 although
  its impact is 4: the dimensions show that the behaviour matters, and the
  note shows where it is covered.

## 6. toolbar-controls-named: 3 / 3 / 3 / 5 → 31

- **Behaviour:** Search (Ctrl+K), Appearance, Settings, Close window and
  Toggle sidebar (Ctrl+B) are present by accessible name.
- **Impact 3:** an unnamed control is unusable with a screen reader. This is
  product surface, not cosmetics.
- **Coverage 3:** Appearance (42 scenarios) and Settings (15) are targeted by
  name elsewhere, so they are already guarded. `enabled-tenxer-survives-restart`
  checks Close window and that Search is absent in 10xer mode. Search being
  present and Toggle sidebar are checked only here. No unit test asserts
  these names.
- **Criticality 3:** names are built from key-binding labels
  (`shortcut_reference::with_keys`), which the #1576 key catalog feeds; the
  sidebar toggle was added in fix #1606. A plausible refactor regression, but
  simple code.
- **Diagnostic 5:** each failure reads `'<name>' is unnamed`.

## 7. appearance-menu-switches-view: 3 / 2 / 3 / 4 → 14

- **Behaviour:** choose Icons from the Appearance menu; the view switches, the
  location is kept and `browser_mode = "icons"` is saved.
- **Impact 3:** the view menu breaks, but Ctrl+2 still works.
- **Coverage 2:** `appearance-menu-switches-view-list` takes the same menu
  route to List, checks the same location and persistence, and goes further.
  `shortcut-round-trip-keeps-selection` and `appearance-options-expose-chosen`
  cover the checked state. Only the `icons` value is unique. `similar` misses
  this pair (2 of 7 steps shared), because one value differs: grep for
  `"radio menu item","name":"Icons"` and `browser_mode` instead.
- **Criticality 3:** `src/ui/browser_modes` is high churn (81 commits, 36
  fixes), but the menu-to-mode wiring is simple.
- **Diagnostic 4:** seven steps. A failure says that the view did not switch,
  the location was lost, or the preference was not written.

## 8. icons-hover-baseline: 2 / 2 / 3 / 1 → 7

- **Behaviour:** with thumbnails rendered, the pointer over `todo.txt` in
  Icons matches the `icons-hover` baseline image.
- **Impact 2:** a hover rendering glitch is visible but harmless.
- **Coverage 2:** the only capture of a hovered tile. The fixture, thumbnail
  waits and view are shared with `icons-view-baseline` (5 of 7 steps). Visual
  content is capped at 2.
- **Criticality 3:** baselines have been refreshed by real fixes (#1601, glyph
  tops clipped) and broad restyles (#1680, #1688). Only a screenshot shows
  such a regression.
- **Diagnostic 1:** a failure is a percentage of differing pixels across the
  window. The diff image shows where, not why.

## 9. filter-selection-uses-theme: 2 / 2 / 3 / 3 → 10

- **Behaviour:** after Ctrl+A in the recursive filter field, more than 100
  accent-coloured pixels are inside it.
- **Impact 2:** a wrongly coloured or invisible selection highlight in one
  field; selection still works.
- **Coverage 2:** it is the only `accent_pixels` check in the corpus, but its
  unique assertion is a colour, so it is capped at 2. Theme unit tests
  (`src/ui/theme/tests.rs`) check generated palettes, not this widget.
- **Criticality 3:** its origin is fix #778, and theme colours were fixed again
  in triage batch 5. Low churn in `src/ui/browser/listing_filter.rs`.
- **Diagnostic 3:** one focused check, but a pixel-count heuristic. A failure
  could be the theme, the field's styling or render timing.

## 10. column-header-titles-bounded: 2 / 1 / 2 / 2 → 0

- **Behaviour:** for a short and a very long folder name, the column is at
  most 320 px wide, and the title sits left of the Refresh button and is
  vertically centred with it.
- **Impact 2:** the long-name bound keeps the column usable. The centring
  checks alone would be impact 1.
- **Coverage 1:** every assertion is geometry, which AGENTS.md "Test value"
  rules out ("Never write layout tests"). The `short` variant takes the same
  route as `long` and adds nothing.
- **Criticality 2:** `src/ui/browser/pane_header.rs` changes moderately (20
  commits), but layout regressions are verified with screenshots, not at this
  level.
- **Diagnostic 2:** unlabelled geometry comparisons. A failure gives two
  numbers, and the artifacts are needed to see what moved.

## Lessons the anchors encode

- Duplication lowers only coverage (#5 keeps impact 4), but coverage scales
  the whole score (#5 scores 17 against #2's 52).
- Of a duplicate pair, only the subset scores low (#2 against #5).
- Unit coverage of the logic still leaves GUI wiring to count (#3).
- Accessibility naming is behaviour (#6), not cosmetics.
- Cosmetic content caps coverage at 2, and so the score at 25 (#8, #9);
  geometry-only gets coverage 1, and so a score of 0 (#10).
- Step similarity misses one-value pairs; grep for the target (#7).
- A fix commit as origin raises criticality (#4, #9).
- A broad capture diagnoses little (#8); a check that names its subject
  diagnoses well (#4, #6).
