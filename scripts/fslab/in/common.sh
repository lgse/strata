# shellcheck shell=bash
set -euo pipefail

LAB=/opt/fslab
STATE=/run/fslab
MNT=/mnt/fslab
# Container-private tmpfs. Host udisks reads loop backing files relative to its
# own root, so these images show up as /var/lib/extensions/fslab/*.img, a path
# the host's udiskie never automounts.
IMAGES=/var/lib/extensions/fslab
HOST_UDEV=/host-udev
OUT=/out
TEST_UID=$(id -u strata)
TEST_GID=$(id -g strata)
# The UNC server of the lab's SMB mounts, which tells their kernel messages from the host's.
SMB_SERVER=127.0.0.1

ALL_MOUNTS=(tmpfs ext4 ext4-casefold vfat exfat exfat-fuse ntfs3 ntfs-3g nfs42 nfs3 cifs cifs-nocase cifs-posix sshfs)

log() { printf '[fslab] %s\n' "$*" >&2; }
die() { log "ERROR: $*"; exit 1; }

# "tmpfs" is the container's own /tmp (a tmpfs, like a developer machine): the
# harness keeps HOME there too, so Trash behaves as it does on the host.
mountpoint_of() { if [[ $1 == tmpfs ]]; then echo /tmp; else echo "$MNT/$1"; fi; }
tmpdir_of() { if [[ $1 == tmpfs ]]; then echo /tmp/fslab-tmpfs; else echo "$MNT/$1/tmp"; fi; }

# Reads mountinfo instead of stat()ing the path, which can hang on a dead NFS server.
is_mounted() { awk -v target="$1" '$5 == target { found = 1 } END { exit !found }' /proc/self/mountinfo; }

as_test_user() {
  setpriv --reuid="$TEST_UID" --regid="$TEST_GID" --init-groups "$@"
}

known_mount() {
  local name
  for name in "${ALL_MOUNTS[@]}"; do [[ $name == "$1" ]] && return 0; done
  return 1
}

active_mounts() {
  local name
  for name in "${ALL_MOUNTS[@]}"; do
    if is_mounted "$(mountpoint_of "$name")" && [[ -d $(tmpdir_of "$name") ]]; then printf '%s\n' "$name"; fi
  done
}

host_udev_db() { printf '%s/data/b%s\n' "$HOST_UDEV" "$1"; }

assert_host_blind() {
  local dev majmin name db
  [[ -d $HOST_UDEV/data ]] || return 0
  while read -r dev majmin name; do
    db=$(host_udev_db "$majmin")
    if [[ -r $db ]] && grep -q '^E:ID_FS_TYPE=.' "$db"; then
      die "host udev identified a filesystem on $dev ($name); run 'fslab.sh down' now"
    fi
  done < "$STATE/loops"
}
