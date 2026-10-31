# shellcheck shell=bash
# Sourced by scripts/qa.sh and scripts/qa-drive.sh; $repository must name the checkout.

qa_container_engine() {
  engine="${STRATA_CONTAINER_ENGINE:-}"
  if [[ -z "$engine" ]]; then
    if command -v podman >/dev/null 2>&1; then engine=podman; else engine=docker; fi
  fi
}

qa_container_prepare() {
  qa_container_engine
  if ! command -v "$engine" >/dev/null 2>&1; then
    echo "QA scenarios require Docker or Podman; see docs/testing.md" >&2
    exit 1
  fi

  cargo_cache="$(realpath -m --relative-to="$repository" "${STRATA_TEST_CARGO_CACHE:-$repository/target/qa-container}")"
  if [[ "$cargo_cache" != target/* ]]; then
    echo "STRATA_TEST_CARGO_CACHE must name a directory under the checkout's target/." >&2
    exit 1
  fi

  image="${STRATA_TEST_IMAGE:-}"
  if [[ -z "$image" ]]; then
    image="$(python3 "$repository/scripts/image_base.py" ensure --engine "$engine")"
  else
    image="$(python3 "$repository/scripts/image_base.py" verify "$image" --engine "$engine")"
  fi

  user_id="$(id -u)"
  group_id="$(id -g)"
  accounts="$repository/target/qa-container/accounts-$user_id-$group_id"
  mkdir -p "$accounts"
  printf 'root:x:0:0:root:/root:/bin/sh\n' > "$accounts/passwd"
  printf 'root:x:0:\n' > "$accounts/group"
  if [[ "$user_id" != 0 ]]; then
    printf 'strata-test:x:%s:%s:Strata test:/tmp/strata-build-home:/bin/sh\n' "$user_id" "$group_id" >> "$accounts/passwd"
  fi
  if [[ "$group_id" != 0 ]]; then
    printf 'strata-test:x:%s:\n' "$group_id" >> "$accounts/group"
  fi
}

qa_container_options() {
  # /tmp is a tmpfs; artifacts and the Cargo cache must live under /workspace.
  options=("$@" --platform=linux/amd64 --user "$user_id:$group_id" --shm-size=512m
    --tmpfs /tmp:rw,exec,mode=1777)
  if [[ "$(basename "$engine")" == podman ]]; then
    options+=(--userns=keep-id --passwd=false --security-opt 'unmask=/proc/*')
  else
    options+=(--security-opt systempaths=unconfined --security-opt seccomp=unconfined --security-opt apparmor=unconfined)
  fi
}

qa_container_mounts() {
  mounts=(
    --mount "type=bind,source=$repository,target=/workspace"
    --mount "type=bind,source=$accounts/passwd,target=/etc/passwd,readonly"
    --mount "type=bind,source=$accounts/group,target=/etc/group,readonly"
    --workdir /workspace
    --env HOME=/tmp/strata-build-home
    --env "CARGO_HOME=/workspace/$cargo_cache/cargo"
    --env "CARGO_TARGET_DIR=/workspace/$cargo_cache/build"
    --env CARGO_PROFILE_DEV_DEBUG=0 --env CARGO_INCREMENTAL=0
    --env STRATA_QA_CONTAINER=1
  )
}
