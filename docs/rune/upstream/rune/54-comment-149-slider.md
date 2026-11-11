# 54. Comment on #149: what a kit Slider needs, from a file manager's sliders

| | |
| --- | --- |
| Kind | Comment on [#149](https://github.com/anomalyco/rune/issues/149) (rune-kit: no Slider component) |
| Drafts | C20a's #149 comment draft |
| Affects | C20a (slider), C13 (volume and timeline) |
| Retires | C20a's own Slider (with 53) |
| Checked | At `2a44156`: no rune-kit Slider (no `Slider` in `rune-kit/src`); #149 open, no comments (live, 2026-11-10). The Page repro is 53's, re-run at `2a44156`. Gallery: C20a's own slider is still there (`controls/slider/`). Earlier: no Slider at `04b38ae`. |
| Status | Ready to file, trimmed: dropped the `Cancel` bullet, as #149 already asks that Escape or a lost capture restore the value from before the press |

Paste from here.

---

From Strata's (lgse/strata, a file manager on Rune) settings and media sliders (GTK `GtkScale`
parity), beyond the issue's list:

- snap points with a snap distance (GTK's 12 px), and labelled snaps that act as toggles;
- a page size (Page Up and Down are always 10 steps today: (link to 53));
- a free drag beside a stepped keyboard;
- a wheel that is off by default, so the slider's scroll view keeps the wheel;
- sizes that follow the text size.
