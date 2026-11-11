# 42. Accessibility: `aria-current`, a scroll bar role, busy, and a two-axis control

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U-C6-2, U-C10b-3, R-C14b-1, U-C20b-3 |
| Affects | C6 (breadcrumbs), C10b and C24 (scroll bars), C14b (3D model view), C20b (colour plane) |
| Retires | C6's current crumb as a label last in the list, C10b's position bar read as a slider, C14b's spinner label standing for busy |
| Checked | Re-checked at `2a44156` by source: `role("scrollbar")` is accepted (`el/rare.rs:206`) but `a11y/roles.rs` still maps no `"scrollbar"`, so it falls to a button or group (`:103`); the role table maps `"slider"` only (`:38`). There is still no `El::current`, `El::busy` or other builder for `aria-current` or AccessKit's busy (no match in `el/` or `a11y/`), and no per-axis value for a two-axis control. The `a11y/` changes since the pin (4 files, 5 lines) add none. In the gallery, C10b's D5 (the position bar reads as a slider, `code_view/mod.rs:88`, `:151`) and C20b's U-C20b-3 (`controls/color_picker/mod.rs:18`, `:159`) are still there, and `mise run test assistive` passed (the C10 bar and C20b strip tests among them). Earlier: by source at `5d421fd` and `04b38ae`. No program needed. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Accessibility: no `aria-current`, no scroll bar role, no busy state, and no way to expose a two-axis control

Context: Strata (lgse/strata), a file manager on Rune.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
- **Current item.** No `aria-current`, so the current breadcrumb or the menu's current folder
  can't say so.
- **Scroll bar.** `role("scrollbar")` is accepted, but `a11y/roles.rs` maps no `"scrollbar"`, so
  it reads as a group (or a button, if pressable), and a scroll bar that has to carry its value
  is given `"slider"` instead and reads as a slider.
- **Busy.** No builder sets AccessKit's busy, so a picture being rendered is heard only through
  a spinner's label.
- **Two axes.** A saturation and value plane can only be one slider with a value text: no range,
  set-value or increments per axis.

**Ask.** `El::current(kind)`; `"scrollbar"` mapped to `Role::ScrollBar` with the range value and
steps a slider has; `El::busy(bool)`; and guidance (or a builder) for a two-axis control, such as
two child value nodes on one focus stop.

Related: #111.
