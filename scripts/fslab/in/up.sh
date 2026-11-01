#!/usr/bin/env bash
source /opt/fslab/in/common.sh

load_modules() {
  modprobe -a loop vfat exfat ntfs3 nfsd nfs nfsv3 nfsv4 cifs
}

# The attach uevent must be fully processed by the host's udev while the device
# is still blank; afterwards mkfs writes only through this container's own
# device node, which the host's inotify watch on its /dev node never sees.
wait_for_host_udev() {
  local majmin=$1 image=$2 db deadline
  db=$(host_udev_db "$majmin")
  if [[ ! -d $HOST_UDEV/data ]]; then
    log "host udev database not mounted; sleeping instead of waiting for the attach event"
    sleep 2
    return
  fi
  deadline=$((SECONDS + 15))
  until [[ -r $db ]] && grep -qxF "E:ID_LOOP_BACKING_FILENAME=$image" "$db"; do
    ((SECONDS < deadline)) || die "host udev did not process the attach of $image"
    sleep 0.05
  done
}

attach_blank() {
  local name=$1 size=$2 image dev majmin attempt error
  image=$IMAGES/$name.img
  rm -f "$image"
  truncate -s "$size" "$image"
  # A host attach can take the device between the find and the attach.
  for attempt in 1 2 3 4 5 6 7 8 9 10; do
    dev=$(losetup -f | awk '{print $1}')
    majmin=$(cat "/sys/block/${dev#/dev/}/dev")
    [[ -b $dev ]] || mknod "$dev" b "${majmin%:*}" "${majmin#*:}"
    error=$(losetup "$dev" "$image" 2>&1) && break
    [[ $error == *"Device or resource busy"* && $attempt -lt 10 ]] || die "attaching $image: $error"
    log "$dev was taken before attaching $image; retrying"
    sleep 0.2
  done
  printf '%s %s %s\n' "$dev" "$majmin" "$name" >> "$STATE/loops"
  wait_for_host_udev "$majmin" "$image"
  printf '%s\n' "$dev"
}

prepare_root() {
  local mountpoint=$1
  chown "$TEST_UID:$TEST_GID" "$mountpoint" 2>/dev/null || true
  as_test_user mkdir -p "$mountpoint/tmp"
}

mount_block() {
  local name=$1 dev
  local mountpoint=$MNT/$name
  mkdir -p "$mountpoint"
  case $name in
    ext4)
      dev=$(attach_blank "$name" 512M)
      mkfs.ext4 -q -F -L FSLABEXT4 "$dev"
      mount -i -t ext4 -o errors=remount-ro "$dev" "$mountpoint"
      prepare_root "$mountpoint"
      ;;
    ext4-casefold)
      dev=$(attach_blank "$name" 512M)
      mkfs.ext4 -q -F -O casefold -L FSLABCASE "$dev"
      mount -i -t ext4 -o errors=remount-ro "$dev" "$mountpoint"
      chown "$TEST_UID:$TEST_GID" "$mountpoint"
      mkdir "$mountpoint/tmp"
      chattr +F "$mountpoint/tmp"
      chown "$TEST_UID:$TEST_GID" "$mountpoint/tmp"
      ;;
    vfat)
      # Mount options mirror udisks' built-in vfat defaults.
      dev=$(attach_blank "$name" 512M)
      mkfs.vfat -F 32 -n FSLABVFAT "$dev" >/dev/null
      mount -i -t vfat -o "uid=$TEST_UID,gid=$TEST_GID,shortname=mixed,utf8=1,showexec,flush" "$dev" "$mountpoint"
      prepare_root "$mountpoint"
      ;;
    exfat)
      dev=$(attach_blank "$name" 512M)
      mkfs.exfat -L FSLABEXFAT "$dev" >/dev/null
      mount -i -t exfat -o "uid=$TEST_UID,gid=$TEST_GID,iocharset=utf8,errors=remount-ro" "$dev" "$mountpoint"
      prepare_root "$mountpoint"
      ;;
    exfat-fuse)
      # relan exfat-fuse 1.4 built against libfuse3 (the Debian 13/Ubuntu 24.04 build).
      dev=$(attach_blank "$name" 512M)
      mkfs.exfat -L FSLABXFUSE "$dev" >/dev/null
      mount.exfat-fuse -o "uid=$TEST_UID,gid=$TEST_GID,allow_other" "$dev" "$mountpoint"
      prepare_root "$mountpoint"
      ;;
    ntfs3)
      dev=$(attach_blank "$name" 512M)
      mkntfs -F -Q -q -L FSLABNTFS3 "$dev" >/dev/null 2>&1
      mount -i -t ntfs3 -o "uid=$TEST_UID,gid=$TEST_GID" "$dev" "$mountpoint"
      prepare_root "$mountpoint"
      ;;
    ntfs-3g)
      # udisks' ntfs-3g defaults: uid,gid,windows_names. ntfs-3g adds allow_other as root.
      dev=$(attach_blank "$name" 512M)
      mkntfs -F -Q -q -L FSLABNTFS3G "$dev" >/dev/null 2>&1
      ntfs-3g -o "uid=$TEST_UID,gid=$TEST_GID,windows_names" "$dev" "$mountpoint"
      prepare_root "$mountpoint"
      ;;
    *) die "unknown block mount $name" ;;
  esac
}

