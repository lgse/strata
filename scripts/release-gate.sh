#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Non-publishing, native-architecture release-artifact validation.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
repo=$PWD
build_environment=false
if [[ ${1:-} == --build-environment ]]; then build_environment=true; shift; fi
[[ $# == 0 ]] || { echo 'usage: release-gate.sh [--build-environment]' >&2; exit 2; }
engine=${STRATA_CONTAINER_ENGINE:-}
if [[ -z "$engine" ]]; then
    if command -v podman >/dev/null; then engine=podman; else engine=docker; fi
fi
case "$(uname -m)" in
    x86_64) architecture=amd64; target=x86_64-unknown-linux-gnu ;;
    aarch64) architecture=arm64; target=aarch64-unknown-linux-gnu ;;
    *) echo 'Unsupported release target' >&2; exit 1 ;;
esac
inputs=$(python3 scripts/e2e_bundle.py image-key)
image="strata-release-gate:${inputs}-${architecture}-$(id -u)-$(id -g)"
if ! "$engine" image inspect "$image" >/dev/null 2>&1; then
    $build_environment || { echo 'Explicitly bootstrap this unpublished native environment with --build-environment.' >&2; exit 1; }
    "$engine" build --platform="linux/$architecture" --target toolchain --tag "$image" \
        --build-arg "E2E_UID=$(id -u)" --build-arg "E2E_GID=$(id -g)" \
        --label "org.strata.release-gate.inputs=$inputs" --file tests/e2e/Dockerfile .
fi
"$engine" image inspect "$image" | python3 -I -c '
import json,sys
image, = json.load(sys.stdin)
assert image["Architecture"] == sys.argv[1] and image["Os"] == "linux"
assert image["Config"]["Labels"]["org.strata.release-gate.inputs"] == sys.argv[2]
assert "RUSTUP_TOOLCHAIN=1.98.1" in image["Config"]["Env"]
print("Release gate environment:", image["Id"], sys.argv[1], sys.argv[2])
' "$architecture" "$inputs"
python3 scripts/check-legacy-fixtures.py
source_sha=$(git rev-parse HEAD)
out=${STRATA_RELEASE_GATE_OUTPUT:-target/release-gate-evidence}
mkdir -p "$out" target/release-gate/cargo target/release-gate/build
out=$(cd "$out" && pwd -P)
mkdir -p "$out/tmp" "$out/home"
options=(--rm --mount "type=bind,source=$repo,target=/workspace,readonly"
    --mount "type=bind,source=$repo/target/release-gate,target=/workspace/target/release-gate"
    --mount "type=bind,source=$out,target=/evidence" --workdir /workspace --env CARGO_PROFILE_RELEASE_DEBUG=line-tables-only)
if [[ $(basename "$engine") == podman ]]; then
    options+=(--userns=keep-id --passwd=false --security-opt 'unmask=/proc/*')
else
    options+=(--security-opt systempaths=unconfined --security-opt seccomp=unconfined --security-opt apparmor=unconfined)
fi
for generation in 1 2; do
    "$engine" run "${options[@]}" --user "$(id -u):$(id -g)" \
        --env HOME=/evidence/home --env TMPDIR=/evidence/tmp \
        --env CARGO_HOME=/workspace/target/release-gate/cargo --env CARGO_TARGET_DIR=/workspace/target/release-gate/build \
        --env CARGO_INCREMENTAL=0 --env TARGET="$target" --env VERSION="0.0.0-rc.$generation" \
        --env SOURCE_SHA="$source_sha" --env STRATA_RELEASE_OUTPUT="/evidence/generation-$generation" \
        "$image" ./scripts/build-release-bundle.sh
done
old="/evidence/generation-1/strata-0.0.0-rc.1-$target"
new="/evidence/generation-2/strata-0.0.0-rc.2-$target"
"$engine" run "${options[@]}" --network=none --user "$(id -u):$(id -g)" \
    --env HOME=/evidence/home --env TMPDIR=/evidence/tmp \
    --env CARGO_HOME=/workspace/target/release-gate/cargo --env CARGO_TARGET_DIR=/workspace/target/release-gate/build \
    --env STRATA_LEGACY_ARCHIVE="$new.tar.gz" --env STRATA_LEGACY_PREVIOUS="$old/strata" \
    --env STRATA_LEGACY_UI="$new/strata" --env STRATA_LEGACY_OUTPUT=/evidence/legacy \
    "$image" cargo test --offline --release --locked --target "$target" --test legacy_release -- --ignored --nocapture
"$engine" run "${options[@]}" --network=none --user "$(id -u):$(id -g)" \
    --env HOME=/evidence/home --env TMPDIR=/evidence/tmp \
    --env CARGO_HOME=/workspace/target/release-gate/cargo --env CARGO_TARGET_DIR=/workspace/target/release-gate/build \
    --env STRATA_LEGACY_PREVIOUS="$old/strata" --env STRATA_LEGACY_UI="$new/strata" --env STRATA_RUST_OUTPUT=/evidence/rust-installed \
    "$image" ./scripts/test-headless.py --offline --release --locked --target "$target" native_release_archives -- --ignored --nocapture
"$engine" run "${options[@]}" --network=none --user "$(id -u):$(id -g)" \
    --env HOME=/evidence/home --env TMPDIR=/evidence/tmp "$image" \
    /opt/e2e-venv/bin/python scripts/check-release-bundle.py "$new" --previous "$old" --output /evidence/installed --legacy /evidence/legacy --rust-installed /evidence/rust-installed/strata
"$engine" run "${options[@]}" --network=none --user 0 "$image" \
    /opt/e2e-venv/bin/python scripts/check-release-capabilities.py "$new" \
    --output /evidence/capabilities --uid "$(id -u)" --gid "$(id -g)"
printf 'Verified native %s release artifacts; evidence: %s\n' "$target" "$out"
