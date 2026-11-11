# 72. A scroll offset survives an instant removal, but not a faded one

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C20c-4 |
| Affects | C20c (rebuilt pages) |
| Retires | C20c's per-visit key for rebuilt pages |
| Checked | Re-run at `2a44156` as probe `rv_72` (C20c's P2r); the same program at `5d421fd` prints the same. A column wheeled to 200 comes back at 200 when it was removed under `instant_changes` on its row, or in a `Tx::instant` commit. It comes back at 0 with the default fade, and with `instant_changes` on the column itself, whose own exit keeps its fade (`el/motion.rs:176–181`). Gallery: C20c's per-visit key stands (`controls/tabbed_form/view.rs:209`); `a_kept_page_keeps_its_scroll_offset_and_a_rebuilt_one_starts_at_the_top` passes. Earlier: C20c's probe at the pin. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** A scroll viewport removed in an `instant_changes` commit comes back at its old offset; one removed with a fade comes back at 0

Context: Strata (lgse/strata), a file manager on Rune.

**Repro** (headless, `2a44156`; the same at `5d421fd`): a row with `instant_changes` holding one
of two keyed scroll columns by a flag; wheel the first down 200, flip the flag, run 40 frames,
flip it back: its content's offset is 200. The same in a `Tx::instant` commit. Without
`instant_changes`, or with it on the column only: 0.

**Ask.** One rule: a removed viewport forgets its offset (or an explicit keep).