mount_tmpfs() {
  [[ $(findmnt -n -o FSTYPE /tmp) == tmpfs ]] || die "/tmp must be a tmpfs (docker run --tmpfs /tmp)"
  as_test_user mkdir -p "$(tmpdir_of tmpfs)"
}

start_nfs() {
  local export
  # tmpfs, not a loop device: after nfsd stops, the kernel can keep a reference
  # to an NFSv4.2 export, which would pin a loop device on the host until reboot.
  # Each export is its own tmpfs, so nfsd switches to it and a leak pins only that one.
  mkdir -p /srv/nfs
  mount -t tmpfs -o size=1m,mode=0755 fslab-nfs /srv/nfs
  for export in v42 v3; do
    mkdir "/srv/nfs/$export"
    mount -t tmpfs -o "size=1g,mode=0755,uid=$TEST_UID,gid=$TEST_GID" "fslab-nfs-$export" "/srv/nfs/$export"
    as_test_user mkdir "/srv/nfs/$export/tmp"
  done
  # tmpfs has no UUID, so every export needs an fsid=; fsid=0 makes /srv/nfs the
  # NFSv4 pseudo-root instead of the container's overlayfs root, which nfsd cannot export.
  cat > /etc/exports <<'EOF'
/srv/nfs     127.0.0.1(ro,sync,no_subtree_check,no_root_squash,insecure,fsid=0)
/srv/nfs/v42 127.0.0.1(rw,sync,no_subtree_check,no_root_squash,insecure,fsid=101)
/srv/nfs/v3  127.0.0.1(rw,sync,no_subtree_check,no_root_squash,insecure,fsid=102)
EOF
  cat > /etc/nfs.conf <<'EOF'
[nfsd]
threads=8
grace-time=10
lease-time=10
rdma=n
vers3=y
vers4=y
vers4.0=y
vers4.1=y
vers4.2=y
EOF
  is_mounted /proc/fs/nfsd || mount -t nfsd nfsd /proc/fs/nfsd
  is_mounted /var/lib/nfs/rpc_pipefs || mount -t rpc_pipefs sunrpc /var/lib/nfs/rpc_pipefs
  mkdir -p /run/rpcbind
  rpcbind -w
  rpc.statd --no-notify
  nfsdcld || log "nfsdcld did not start (v4 client tracking disabled)"
  exportfs -ra
  # nfs-utils 2.9.2 netlink export downcalls fail with E2BIG on 7.2.5; use the proc channels.
  rpc.mountd --no-netlink
  nfsdctl autostart >/dev/null 2>&1 || rpc.nfsd --grace-time 10 --lease-time 10 8
  mkdir -p "$MNT/nfs42" "$MNT/nfs3"
  timeout 90 mount -t nfs -o vers=4.2 127.0.0.1:/v42 "$MNT/nfs42"
  timeout 90 mount -t nfs -o vers=3 127.0.0.1:/srv/nfs/v3 "$MNT/nfs3"
}

