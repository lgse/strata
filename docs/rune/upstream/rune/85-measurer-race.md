# 85. A thread's measurer can keep the wrong fonts under the new fonts' generation

| | |
| --- | --- |
| Kind | New issue |
| Drafts | R-K6-1 |
| Affects | K6 (font tests), strata-kit and gallery test layout |
| Retires | The rule that a test needing the measure path's fonts lives alone in its own test binary |
| Checked | Re-checked at `2a44156` by source: `with_text` still builds the measurer, then reads `fonts_generation` again with no lock held across the build (`text/measure.rs:233–239`), and the families-only branch keeps the loaded fonts on a generation match (`:228–231`). Gallery: K6's weight test still lives alone in `crates/strata-kit/tests/fonts_weight.rs` and passes (`mise run test the_wght_axis_follows_the_weight`, 1 test). Not run as a race (flaky by nature, about one run in six). Earlier: by source at `5d421fd` (`:227`) and `04b38ae` (`:238`). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `with_text`'s measurer can be tagged with a newer fonts generation than the fonts it loaded

Context: Strata (lgse/strata), a file manager on Rune.

`with_text` (`text/measure.rs` at `2a44156`) builds a thread's measurer from the fonts it read,
then reads the state's `fonts_generation` again (line 238) to tag it, with no lock held across
the build. A `TextSystem::new` with other fonts on another thread in between leaves the measurer
holding the old fonts under the new generation. Later, the families-only branch (line 228) sees
the generation match and keeps the loaded fonts, changing only the families: text measures in
the old fonts under the new names (in our tests, about one run in six measured 258 pt where 600
was due).

**Repro sketch** (headless, a loop of fresh processes): thread A takes `with_families("F", "F")`
in a process with no text system yet, while thread B calls `TextSystem::new(&[FONT_F], ("F",
"F"))`. A then calls `TextSystem::new(&[FONT_F], ("F", "F"))` and `rune::text::advance` on a run
whose width tells F from Inter: some runs measure Inter.

**Ask.** Tag the measurer with the generation it read the fonts at, or hold the state's lock from
the read through the tag.
