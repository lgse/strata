# Code notes: #537 unlock prompt ownership (round 1)

## Investigation (PARTIAL)

This Cloud VM still cannot prove competing-prompt ownership:

- Display: XFCE/X11 (`DISPLAY=:1` TigerVNC). `WAYLAND_DISPLAY` unset. Not Omarchy/Hyprland.
- Nautilus is not installed. `thunar-volman` is not installed. `Thunar` and `gvfsd` are running.
- Device-mapper is not usable (`lsmod` has no `dm_mod` / `dm_crypt`).
- Isolated Strata overlay already works on current main (issue comment on 2026-09-10 and GTK coverage here).

Did not attach a LUKS loop to the live XFCE session (never `DISPLAY=:1` for GTK). Did not install Nautilus. Attach-time vs sidebar-click process owners remain unverified. Finding A is not claimed. Finding C (`Unhandled` when `ModalHost::blurred_for` fails) is unchanged: a presented window already hosts the overlay.

## Product change (Finding B/D)

Smallest matching fix: treat an in-flight foreign mount as wait/navigate, not `Unable to mount volume`.

On Devices sidebar click, `mount_device_volume` still calls `volume.mount` with Strata’s `gtk::MountOperation`. If the result is `Pending` / `Busy`, or the message contains `already unlocking` / `already in progress`, wait up to 8s for `volume-changed`, `removed`, or `get_mount()`. Then:

- mount present → navigate (same as `AlreadyMounted` / late `get_mount()`)
- still locked → start a Strata-owned mount (one wait already done)
- volume gone, or still in-flight after that owned remount’s wait → quiet (no error dialog)

Does not cancel another app’s mount job. Does not disable automount. Overlay submit/cancel/retry for Strata-owned `ask-password` is unchanged. `NotSupported` (`Operation not supported` / missing `dm_mod`) stays a terminal error. Passphrase rejects still retry.

## Files

- `src/ui/browser/location.rs` — classification, bounded wait, remount
- `src/ui/browser/location/tests.rs` — pending/busy vs `NotSupported`; `AlreadyMounted` / late mount; wait follow-up

## Tests run

Private Xvfb; never `DISPLAY=:1`. Host needed GStreamer `-dev` and a `g++` wrapper that drops `-stdlib=libc++` so `unrar_sys` links.

```bash
cargo fmt --all --check
PATH="/tmp/cxx-nolibcxx:$PATH" CC=gcc CXX=/tmp/cxx-nolibcxx/c++ \
  RUSTFLAGS="-C link-arg=-L/usr/lib/gcc/x86_64-linux-gnu/13" \
  cargo clippy --all-targets --all-features -- -D warnings
xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
  GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  CC=gcc CXX=/tmp/cxx-nolibcxx/c++ PATH="/tmp/cxx-nolibcxx:$PATH" \
  RUSTFLAGS="-C link-arg=-L/usr/lib/gcc/x86_64-linux-gnu/13" \
  cargo test --all-targets --all-features -- ui::browser::location::tests
```

- fmt: pass
- clippy `-D warnings`: pass
- `ui::browser::location::tests`: **13 passed**, 0 failed (includes overlay submit/cancel, quiet cancel, new pending/busy and external-completion cases)

Omitted: full `./scripts/quality.sh` (fmt/clippy run native; no `target/quality-container` cache), `./scripts/e2e.sh` (wait is local to `location.rs`, not shared VolumeMonitor/sidebar rebuild). `cargo-deny` / `typos` are not installed; not treated as a CI pass.

## Remaining gaps

- Manual Wayland cases 6–9 in `test-cases.md` (attach ownership, automounter present/absent, external unlock while Connecting).
- No LUKS E2E. No real credentials. Fixture passphrase in docs: `fixture-passphrase-537`.
