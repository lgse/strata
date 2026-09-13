# Plan: X11 WM_CLASS matches StartupWMClass

Issue: [#812](https://github.com/lgse/strata/issues/812)
Labels: `bug`, `P3` (unchanged; do not retitle or relabel)
Confirmed: filed on 0.15.0 (`66fb0e6`); `src/main.rs` on `lgse/strata` `main` (`63b6b80`) still has no `set_prgname` on the file-manager launch path. No comments. No linked PR.

## Goal

On X11, the mapped file-manager window’s ICCCM `WM_CLASS` instance and class must both be `io.github.lgse.Strata`, matching `StartupWMClass` in `data/io.github.lgse.Strata.desktop`. Shells that key off `WM_CLASS` (not `_GTK_APPLICATION_ID`) can then associate the window with the desktop file and icon.

## Research

### Reported behavior

`xprop -name Strata WM_CLASS _GTK_APPLICATION_ID` on a private Xvfb:

    WM_CLASS(STRING) = "strata", "strata"
    WM_NAME(STRING) = "Strata"
    _GTK_APPLICATION_ID(UTF8_STRING) = "io.github.lgse.Strata"

Desktop file already has `StartupWMClass=io.github.lgse.Strata`. Wayland `app_id` is already the GtkApplication id; this bug is X11 `WM_CLASS`.

### Code

- `APPLICATION_ID` is `io.github.lgse.Strata` (`src/main.rs`). `gtk::Application` is built with that id and windows get `_GTK_APPLICATION_ID`. GDK still derives X11 `WM_CLASS` from `g_get_prgname()` (instance) and the X11 display program class (class). Default prgname is the basename of `argv[0]` → `strata`.
- File-chooser portal already sets identity before GTK init (`src/portal.rs`): `glib::set_prgname(Some(CHOOSER_APPLICATION_ID))` and `glib::set_application_name("Strata")`, with `CHOOSER_APPLICATION_ID = io.github.lgse.Strata.FileChooser`. Hyprland centering matches only that class (`src/portal/window_geometry.rs`). Do not change that split.
- `g_application_run` sets prgname from `argv[0]` only if it is not already set. Call `set_prgname` on the Application arm before `application.run()`.
- GTK 4 removed `gtk_window_set_wmclass`. The X11 class string is `gdk_x11_display_set_program_class`. Default class is typically the prgname with the first letter capitalized (`Io.github.lgse.Strata`), which would not match `StartupWMClass`. Set the program class to `APPLICATION_ID` once an X11 `GdkDisplay` exists (`Application::connect_startup` already used for FileManager1). Wayland: skip (same pattern as `gdk4_wayland::WaylandToplevel` in `src/ui/chooser.rs`).
- `Cargo.toml` already depends on `gdk4-wayland` `0.11.4`. Add `gdk4-x11` at the same version for the X11 downcast only. Do not introduce raw Xlib or restore deprecated GTK 3 APIs.

### Callers

- E2E finds the process with AT-SPI name `strata` (`tests/e2e/harness/application.py` `APPLICATION_NAME`). AT-SPI often uses prgname. After `set_prgname(APPLICATION_ID)` the desktop child name may become `io.github.lgse.Strata`. Confirm on a private Xvfb and update the harness if it changes. Window title stays `Strata`.
- `--portal`, `--preview-helper`, `--gvfs-probe`, `--version`, and portal setup flags must not inherit the file-manager prgname. Identity belongs only on `LaunchMode::Application`.
- Default icon already uses `APPLICATION_ID` (`src/assets.rs`). Desktop install path (`README.md`, `mise.toml`, packaging) is already `io.github.lgse.Strata.desktop`. No preference, theme token, or icon work.

### Precedents

- Portal identity: `set_prgname` + `set_application_name` before GTK init.
- Backend-specific GDK: Wayland-only downcast in `apply_external_parent`; X11-only `set_program_class` should fail open on non-X11 displays.
- `src/tests.rs` already covers `launch_mode` (including `--version` on current `main`). Keep identity tests there.

## Approach

Smallest fix: advertise the same application id GTK already uses, on the X11 class hints.

1. Extract `install_application_identity()` in `src/main.rs`: `glib::set_prgname(Some(APPLICATION_ID))` and `glib::set_application_name("Strata")`. Call it only on the Application path, after the launch-mode match and before `gtk::Application` `run()`.
2. In the existing `connect_startup` handler (or immediately after display init on that path), if `gdk::Display::default()` downcasts to `gdk4_x11::X11Display`, call `set_program_class(APPLICATION_ID)` so both `WM_CLASS` strings are `io.github.lgse.Strata`.
3. Leave `data/io.github.lgse.Strata.desktop` `StartupWMClass` as-is. Do not change the portal prgname.
4. If AT-SPI’s application name follows prgname, point `APPLICATION_NAME` at `io.github.lgse.Strata` (one harness constant). Do not retarget by window title.

No `settings.toml` field. No window-local preference. No new Lucide assets.

## Constraints

- GTK 4.12+ bindings / 4.14 baseline. Private Xvfb for GTK tests (`STRATA_REQUIRE_GTK_TESTS=1`). Never `DISPLAY=:1`.
- Tests: module tests in `src/tests.rs` (already declared from `main.rs`). Do not put tests inline in `main`. E2E harness change only if AT-SPI identity moves.
- Do not add `x11-utils` to `tests/e2e/Dockerfile` (pinned image). Canonical `e2e.sh` has no `xprop`.
- `gdk4-x11` is an X11 display downcast, not a new sandbox/media runtime.

## Risks

- AT-SPI / E2E: changing prgname can rename the accessible application. Confirm before calling the suite green; one harness constant is the expected fix, not a scenario rewrite.
- Program class vs instance: `set_prgname` alone can leave class as `Io.github.lgse.Strata`. The issue asks for shells that match class name — set both hints.
- Startup order: `set_prgname` after `run()` is too late (`g_application_run` would already have filled it from `argv[0]`). Keep it before `run()`. `set_program_class` needs a live X11 display; `startup` is the right hook.
- Portal process: separate binary invocation (`--portal`) keeps FileChooser identity. Do not call `install_application_identity()` from `portal::run`.

## Non-goals

- Wayland `app_id` (already `APPLICATION_ID` via GtkApplication).
- Changing `StartupWMClass`, desktop filename, D-Bus names, or the FileChooser id.
- Matching `WM_CLASS` on dialogs/popovers beyond the toplevel file-manager window.
- Adding `xprop` to the E2E image or rebuilding the E2E base.
- Closing leftover #597 (List/Columns leftover already shipped; not this issue).
