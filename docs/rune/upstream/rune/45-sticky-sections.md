# 45. Section headers that stick across virtual rows

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C1f-4, R-K4 |
| Affects | C1e/C1f (grouped collections), C4 (grouped list table) |
| Retires | C1f's stuck-header copy drawn over the viewport's top line, pushed by a live value |
| Checked | Re-checked at `2a44156`: the body's repro, run as a probe (`repros-head/src/bin/rv_45.rs`: 10,000 virtual rows in sections of 1,000, each section's first row holding a `.sticky_top(0.0)` header, scrolled 20 rows into section 1), prints `h0` shown at the top and, inside section 1, no header at all (the top row is `r1018`). By source, `El::sticky_top`'s docs still say a header in `virtual_rows` sticks inside its own row (`el/sticky.rs:41–44`), as the README's "Sticky headers" does (`crates/rune/README.md:382–383`). `El::stuck_fade` (`59e7c017`, #268, before the pin) and `El::stuck_reveal` (`c9cd9df3`, #399, past the pin; `el/sticky.rs:100`, `:109`) only style a stuck header's children; neither lets a header stick past its row. In the gallery, `mise run test stuck` passed (9 C1e/C1f tests and C4's `the_top_groups_header_stays_stuck_while_the_list_scrolls`), and C1f's stuck-header copy over the viewport's top line is still there (`collection/element/mod.rs:98–106`, `:154`). Earlier: by source at `5d421fd`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Sticky section headers across `virtual_rows`

Context: Strata (lgse/strata), a file manager on Rune.

A sticky element sticks only within its parent, and a virtual list's rows are the parents, so a
header sticks only if its whole section is one row (README, "Sticky headers"). For a section of
thousands of rows that defeats virtualization.

**Repro** (headless; ran at `2a44156`): `virtual_rows` of 10,000 one-line rows in sections of
1,000, each section's first row holding a `.sticky_top(0.0)` header; scroll 20 rows into a
section: the header scrolls away with its row, and no header shows. `El::stuck_fade` and
`El::stuck_reveal` style a stuck header's children, but don't change where it can stick.

**Ask.** Sections of a virtual list (a section start per index, or a header builder) whose
header sticks while any of the section's rows show, pushed up by the next.
