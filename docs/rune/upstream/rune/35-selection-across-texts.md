# 35. Selection across several texts: `select_text`, triple-click, an empty end, links

| | |
| --- | --- |
| Kind | New issue |
| Drafts | R1, R5, R6 (C11); U-C10b-6, U-C10b-7 |
| Affects | C11 (document view), C10b (code view) |
| Retires | C11's D4 (a find match across units selects its first part) and D9; C10b's D11 and D12 |
| Checked | Re-checked at `2a44156` with a new headless probe, `rv_35`, which re-runs the body's five repros: (1) `Tx::select_text(key, range)` (`el/tx.rs:407`) still takes one key and one range; (2) a triple-click on "aaa" in a group with "bbb" beside it selects "aaa"; (3) a press 80 pt right of "one"'s end dragged into "two" reports `[("two", 0..2)]` alone; (4) a double-click on a link opens it twice and selects nothing, and a Shift-click on it after a double-clicked word drops the selection and opens the link; (5) a selection in virtual row 0 scrolled 10,000 pt away is still reported whole, while the README still says it drops (`crates/rune/README.md:1512`). The README now also says a triple-click takes one text's paragraph (`:1506`), so (2) is documented behaviour; the ask stands. In the gallery, `mise run test find_reads_across_units`, `a_match_across_a_split_is_cut_to_each_row` and `on_a_link` passed (1, 1 and 2 tests: C11's and C10b's workarounds). C11's D4 and D9 (`document_view/mod.rs:85`, `:93`) and C10b's D11 and D12 (`code_view/mod.rs:101`, `:103`) are still there. Earlier: by source at `5d421fd` (`el/tx.rs:318`); the repros were C10b's and C11's probes at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Selection across texts: code can't select a range spanning two texts, a triple-click stops at one text, and a drag that starts past a line's end loses it

Context: in Strata (lgse/strata), a file manager on Rune, a document or code preview draws one
logical line or paragraph as several texts (virtual rows, a long line split for layout), inside one
selection group.

**What happens** (`5d421fd`, unchanged at `2a44156`; each repro ran headless at `2a44156`):
1. `Tx::select_text(key, range)` selects within one text, so a find match across two virtual
   rows can't be selected from code (`el/tx.rs:407` at `2a44156`).
2. A triple-click selects one text's paragraph, even inside a selection group. Repro: a
   selection group of two selectable texts "aaa" and "bbb" side by side in a row; triple-click
   "aaa": `Runtime::selected_text()` is "aaa". The README says so (`README.md:1506`), but a view
   that draws one paragraph as several texts then can't triple-click it.
3. A selection's report leaves out a start text whose selected part is empty. Repro: a group of
   two selectable texts "one" and "two" in a column; press right of "one"'s end and drag into
   "two": `TextSelection::texts()` holds "two" alone, so the line break after "one" is lost from
   the copy.
4. A press on a link inside selectable text is the link's: a double-click on it opens the link
   twice and selects no word, and a Shift-click on it drops the selection and opens the link
   instead of extending the selection.
5. The README says "a virtual row scrolling out drops the selection it holds" (`README.md:1512`
   at `2a44156`); the runtime pins both ends and keeps it (the code is right, the README isn't):
   text selected in virtual row 0 is still reported after a 10,000 pt scroll.

**Ask.**
1. A `select_text` from (key, offset) to (key, offset).
2. A way to mark texts as parts of one paragraph (or take the paragraph across texts sharing a
   row key), so a triple-click selects them all.
3. Report the start text with its empty range at its end, as the end text's range is reported.
4. A double-click or Shift-click on a link in selectable text selects, as on plain text.
5. The README matched to the runtime.

Related: (link to 36).
