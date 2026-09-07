#!/usr/bin/env bash
set -euo pipefail

unset DISPLAY WAYLAND_DISPLAY
repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
engine="${STRATA_CONTAINER_ENGINE:-docker}"
if ! command -v "$engine" >/dev/null 2>&1; then
  echo "E2E tests require Docker or Podman; see docs/e2e-testing.md" >&2
  exit 1
fi
if [[ -n "${STRATA_BINARY:-}" ]]; then
  echo "Use scripts/e2e-native.sh for host binaries; the canonical suite builds inside its container." >&2
  exit 1
fi

image="strata-e2e:$(sha256sum "$repository/tests/e2e/Dockerfile" "$repository/tests/e2e/requirements.txt" | cut -d' ' -f1 | sha256sum | cut -c1-16)"
"$engine" build --platform=linux/amd64 --tag "$image" --file "$repository/tests/e2e/Dockerfile" "$repository/tests/e2e"

options=(--rm --platform=linux/amd64 --user "$(id -u):$(id -g)" --shm-size=512m)
if [[ "$(basename "$engine")" == podman ]]; then
  options+=(--userns=keep-id)
fi
exec "$engine" run "${options[@]}" \
  --mount "type=bind,source=$repository,target=/workspace" \
  --workdir /workspace \
  --env HOME=/tmp/strata-build-home \
  --env CARGO_HOME=/workspace/target/e2e-container/cargo \
  --env CARGO_TARGET_DIR=/workspace/target/e2e-container/build \
  --env "STRATA_E2E_UPDATE_BASELINES=${STRATA_E2E_UPDATE_BASELINES:-0}" \
  "$image" bash -euc '
    mkdir -p "$HOME" "$CARGO_HOME"
    printf "GTK: "; pkg-config --modversion gtk4
    rustc --version
    cargo build --locked --bin strata
    export STRATA_BINARY="$CARGO_TARGET_DIR/debug/strata"
    exec ./scripts/e2e-native.sh "$@"
  ' bash "$@"
