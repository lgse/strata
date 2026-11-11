# 57. rune-kit: export the Combobox fuzzy matcher, with match positions

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U17 (S5, from W1) |
| Affects | W1's `contract::fuzzy`, C17, C19, C20c, C20d |
| Retires | The port of `score` that also records matched ranges |
| Checked | At `2a44156`, by source: `fn score` is still private (`rune-kit/src/combobox.rs:108`) and returns only a score; nothing fuzzy is exported (`rune-kit/src/lib.rs`). No program needed. Gallery: the port is still there (`contract/matched.rs:176`); the `matched` tests pass. Earlier: private at `5d421fd` (`:44`) and `04b38ae` (`:108`, after `1ea05bc7`). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** rune-kit: export the Combobox fuzzy matcher, with match positions

Strata's (lgse/strata, a file manager on Rune) command palette, settings search and shortcut
reference rank results as `Combobox` does and bold the matched characters. rune-kit's ranking is the
private `combobox::score` (`rune-kit/src/combobox.rs:108` at `2a44156`). It returns a score but not
which characters matched, so apps copy it to rank the same way.

**Ask.** A public `rune_kit::fuzzy_match(query, text) -> Option<FuzzyMatch { score, ranges }>`,
the one Combobox uses, with the matched character ranges, so a list filtered with it orders
exactly as a Combobox does.
