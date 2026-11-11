# 95. Whether a `Pixels` is drawn

| | |
| --- | --- |
| Kind | New issue (low priority) |
| Drafts | U-C25-2 |
| Affects | C25 (thumbnail store's recency) |
| Retires | C25's recency approximated from builds and visible ranges |
| Checked | Re-checked at `2a44156` by source: `Pixels`' public API is still `new`, `source_url`, `source_set`, `set` and `size` (`pixels.rs:50–118`), with no draw accessor; the atlas now keeps when an animated image was first drawn (`ImageAtlas::starts`, `pixels.rs:243–245`), but only inside the renderer. Gallery: C25's recency from builds and visible ranges is still there (`crates/strata-components/src/thumbnails/mod.rs:200–203`). No program needed. Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** `Pixels` can't say whether or when it was last drawn

Context: Strata (lgse/strata), a file manager on Rune.

A thumbnail cache evicts what hasn't been seen for longest, but an app can't tell whether a
`Pixels` handle was drawn, so recency is approximated from builds and visible ranges.

**Ask.** `Pixels::last_drawn()` (a frame number) or `is_shown()`.
