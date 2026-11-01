#!/usr/bin/env bash
set -euo pipefail

lab="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repository="$(cd "$lab/../.." && pwd)"
output="$repository/target/fslab"
image=strata-fslab:latest
container=strata-fslab
lab_copy=/opt/fslab
lab_label=org.strata.fslab.lab
cargo_volume=strata-fslab-cargo
target_volume=strata-fslab-target

usage() {
  cat >&2 <<'EOF'
Usage: scripts/fslab/fslab.sh COMMAND
  image                               build the lab image on the pinned test image
  up | down [--force] | status | shell
                                      manage the privileged lab container; down
                                      releases what is free and keeps the
                                      container running while a mount stays busy,
                                      and --force unmounts lazily, which can pin
                                      host loop devices or tmpfs memory until
                                      reboot
  probe [MOUNT...]                    run fsprobe.py on every (or the named) mount
  test [--rev REV] FILTER [MOUNT...]  run test-headless.py FILTER on each mount,
                                      against the working tree or a git revision
EOF
  exit 2
}

die() {
  echo "fslab: $*" >&2
  exit 1
}

base_image() {
  if [[ -n "${FSLAB_BASE_IMAGE:-}" ]]; then
    python3 "$repository/scripts/image_base.py" verify "$FSLAB_BASE_IMAGE" --engine docker
  else
    python3 "$repository/scripts/image_base.py" ensure --engine docker
  fi
}

image_inputs() {
  { printf '%s %s %s\n' "$1" "$(id -u)" "$(id -g)"; cat "$lab/Dockerfile"; } | sha256sum | cut -d' ' -f1
}

build_image() {
  local base=$1 tag
  # BuildKit resolves FROM by name only, so give the verified image ID a local tag.
  tag="strata-fslab-base:${base#sha256:}"
  docker tag "$base" "$tag"
  docker build --platform=linux/amd64 --tag "$image" \
    --build-arg "BASE=$tag" --build-arg "FSLAB_UID=$(id -u)" --build-arg "FSLAB_GID=$(id -g)" \
    --label "org.strata.fslab.inputs=$(image_inputs "$base")" - < "$lab/Dockerfile"
}

ensure_image() {
  local base current
  base="$(base_image)"
  current="$(docker image inspect --format '{{ index .Config.Labels "org.strata.fslab.inputs" }}' "$image" 2>/dev/null || true)"
  [[ "$current" == "$(image_inputs "$base")" ]] || build_image "$base"
}

running() {
  [[ "$(docker container inspect -f '{{.State.Running}}' "$container" 2>/dev/null)" == true ]]
}

remove_container() {
  local label
  label="$(docker container inspect -f "{{ index .Config.Labels \"$lab_label\" }}" "$container" 2>/dev/null)" || return 0
  [[ "$label" == 1 ]] || die "refusing to remove container $container: it lacks the $lab_label label"
  docker rm -f "$container" >/dev/null
}

# The working directory is / because a lab mount may be gone by the next exec.
in_lab() { docker exec -w / "$container" "$@"; }

# The container runs its own copy of the lab scripts, refreshed by every command,
# so a checkout that changes or vanishes while the lab is up cannot break its teardown.
sync_scripts() {
  in_lab bash -c "rm -rf $lab_copy.new && mkdir -p $lab_copy.new" || return
  tar -C "$lab" -c --exclude=__pycache__ --owner=0 --group=0 -f - in fsprobe |
    docker cp - "$container:$lab_copy.new" || return
  in_lab bash -c "rm -rf $lab_copy.old && if [[ -e $lab_copy ]]; then mv $lab_copy $lab_copy.old; fi && mv $lab_copy.new $lab_copy && rm -rf $lab_copy.old"
}

cmd_up() {
  if ! running; then
    ensure_image
    mkdir -p "$output"
    remove_container
    # Privileged for modprobe, loop devices and nfsd. No network, so the NFS and
    # SMB servers are reachable only over the container's own loopback.
    docker run -d --name "$container" --label "$lab_label=1" --hostname fslab \
      --privileged --init --network none \
      --platform=linux/amd64 --shm-size=512m \
      --tmpfs /tmp:rw,exec,mode=1777,size=16g \
      -v /lib/modules:/lib/modules:ro \
      -v /run/udev:/host-udev:ro \
      -v "$output:/out" \
      -v "$cargo_volume:/cache/cargo" \
      -v "$target_volume:/cache/target" \
      "$image" sleep infinity >/dev/null
  fi
  sync_scripts
  if ! in_lab bash "$lab_copy/in/up.sh"; then
    echo "fslab: setup failed; tearing down" >&2
    cmd_down || true
    exit 1
  fi
}

cmd_down() {
  local status=0
  case "${1:-}" in
    "") ;;
    --force) echo "fslab: WARNING: --force lazily unmounts busy lab filesystems; until reboot their loop devices can stay attached on the host, and tmpfs ones keep their memory" >&2 ;;
    *) usage ;;
  esac
  if running; then
    sync_scripts || die "could not copy the lab scripts into $container; nothing was torn down"
    in_lab bash "$lab_copy/in/down.sh" "$@" || status=$?
  fi
  if ((status)) && [[ "${1:-}" != --force ]]; then
    echo "fslab: teardown incomplete; the container stays running so its mounts remain reachable." >&2
    echo "fslab: once what is listed above is no longer busy, run 'scripts/fslab/fslab.sh down' again." >&2
    echo "fslab: 'scripts/fslab/fslab.sh down --force' unmounts lazily instead and can pin loop devices or tmpfs memory until reboot." >&2
    return 1
  fi
  remove_container
}

