#!/usr/bin/env bash
set -euo pipefail

unset DISPLAY WAYLAND_DISPLAY NOTIFY_SOCKET
repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
engine="${STRATA_CONTAINER_ENGINE:-}"
if [[ -z "$engine" ]]; then
  if command -v podman >/dev/null 2>&1; then engine=podman; else engine=docker; fi
fi
if ! command -v "$engine" >/dev/null 2>&1; then
  echo "E2E tests require Docker or Podman; see docs/e2e-testing.md" >&2
  exit 1
fi
if [[ -n "${STRATA_BINARY:-}" ]]; then
  echo "Use scripts/e2e-native.sh for host binaries; the canonical suite builds inside its container." >&2
  exit 1
fi

user_id="$(id -u)"
group_id="$(id -g)"
image="${STRATA_E2E_IMAGE:-}"
image_key="$(python3 "$repository/scripts/e2e_bundle.py" image-key)"
bundle=""
if [[ -n "${STRATA_E2E_BUNDLE:-}" ]]; then
  if [[ -z "$image" ]]; then
    echo "A CI bundle requires its pinned runtime image (STRATA_E2E_IMAGE)." >&2
    exit 1
  fi
  bundle="$(realpath --relative-to="$repository" "$STRATA_E2E_BUNDLE")"
  if [[ "$bundle" == .. || "$bundle" == ../* ]]; then
    echo "The CI bundle must be inside the checkout." >&2
    exit 1
  fi
  python3 "$repository/scripts/e2e_bundle.py" verify "$repository/$bundle"
  image_inputs="$("$engine" image inspect --format '{{ index .Config.Labels "org.strata.e2e.inputs" }}' "$image")"
  if [[ "$image_inputs" != "$image_key" ]]; then
    echo "The runtime image was not built from this checkout's pinned E2E inputs." >&2
    exit 1
  fi
fi
if [[ -z "$image" ]]; then
  image="$(python3 "$repository/scripts/e2e_base.py" ensure --engine "$engine")"
elif [[ -z "$bundle" ]]; then
  image="$(python3 "$repository/scripts/e2e_base.py" verify "$image" --engine "$engine")"
fi

accounts="$repository/target/e2e-container/accounts-$user_id-$group_id"
mkdir -p "$accounts"
printf 'root:x:0:0:root:/root:/bin/sh\n' > "$accounts/passwd"
printf 'root:x:0:\n' > "$accounts/group"
if [[ "$user_id" != 0 ]]; then
  printf 'strata-e2e:x:%s:%s:E2E:/tmp/strata-build-home:/bin/sh\n' "$user_id" "$group_id" >> "$accounts/passwd"
fi
if [[ "$group_id" != 0 ]]; then
  printf 'strata-e2e:x:%s:\n' "$group_id" >> "$accounts/group"
fi

options=(--rm --platform=linux/amd64 --user "$user_id:$group_id" --shm-size=512m)
if [[ "$(basename "$engine")" == podman ]]; then
  # Bubblewrap needs an unmasked proc tree to mount its nested private /proc.
  options+=(--userns=keep-id --passwd=false --security-opt 'unmask=/proc/*')
else
  # Docker also blocks the nested namespace/mount syscalls used by bubblewrap.
  options+=(--security-opt systempaths=unconfined --security-opt seccomp=unconfined --security-opt apparmor=unconfined)
fi
exec "$engine" run "${options[@]}" \
  --mount "type=bind,source=$repository,target=/workspace" \
  --mount "type=bind,source=$accounts/passwd,target=/etc/passwd,readonly" \
  --mount "type=bind,source=$accounts/group,target=/etc/group,readonly" \
  --workdir /workspace \
  --env HOME=/tmp/strata-build-home \
  --env CARGO_HOME=/workspace/target/e2e-container/cargo \
  --env CARGO_TARGET_DIR=/workspace/target/e2e-container/build \
  --env CARGO_PROFILE_DEV_DEBUG=0 --env CARGO_INCREMENTAL=0 \
  --env "STRATA_E2E_UPDATE_BASELINES=${STRATA_E2E_UPDATE_BASELINES:-0}" \
  --env "STRATA_E2E_WORKERS=${STRATA_E2E_WORKERS:-auto}" \
  --env "STRATA_E2E_BUNDLE=${bundle:+/workspace/$bundle}" \
  "$image" bash -euc '
    mkdir -p "$HOME" "$CARGO_HOME"
    if [[ -n "${STRATA_E2E_BUNDLE:-}" ]]; then
      export STRATA_BINARY="$STRATA_E2E_BUNDLE/strata"
      test -x "$STRATA_BINARY"
      test -x "$STRATA_E2E_BUNDLE/strata-media-helper"
    else
      rustc --version
      cargo build --locked --workspace --bins
      export STRATA_BINARY="$CARGO_TARGET_DIR/debug/strata"
    fi
    exec ./scripts/e2e-native.sh "$@"
  ' bash "$@"
