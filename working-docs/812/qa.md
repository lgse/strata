# QA: #812 X11 WM_CLASS matches StartupWMClass

- **Verdict:** `pass-with-nits`
- **Round:** 1
- **Staging PR:** https://github.com/lgse/strata/pull/957 (draft; left draft)
- **Product SHA tested:** `9553be04e418a0ecefff17cff44600b3305bfde6`
- **Docs tip at QA start:** `3733e4285c7c58fb55308f59ace5e6d54192d4d5` (review.md + status only; product tree identical to `9553be04`)
- **Agent id:** `bc-4e962a5b-dc53-558c-ab3a-d774e1adeb34`

## Verdict

`pass-with-nits` — mapped file-manager `WM_CLASS` instance and class are both `io.github.lgse.Strata`, matching `StartupWMClass`. `_GTK_APPLICATION_ID` and `WM_NAME` stay `io.github.lgse.Strata` / `Strata` on the mapped window. Portal FileChooser identity stays split. E2E still finds the app after `APPLICATION_NAME` follows prgname. No product non-nits.

Did not implement, undraft, squash, merge, file issues, post a cleanup comment, or send anything to Origin.

## Coverage

| Case | Result | Evidence |
| --- | --- | --- |
| 1 Desktop `StartupWMClass` vs `APPLICATION_ID`; chooser id distinct | pass | `desktop_startup_wmclass_matches_application_id`; desktop key `StartupWMClass=io.github.lgse.Strata`; `CHOOSER_APPLICATION_ID` is `io.github.lgse.Strata.FileChooser` |
| 2 `install_application_identity()` prgname / application name | pass | `application_identity_sets_file_manager_prgname` via `gtk_test` |
| 3 X11 program class helper on private Xvfb | pass | `x11_program_class_matches_application_id` (X11 downcast + `set_program_class`; GDK 4.14 has no getter). Class-string proof is case 4 |
| 4 Mapped window `xprop` (reporter sequence) | pass | Private `:90`, never `DISPLAY=:1`. Mapped 1200×760 `IsViewable` window `0x200004` |
| 5 AT-SPI / E2E still locates the app | pass | `PATH=/tmp/docker-hostnet:$PATH STRATA_E2E_WORKERS=1 ./scripts/e2e.sh tests/e2e/scenarios/test_startup_arguments.py` — **2 passed** in 2.98s. Harness `APPLICATION_NAME` is `io.github.lgse.Strata`; window title still `Strata` |
| 6 Portal FileChooser identity unchanged | pass | `portal::window_geometry::tests` (centering regex still `^io[.]github[.]lgse[.]Strata[.]FileChooser$`). Live: portal 1×1 leader class is `io.github.lgse.Strata.FileChooser` while the mapped FM window stays `io.github.lgse.Strata` |

Targeted Rust filter (nonzero: 12 tests):

```bash
xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
  GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  cargo test --all-targets --all-features -- \
  desktop_startup_wmclass application_identity x11_program_class portal::window_geometry::tests
```

**12 passed**, 0 failed (3 new `src/tests.rs` cases + existing portal geometry/centering).

Adjacent:

- Second FM window (second directory on the same display): both 1200×760 windows advertised `WM_CLASS` / `_GTK_APPLICATION_ID` as `io.github.lgse.Strata`.
- `strata --version` still prints `strata 0.17.0` and returns before identity install.
- `tests/e2e/Dockerfile` was not given `x11-utils` (no `xprop` in the pinned image). Host `xprop` only.

Skipped (out of scope / not product fails): Wayland `app_id`; GNOME/KDE/XFCE icon screenshots; dialog/popover `WM_CLASS`; full `./scripts/e2e.sh`; E2E base rebuild.

## Findings — product non-nits

None.

## Findings — nits / process

1. **Case 4’s written `xprop -name Strata` step still hits the 1×1 client-leader first.** Leader `0x200002` has `WM_CLASS` / `WM_NAME` both correct and no `_GTK_APPLICATION_ID`. The mapped 1200×760 window has the full expected triple. Test-plan ambiguity, not a product break (review already flagged this). QA used the mapped window.
2. **PR body is still working-docs-only** (review nit). Head has the identity change; description still says this commit only adds `working-docs/812/`.

## Gaps

- GDK 4.14 has no `program_class` getter; case 3 cannot assert the class string in-process. Live mapped-window `xprop` is that proof.
- No live FileChooser *dialog* (portal process with no request only created the FileChooser 1×1 leader). Identity split is still visible on that leader vs the mapped FM window, plus centering tests.
- Full E2E suite not rerun. Planned smoke is `test_startup_arguments.py` (2 passed). Other scenarios share `APPLICATION_NAME`; residual only if that constant were wrong (it is not — startup found the app).
- Draft GitHub CI not treated as a pass. Native fmt/clippy not rerun this round (QA, not pre-push).

## Case 4 mapped window (private `:90`)

```
0x200004 "Strata": ("io.github.lgse.Strata" "io.github.lgse.Strata")  1200x760+0+0
WM_CLASS(STRING) = "io.github.lgse.Strata", "io.github.lgse.Strata"
WM_NAME(STRING) = "Strata"
_GTK_APPLICATION_ID(UTF8_STRING) = "io.github.lgse.Strata"
```

`xprop -name Strata` (first name match / leader):

```
WM_CLASS(STRING) = "io.github.lgse.Strata", "io.github.lgse.Strata"
WM_NAME(STRING) = "Strata"
_GTK_APPLICATION_ID:  not found.
```

Portal still up on the same display:

```
0x400002 "Strata": ("io.github.lgse.Strata.FileChooser" "io.github.lgse.Strata.FileChooser")  1x1+0+0
0x200004 "Strata": ("io.github.lgse.Strata" "io.github.lgse.Strata")  1200x760+0+0
```

## Head SHA tested

`9553be04e418a0ecefff17cff44600b3305bfde6`
