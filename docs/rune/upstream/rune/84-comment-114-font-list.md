# 84. Comment on #114: list the installed families

| | |
| --- | --- |
| Kind | Comment on [#114](https://github.com/anomalyco/rune/issues/114) (which #226 links) |
| Drafts | U16 (S5) |
| Affects | K2 (font picker) |
| Retires | K2's font list passed in as data |
| Checked | Re-checked at `2a44156` (2026-11-10) by source: `installed_family_files` is still `pub(super)` (`text/system_fonts.rs:149`, in the private `mod system_fonts`, `text.rs:41`), and nothing in `text` or `TextSystem` lists families (`TextSystem::families`, `text/families.rs:49`, returns the two current slots); `system_fonts.rs` is unchanged since the pin. Names resolve since `99c123d` (#245), before the pin. #114 is open with no comments (`gh issue view`, 2026-11-10); #226 (open, the Windows side) has one comment linking #114 and doesn't ask for a list. Gallery: K2's font list still comes in as data (the gallery's is a fake list, `strata-gallery/src/scenes/k2.rs:14-15`); `mise run test -- k2::` passes (4 behaviour tests plus a11y, png and scripts). Earlier: by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

From Strata (lgse/strata), a file manager on Rune: since `99c123d` (#245) a family named in a slot
resolves through fontconfig, which covers the first ask. A font picker also needs the list of
installed families to offer, and none is public: `text::system_fonts::installed_family_files` is
`pub(super)` (`text/system_fonts.rs:149`), and the lookup collection it builds isn't exposed. Could
Rune add `rune::text::installed_families() -> Vec<String>` (fontconfig on Linux, CoreText and
DirectWrite elsewhere), ideally with whether each is monospaced, so a picker can offer only
monospaced families for the mono slot?
