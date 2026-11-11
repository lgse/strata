# 66. rune-kit `HoverCard`: placements, gap, width, hover on the surface, a role and name

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C22-2 |
| Affects | C22 (folder peek) |
| Retires | C22's computed anchor and its overhang deviation |
| Checked | Re-checked at `2a44156` by source. `Placement` (`rune-kit/src/overlay.rs:16`) still has no LeftStart, LeftEnd or RightEnd. `HoverCard::build` (`overlay.rs:554`, `:583`) still builds a 200–320 pt card 4 pt off, with no hover handler, role or label. Past the pin `overlay.rs` changed only in `TooltipStyle::help_look`'s fade and row gap. Gallery: C22's accessibility goldens still hear `c22-list-peek-card` as an unnamed `group` under `kit-root`; `c22::` (a11y, png, scripts) and `folder_peek::` pass. Earlier: by source at `5d421fd` (`overlay.rs:16, 550`) and `04b38ae`; C22 re-checked at `26769da`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** rune-kit `HoverCard`: no LeftStart, LeftEnd or RightEnd, a fixed gap and width, no hover on the surface, and no role or name

Context: Strata (lgse/strata), a file manager on Rune.

At `2a44156`, `Placement` (`rune-kit/src/overlay.rs:16`) has no LeftStart, LeftEnd or RightEnd;
the card's gap is 4 pt and its width 200 to 320 pt; the surface takes no hover handler; and
the card has no role or name: assistive technology hears an unnamed group
(`kit-root/{key}-card`, role `group`).

**Ask.** Those placements, a gap and width, hover on the surface, and a role and label (as
Kobalte's `HoverCard.Content` takes ARIA attributes).