start_smb() {
  local dev share password
  password=$(python3 -c 'import secrets; print(secrets.token_hex(16))')
  mkdir -p /srv/smb
  dev=$(attach_blank srv-smb 1G)
  mkfs.ext4 -q -F -L FSLABSMB "$dev"
  mount -i -t ext4 "$dev" /srv/smb
  for share in cifs nocase posix; do
    mkdir -p "/srv/smb/$share/tmp"
    chown -R "$TEST_UID:$TEST_GID" "/srv/smb/$share"
  done
  mkdir -p /etc/samba /var/lib/samba/private /var/cache/samba /run/samba /var/log/samba
  cat > /etc/samba/smb.conf <<'EOF'
[global]
  server role = standalone server
  workgroup = FSLAB
  security = user
  map to guest = never
  interfaces = lo
  bind interfaces only = yes
  smb ports = 445
  disable netbios = yes
  server min protocol = SMB2_10
  load printers = no
  printing = bsd
  printcap name = /dev/null
  disable spoolss = yes
  log file = /var/log/samba/log.%m
  log level = 1
  smb3 unix extensions = yes

[cifs]
  path = /srv/smb/cifs
  read only = no
  valid users = strata

[nocase]
  path = /srv/smb/nocase
  read only = no
  valid users = strata

[posix]
  path = /srv/smb/posix
  read only = no
  valid users = strata
EOF
  printf '%s\n%s\n' "$password" "$password" | smbpasswd -a -s strata >/dev/null
  smbd --daemon
  local deadline=$((SECONDS + 20))
  until (exec 3<>/dev/tcp/127.0.0.1/445) 2>/dev/null; do
    ((SECONDS < deadline)) || die "smbd did not listen on 127.0.0.1:445"
    sleep 0.2
  done
  local login="vers=3.1.1,username=strata,password=$password"
  local owner="uid=$TEST_UID,gid=$TEST_GID"
  mkdir -p "$MNT/cifs" "$MNT/cifs-nocase" "$MNT/cifs-posix"
  timeout 60 mount -t cifs "//$SMB_SERVER/cifs" "$MNT/cifs" -o "$login,$owner"
  timeout 60 mount -t cifs "//$SMB_SERVER/nocase" "$MNT/cifs-nocase" -o "$login,$owner,nocase"
  # POSIX extensions carry real modes, but owners arrive as SIDs that need the
  # cifs.idmap upcall; forcing the owner keeps client-side permission checks sane.
  timeout 60 mount -t cifs "//$SMB_SERVER/posix" "$MNT/cifs-posix" -o "$login,$owner,posix" \
    || log "cifs-posix: SMB3 POSIX extensions mount failed"
}

start_sshfs() {
  local port=2022 deadline
  mkdir -p /srv/sshfs "$MNT/sshfs"
  mount -t tmpfs -o "size=1g,mode=0755,uid=$TEST_UID,gid=$TEST_GID" fslab-sshfs /srv/sshfs
  as_test_user mkdir /srv/sshfs/tmp
  # sshfs speaks SFTP straight to a loopback sftp-server instead of through ssh.
  # The server runs as the test user, like the remote end of a user's mount, and
  # socat serves that single connection. Like a remote host, the server must not
  # see the client mount: following an extracted symlink back into it deadlocks
  # the mount. So socat starts in a private mount namespace made before sshfs
  # mounts, which holds copies of the earlier lab mounts until down stops it.
  setsid -f unshare --mount --propagation private socat "TCP-LISTEN:$port,bind=127.0.0.1,reuseaddr" \
    EXEC:"setpriv --reuid=$TEST_UID --regid=$TEST_GID --init-groups /usr/lib/ssh/sftp-server" \
    > "$STATE/sshfs-server.log" 2>&1
  deadline=$((SECONDS + 20))
  until sshfs -o "directport=$port,allow_other" "127.0.0.1:/srv/sshfs" "$MNT/sshfs" 2>> "$STATE/sshfs-server.log"; do
    ((SECONDS < deadline)) || die "sshfs did not mount: $(tail -3 "$STATE/sshfs-server.log")"
    sleep 0.2
  done
}

# Each step runs in its own bash process so `set -e` applies inside it.
if [[ ${1:-} == step ]]; then
  case $2 in
    tmpfs) mount_tmpfs ;;
    nfs) start_nfs ;;
    smb) start_smb ;;
    sshfs) start_sshfs ;;
    *) mount_block "$2" ;;
  esac
  exit 0
fi

[[ -e $STATE/up ]] && { log "already up"; exit 0; }
[[ -e $STATE/tearing-down ]] && die "a previous 'fslab.sh down' did not finish; run it again"
mkdir -p "$STATE" "$MNT"
: > "$STATE/loops"
mkdir -p /tmp/.X11-unix && chmod 1777 /tmp/.X11-unix
load_modules
mkdir -p /var/lib/extensions
is_mounted /var/lib/extensions || mount -t tmpfs -o size=12g,mode=0700 fslab-images /var/lib/extensions
mkdir -p "$IMAGES"

for step in tmpfs ext4 ext4-casefold vfat exfat ntfs3 ntfs-3g nfs smb sshfs; do
  bash "$0" step "$step"
done
bash "$0" step exfat-fuse || log "exfat-fuse: optional mount failed"
assert_host_blind
touch "$STATE/up"

log "mounts:"
findmnt -rn -o TARGET,FSTYPE,SOURCE,OPTIONS | awk '$1 ~ "^/mnt/fslab/|^/srv/"' >&2
for name in $(active_mounts); do
  as_test_user test -w "$(tmpdir_of "$name")" || log "WARNING: $(tmpdir_of "$name") is not writable for uid $TEST_UID"
done
