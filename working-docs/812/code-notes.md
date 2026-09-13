# Code notes: #812 X11 WM_CLASS

Round 1. Agent `bc-1d72a758-95b9-586c-99da-6499ca2fc811`.

## What changed

File-manager launch now advertises `io.github.lgse.Strata` as both X11 `WM_CLASS` strings so they match `StartupWMClass`. The FileChooser portal identity is unchanged.

- `install_application_identity()` sets `glib::set_prgname(Some(APPLICATION_ID))` and `glib::set_application_name("Strata")` only on `LaunchMode::Application`, after the launch-mode match and before `gtk::Application::run()`.
- `install_x11_program_class()` downcasts the default display to `gdk4_x11::X11Display` and calls `set_program_class(APPLICATION_ID)`. Non-X11 displays return without error.
- E2E `APPLICATION_NAME` is `io.github.lgse.Strata` because AT-SPI followed prgname.

`gdk4-x11` is `0.11.4` with feature `v4_10` (highest feature at or below the GTK 4.14 baseline). The crate has no `v4_12` feature; `xlib` is not enabled.

## Files

- `Cargo.toml` / `Cargo.lock` — `gdk4-x11` 0.11.4
- `src/main.rs`
- `src/tests.rs`
- `tests/e2e/harness/application.py`

## Tests

Private Xvfb; never `DISPLAY=:1`. Host needed `libgstreamer*-dev` and `/tmp/cxx-nolibcxx` (`g++`, strip `-stdlib=libc++`) plus gcc `libstdc++` `RUSTFLAGS` for `unrar_sys`.

```bash
cargo fmt --all --check
PATH="/tmp/cxx-nolibcxx:$PATH" CC=gcc CXX=/tmp/cxx-nolibcxx/c++ \
  RUSTFLAGS="-C link-arg=-L/usr/lib/gcc/x86_64-linux-gnu/13" \
  cargo clippy --all-targets --all-features -- -D warnings
xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
  GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  PATH="/tmp/cxx-nolibcxx:$PATH" CC=gcc CXX=/tmp/cxx-nolibcxx/c++ \
  RUSTFLAGS="-C link-arg=-L/usr/lib/gcc/x86_64-linux-gnu/13" \
  cargo test --all-targets --all-features -- \
  desktop_startup_wmclass application_identity x11_program_class portal::window_geometry::tests
PATH="/tmp/docker-hostnet:$PATH" STRATA_E2E_WORKERS=1 \
  ./scripts/e2e.sh tests/e2e/scenarios/test_startup_arguments.py
```

- fmt: pass
- clippy `-D warnings`: pass
- targeted Rust filter: **12 passed**, 0 failed (nonzero; new `src/tests.rs` cases plus existing portal centering)
- `test_startup_arguments.py`: **2 passed** in 3.01s (AT-SPI name changed)

Case 4 live `xprop` on private `:90` for the mapped 1200x760 window:

```
WM_CLASS(STRING) = "io.github.lgse.Strata", "io.github.lgse.Strata"
_GTK_APPLICATION_ID(UTF8_STRING) = "io.github.lgse.Strata"
WM_NAME(STRING) = "Strata"
```

GDK 4.14 has no `program_class` getter, so the gtk_test asserts an X11 downcast and calls the startup helper; the mapped-window `xprop` is the class-string proof.

Omitted: container `./scripts/quality.sh` (native fmt/clippy instead), full `./scripts/e2e.sh` (AT-SPI change is one harness constant; startup smoke is the planned file). `cargo-deny` / `typos` not installed; not treated as a CI pass.

## Gaps

- `xprop -name Strata` can hit the 1x1 client-leader window first. That window has the same `WM_CLASS` and `WM_NAME` but no `_GTK_APPLICATION_ID`. The mapped 1200x760 file-manager window has all three properties.
- `ManagePullRequest` cannot update `lgse/strata#957` from this fork checkout (`PR URL must belong to the current repository`). Left draft. Did not open another PR, merge, squash, or post a cleanup comment.
