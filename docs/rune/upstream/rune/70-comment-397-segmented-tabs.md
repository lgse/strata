# 70. Comment on #397: a tab mode for `SegmentedControl`

| | |
| --- | --- |
| Kind | Comment on [#397](https://github.com/anomalyco/rune/issues/397) (SegmentedControl segments can't carry an icon, trailing element or tooltip) |
| Drafts | U-C20c-1 |
| Affects | C20c (settings tabs), C19, C20d (category tabs) |
| Retires | C20c's, C19's and C20d's own tablists |
| Checked | Re-checked at `2a44156`. #397 is open, with no comments (read-only `gh issue view`, 2026-11-10), so the thread doesn't already say this. By source, `SegmentedControl::build` still makes `row().role("radiogroup")` of `role("radio")` segments from text labels (`rune-kit/src/segmented.rs:76`, `:120`), unchanged past the pin. Gallery: the three own tablists stand (C20c `controls/tabbed_form/view.rs:185`, C19 `footer/reference/view.rs:278`, C20d `controls/icon_picker/tabs.rs:28`); `tabbed_form::` and `c20c::` pass. Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; #397 open with an empty thread, no duplicate as of 2026-11-10) |

Paste from here.

---

Adding a related need from Strata (lgse/strata), a file manager on Rune: a tab mode.
`SegmentedControl` builds a `radiogroup` of `radio` segments from text labels
(`rune-kit/src/segmented.rs:76`), so it can't be a settings window's or a picker's page switcher:
that needs `tablist`, `tab` with `selected`, and a `tabpanel` it names. A per-segment trailing slot,
as asked here, would also carry an error dot whose meaning the segment's accessible description
names.
