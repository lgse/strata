#!/usr/bin/env bash
source /opt/fslab/in/common.sh
[[ -e $STATE/up ]] || die "the lab is not up"
assert_host_blind

out=$OUT/probe
mkdir -p "$out"
chown "$TEST_UID:$TEST_GID" "$out"
json=$out/real-probe.jsonl
: > "$json"
chown "$TEST_UID:$TEST_GID" "$json"

if (($#)); then mounts=("$@"); else mapfile -t mounts < <(active_mounts); fi

backing_of() {
  case $1 in
    nfs42) echo /srv/nfs/v42/tmp ;;
    nfs3) echo /srv/nfs/v3/tmp ;;
    cifs) echo /srv/smb/cifs/tmp ;;
    cifs-nocase) echo /srv/smb/nocase/tmp ;;
    cifs-posix) echo /srv/smb/posix/tmp ;;
    sshfs) echo /srv/sshfs/tmp ;;
  esac
}

{
  echo "# mounts probed $(date -u +%FT%TZ), kernel $(uname -r)"
  for name in "${mounts[@]}"; do
    findmnt -n -o TARGET,FSTYPE,SOURCE,OPTIONS "$(mountpoint_of "$name")"
  done
} > "$out/mounts.txt"

for name in "${mounts[@]}"; do
  known_mount "$name" || die "unknown mount $name"
  other=$(tmpdir_of tmpfs)
  [[ $name == tmpfs ]] && other=$(tmpdir_of ext4)
  args=("$name" "$(tmpdir_of "$name")" --other "$other" --json "$json")
  backing=$(backing_of "$name")
  [[ -n $backing ]] && args+=(--backing "$backing")
  log "probing $name"
  as_test_user timeout -k 10 600 python3 -I "$LAB/fsprobe/fsprobe.py" "${args[@]}" > "$out/$name.txt" 2>&1 \
    || log "$name: fsprobe exited with $?"
done
python3 -I "$LAB/fsprobe/matrix.py" "$json" "$out/mounts.txt" > "$out/matrix.md"
chown -R "$TEST_UID:$TEST_GID" "$out"
log "matrix: target/fslab/probe/matrix.md"