ensure_up() {
  if running && in_lab test -e /run/fslab/up; then sync_scripts; else cmd_up; fi
}

cmd_status() {
  running || { echo "fslab: container not running"; return; }
  in_lab bash -c 'findmnt -rn -o TARGET,FSTYPE,OPTIONS | awk "\$1 ~ \"^/mnt/fslab/\""; losetup -l'
}

# The lab has no network, so crates are fetched by an unprivileged container.
fetch_crates() {
  docker run --rm --platform=linux/amd64 --user "$(id -u):$(id -g)" \
    --cap-drop ALL --security-opt no-new-privileges \
    -e HOME=/home/strata -e CARGO_HOME=/cache/cargo -e RUSTUP_HOME=/opt/rustup -e CARGO_NET_OFFLINE=false \
    -v "$cargo_volume:/cache/cargo" -v "$1:/work/strata:ro" -w /work/strata \
    "$image" cargo fetch --locked
}

worktree_files() {
  local path
  git -C "$repository" ls-files -z --cached --others --exclude-standard --deduplicate |
    while IFS= read -r -d '' path; do
      if [[ -e "$repository/$path" || -L "$repository/$path" ]]; then printf '%s\0' "$path"; fi
    done
}

cmd_test() {
  local rev=""
  if [[ "${1:-}" == --rev ]]; then
    (($# >= 2)) || usage
    rev=$2
    shift 2
  fi
  (($# >= 1)) || usage
  local filter=$1 commit name snapshot label run source
  shift
  if [[ -n "$rev" ]]; then
    commit="$(git -C "$repository" rev-parse --verify --quiet "$rev^{commit}")" || die "unknown revision $rev"
    name=$commit
    source="revision $rev ($commit)"
  else
    commit="$(git -C "$repository" rev-parse HEAD)"
    name=worktree
    source="working tree at $commit with $(git -C "$repository" status --porcelain | wc -l) changed paths"
  fi
  snapshot="$output/snapshots/$name"
  rm -rf "$snapshot"
  mkdir -p "$snapshot"
  # -m stamps fresh mtimes so cargo rebuilds the crate for every snapshot.
  if [[ -n "$rev" ]]; then
    git -C "$repository" archive --format=tar "$commit" | tar -x -m -C "$snapshot"
  else
    worktree_files | tar -C "$repository" -c --null --no-recursion -T - -f - | tar -x -m -C "$snapshot"
  fi
  if ! grep -q STRATA_TEST_TMPDIR "$snapshot/scripts/test-headless.py"; then
    cp "$repository/scripts/test-headless.py" "$snapshot/scripts/test-headless.py"
    source="$source, with the working tree's scripts/test-headless.py"
  fi
  label="$(date -u +%Y%m%dT%H%M%SZ)-${name:0:12}-$(printf '%s' "$filter" | tr -c 'A-Za-z0-9_.-' '_')"
  run="$output/runs/$label"
  mkdir -p "$run"
  printf 'source=%s\nfilter=%s\nmounts=%s\n' "$source" "$filter" "$*" > "$run/run.txt"
  ensure_up
  fetch_crates "$snapshot"
  in_lab bash "$lab_copy/in/test.sh" "/out/snapshots/$name" "$filter" "/out/runs/$label" "$@"
  echo "fslab: results in $run/summary.md"
}

[[ "$(id -u)" != 0 ]] || die "run as the repository owner, not root"
case "${1:-}" in
  image) build_image "$(base_image)" ;;
  up) cmd_up ;;
  down) shift; cmd_down "$@" ;;
  status) cmd_status ;;
  shell) ensure_up; docker exec -it -w / "$container" bash ;;
  probe) shift; ensure_up; in_lab bash "$lab_copy/in/probe.sh" "$@" ;;
  test) shift; cmd_test "$@" ;;
  *) usage ;;
esac
