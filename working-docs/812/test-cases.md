# Test cases: #812 X11 WM_CLASS

Narrow set. Do not add an exhaustive desktop-shell matrix.

## 1. Desktop file stays aligned with APPLICATION_ID

- **Purpose:** `StartupWMClass` does not drift from the id the binary advertises.
- **Setup:** none (source fixture).
- **Steps:** Read `data/io.github.lgse.Strata.desktop`. Assert `StartupWMClass=` equals `APPLICATION_ID` (`io.github.lgse.Strata`). Assert `CHOOSER_APPLICATION_ID` remains a different string.
- **Expected:** Desktop key is `io.github.lgse.Strata`. Portal id is still `io.github.lgse.Strata.FileChooser`.
- **Automation:** `src/tests.rs` (include_str desktop file; compare to `crate::APPLICATION_ID` / `crate::portal::CHOOSER_APPLICATION_ID`).

## 2. Application launch installs file-manager prgname before run

- **Purpose:** GDK instance hint is `APPLICATION_ID`, not `strata`.
- **Setup:** isolated GTK child (`test_support::gtk_test`) so `glib::prgname` is not shared with other tests.
- **Steps:** Call `install_application_identity()`. Read `glib::prgname()` and `glib::application_name()`.
- **Expected:** prgname is `io.github.lgse.Strata`. Application name is `Strata`.
- **Automation:** `src/tests.rs` via `gtk_test`. Do not assert this from `--portal` / `--version` paths.

## 3. X11 program class matches StartupWMClass

- **Purpose:** The class string (second `WM_CLASS` field) is not the capitalized prgname.
- **Setup:** private Xvfb, `GDK_BACKEND=x11`, `gtk_test` after identity install; downcast default display to `gdk4_x11::X11Display` and call the same `set_program_class` the startup handler will use.
- **Steps:** Assert X11 `program_class` is `io.github.lgse.Strata`. If the display is not X11, fail under `STRATA_REQUIRE_GTK_TESTS=1` (do not skip).
- **Expected:** class == `APPLICATION_ID`.
- **Automation:** `src/tests.rs` gtk_test. This is the GDK input GTK copies into `WM_CLASS`; do not add `xprop` to the E2E image.

## 4. Mapped window WM_CLASS (reporter sequence)

- **Purpose:** Live proof of the filed `xprop` sequence.
- **Setup:** private Xvfb (never `DISPLAY=:1`), isolated HOME/XDG, debug `strata` binary, `GDK_BACKEND=x11`.
- **Steps:**
  1. Launch Strata.
  2. Wait until a window titled `Strata` is mapped.
  3. `xprop -name Strata WM_CLASS _GTK_APPLICATION_ID`
- **Expected:** `WM_CLASS` instance and class are both `io.github.lgse.Strata`. `_GTK_APPLICATION_ID` stays `io.github.lgse.Strata`. Window name stays `Strata`.
- **Automation:** code-stage evidence on the Cloud VM (`xprop` is on this host). Not an `e2e.sh` case: the pinned E2E image has no `xprop`.

## 5. AT-SPI application name still locatable

- **Purpose:** E2E harness does not lose the process after prgname changes.
- **Setup:** existing E2E `strata` fixture (private Xvfb + AT-SPI).
- **Steps:** Start Strata. Resolve the accessible application. Open one directory (any existing scenario smoke).
- **Expected:** The AT-SPI application node is found. If the name changed from `strata`, `APPLICATION_NAME` in `tests/e2e/harness/application.py` is the new id and startup still waits for the window.
- **Automation:** one existing file is enough, e.g. `tests/e2e/scenarios/test_startup_arguments.py` or `test_locations.py` after the harness constant is fixed. Do not add a new scenario solely to print the name.

## 6. Portal identity is unchanged

- **Purpose:** FileChooser Hyprland class does not become the file-manager id.
- **Setup:** none beyond existing portal tests.
- **Steps:** Re-run the centering tests that assert `CHOOSER_APPLICATION_ID != APPLICATION_ID` and that the Hyprland class regex is `^io[.]github[.]lgse[.]Strata[.]FileChooser$`.
- **Expected:** unchanged.
- **Automation:** `src/portal/window_geometry/tests/centering.rs` (already present). Do not duplicate.

## Out of scope

- Wayland `app_id` / `WAYLAND_DEBUG` grep.
- Per-desktop icon screenshots (GNOME/KDE/XFCE).
- Dialog/popover `WM_CLASS`.
- Rebuild of the E2E base image.
