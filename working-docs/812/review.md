# Review: #812 X11 WM_CLASS matches StartupWMClass

- **Verdict:** `approve-with-comments`
- **Round:** 1
- **Staging PR:** https://github.com/lgse/strata/pull/957 (draft)
- **Head SHA reviewed:** `9553be04e418a0ecefff17cff44600b3305bfde6`
- **Agent id:** `bc-009b4098-4ea7-534b-b4d5-fbb908e67c94`

## Verdict

`approve-with-comments` — file-manager identity is `io.github.lgse.Strata` for GLib prgname (before `run()`) and X11 program class (`connect_startup` downcast). Portal FileChooser identity is unchanged. E2E `APPLICATION_NAME` follows prgname. Added tests match the planned set and are not excess/tautological given GDK 4.14 has no `program_class` getter.

GitHub format/lint/test and E2E were skipped (draft). Metadata policy failed on agent-authored commits; William will resign/sign. Did not rebase, amend, undraft, squash, drop working-docs, merge, or post a GitHub PR comment. Did not send anything to Origin.

## Blockers

None.

Plan alignment on `9553be04`:

- `install_application_identity()` runs only after the launch-mode match, on `LaunchMode::Application`. `--portal`, `--preview-helper`, `--gvfs-probe`, `--version`, and portal-setup flags still return first. Portal still sets `CHOOSER_APPLICATION_ID` in `portal::run()`.
- `glib::set_prgname(Some(APPLICATION_ID))` and `glib::set_application_name("Strata")` are before `gtk::Application::run()`.
- `install_x11_program_class()` fail-opens on missing/non-X11 displays; X11 calls `set_program_class(APPLICATION_ID)`.
- Desktop `StartupWMClass` and FileChooser id are untouched. `gdk4-x11` is `0.11.4` with `v4_10` (crate has no `v4_12`). E2E Dockerfile was not given `x11-utils`.
- E2E `APPLICATION_NAME` is `io.github.lgse.Strata` (one harness constant; window title still `Strata`).

Tests scanned:

1. `desktop_startup_wmclass_matches_application_id` — desktop-file drift vs `APPLICATION_ID`; `CHOOSER_APPLICATION_ID` stays distinct. Planned case 1.
2. `application_identity_sets_file_manager_prgname` — GLib round-trip of prgname and application name via `gtk_test`. Planned case 2. Would fail if identity were skipped or used the wrong id.
3. `x11_program_class_matches_application_id` — X11 downcast plus the startup helper. GDK 4.14 has no `program_class` getter; class-string proof is the mapped-window `xprop` in `code-notes.md`, not this gtk_test. Not treated as tautological given that constraint. Planned case 3 shape.
4. No new E2E scenario; `test_startup_arguments.py` reuses the harness constant (case 5). Portal centering tests were not duplicated (case 6).

## Nits / suggestions

1. **PR body is still working-docs-only.** Description says this commit adds `working-docs/812/` with no product code. Head has the identity change, `gdk4-x11`, tests, and the AT-SPI constant.
2. **`status.md` at this SHA listed `77be0a30`.** That is the code-notes commit; `9553be04` only recorded that SHA. One commit behind the reviewed tip (same pattern as other pipeline status-sync commits).

## Notes / residual risk

- `xprop -name Strata` can hit the 1×1 client-leader first. Leader has the same `WM_CLASS` / `WM_NAME` and no `_GTK_APPLICATION_ID`. The mapped 1200×760 file-manager window has all three. QA should target that mapped window, not the first `Strata` name match.
- Live `xprop` on private `:90` (code-notes): `WM_CLASS` instance and class both `io.github.lgse.Strata`; `_GTK_APPLICATION_ID` and `WM_NAME` unchanged.
- Full `./scripts/e2e.sh` was not run; AT-SPI change is one constant. Targeted `test_startup_arguments.py` (2 passed) is the planned smoke. Residual: other scenarios share `APPLICATION_NAME` and were not re-run here.
- Draft CI skipped. Did not re-run full local CI. Code-notes recorded fmt, clippy `-D warnings`, 12 targeted Rust tests, and the startup E2E file.
- `mergeable_state: blocked`. Left draft.

## Head SHA reviewed

`9553be04e418a0ecefff17cff44600b3305bfde6`
