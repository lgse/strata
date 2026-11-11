#!/usr/bin/env bash
set -euo pipefail

# The GTK application and the Rune side build in separate cargo runs (docs/workspace.md).
scripts="$(dirname "${BASH_SOURCE[0]}")"
gtk="$(python3 "$scripts/lanes.py" gtk)"
rune="$(python3 "$scripts/lanes.py" rune)"

cargo fmt --all --check
# shellcheck disable=SC2086 # each lane is a list of cargo arguments
for lane in "$gtk" "$rune"; do
  cargo check $lane --all-targets --all-features
  cargo clippy $lane --all-targets --all-features -- -D warnings
done
# Display-dependent tests are exercised by test-headless.py and qa.sh.
# shellcheck disable=SC2086
env -u DISPLAY -u WAYLAND_DISPLAY GDK_BACKEND=x11 STRATA_REQUIRE_DEVICE_TESTS=1 cargo test $gtk --all-targets --all-features
# The Rune tests are headless: no X display, and a Wayland socket and session bus that do not
# exist. They build without debug info, which keeps the links of their many binaries small.
# shellcheck disable=SC2086
env -u DISPLAY -u WAYLAND_SOCKET WAYLAND_DISPLAY=strata-test-no-wayland \
  DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent/strata-test-no-bus \
  CARGO_PROFILE_DEV_DEBUG=0 \
  cargo test $rune --all-targets --all-features

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
