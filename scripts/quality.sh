#!/usr/bin/env bash
set -euo pipefail

unset DISPLAY WAYLAND_DISPLAY NOTIFY_SOCKET DBUS_SESSION_BUS_ADDRESS DBUS_STARTER_ADDRESS
repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
phase="${1:-all}"
case "$phase" in all|fmt|clippy|test) ;; *) echo 'Usage: scripts/quality.sh [all|fmt|clippy|test]' >&2; exit 2 ;; esac
engine="${STRATA_CONTAINER_ENGINE:-}"
if [[ -z "$engine" ]]; then
  if command -v podman >/dev/null 2>&1; then engine=podman; else engine=docker; fi
fi
image="${STRATA_QUALITY_IMAGE:-}"
if [[ -z "$image" ]]; then
  image="$(python3 "$repository/scripts/e2e_base.py" ensure --engine "$engine")"
else
  image="$(python3 "$repository/scripts/e2e_base.py" verify "$image" --engine "$engine")"
fi
user_id="$(id -u)"
group_id="$(id -g)"
accounts="$repository/target/quality-container/accounts-$user_id-$group_id"
mkdir -p "$accounts"
printf 'root:x:0:0:root:/root:/bin/sh\n' > "$accounts/passwd"
printf 'root:x:0:\n' > "$accounts/group"
if [[ "$user_id" != 0 ]]; then
  printf 'strata-quality:x:%s:%s:Quality:/tmp/strata-quality-home:/bin/sh\n' "$user_id" "$group_id" >> "$accounts/passwd"
fi
if [[ "$group_id" != 0 ]]; then
  printf 'strata-quality:x:%s:\n' "$group_id" >> "$accounts/group"
fi
options=(--rm --platform=linux/amd64 --user "$user_id:$group_id" --shm-size=512m)
if [[ "$(basename "$engine")" == podman ]]; then
  options+=(--userns=keep-id --passwd=false)
fi
if [[ -n "${STRATA_REQUIRE_DEVICE_TESTS:-}" ]]; then
  options+=(--env STRATA_REQUIRE_DEVICE_TESTS)
fi
for variable in STRATA_QUALITY_TASK STRATA_QUALITY_SHARD; do
  if [[ -n "${!variable:-}" ]]; then options+=(--env "$variable"); fi
done
exec "$engine" run "${options[@]}" \
  --mount "type=bind,source=$repository,target=/workspace" \
  --mount "type=bind,source=$accounts/passwd,target=/etc/passwd,readonly" \
  --mount "type=bind,source=$accounts/group,target=/etc/group,readonly" \
  --workdir /workspace \
  --env HOME=/tmp/strata-quality-home \
  --env CARGO_HOME=/workspace/target/quality-container/cargo \
  --env CARGO_TARGET_DIR=/workspace/target/quality-container/build \
  --env CARGO_PROFILE_DEV_DEBUG=0 --env CARGO_INCREMENTAL=0 \
  --env "STRATA_QUALITY_COMMIT=$(git -C "$repository" rev-parse HEAD)" \
  "$image" bash -euc '
    mkdir -p "$HOME" "$CARGO_HOME"
    rustc --version
    case "$1" in all|fmt) cargo fmt --all --check ;; esac
    case "$1" in all|clippy) cargo clippy --locked --all-targets --all-features -- -D warnings ;; esac
    case "$1" in all|test)
      case "${STRATA_QUALITY_TASK:-test}" in
        build) python3 scripts/quality_ci.py build ;;
        shard)
          xvfb-run -a dbus-run-session -- env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
            GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
            STRATA_REQUIRE_DEVICE_TESTS=1 python3 scripts/quality_ci.py run
          ;;
        test)
          xvfb-run -a dbus-run-session -- env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
            GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
            cargo test --locked --all-targets --all-features
          ;;
        *) echo "Invalid STRATA_QUALITY_TASK" >&2; exit 2 ;;
      esac
      ;;
    esac
  ' bash "$phase"
