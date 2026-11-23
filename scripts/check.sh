#!/usr/bin/env bash
set -euo pipefail

cargo fmt --all --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Run tests headless: no X display, and a Wayland socket and session bus that do not exist.
# Built without debug info to keep links small. strata-gallery's GPU integration tests are on
# demand (`mise run test-gallery`), so only its unit tests run here.
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
headless() {
  env -u DISPLAY -u WAYLAND_SOCKET WAYLAND_DISPLAY=strata-test-no-wayland \
    DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent/strata-test-no-bus \
    GIO_USE_VFS=local \
    CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \
    RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
    HOME="$tmp" \
    XDG_CONFIG_HOME="$tmp/config" \
    XDG_DATA_HOME="$tmp/data" \
    XDG_CACHE_HOME="$tmp/cache" \
    XDG_STATE_HOME="$tmp/state" \
    STRATA_REQUIRE_DEVICE_TESTS=1 \
    CARGO_PROFILE_DEV_DEBUG=0 \
    cargo test "$@"
}
headless --workspace --exclude strata-gallery --all-targets --all-features
headless --package strata-gallery --lib --bins --examples --all-features

if command -v cargo-deny >/dev/null 2>&1; then
  cargo deny --workspace check
else
  echo "note: cargo-deny is not installed; skipping dependency policy checks" >&2
fi

if command -v typos >/dev/null 2>&1; then
  typos
else
  echo "note: typos is not installed; skipping spell checks" >&2
fi
