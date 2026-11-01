#!/usr/bin/env bash
# test.sh SNAPSHOT FILTER OUTDIR [MOUNT...]
source /opt/fslab/in/common.sh
[[ -e $STATE/up ]] || die "the lab is not up"
assert_host_blind
snapshot=$1 filter=$2 out=$3
shift 3
if (($#)); then mounts=("$@"); else mapfile -t mounts < <(active_mounts); fi
for name in "${mounts[@]}"; do known_mount "$name" || die "unknown mount $name"; done

WORK=/work/strata
mkdir -p "$WORK" /cache/cargo /cache/target "$out"
# test-headless.py forwards CARGO_HOME but not CARGO_TARGET_DIR, so the lab's
# build settings live in that CARGO_HOME's config. Crates were fetched beforehand.
cat > /cache/cargo/config.toml <<'EOF'
[build]
target-dir = "/cache/target"
incremental = false

[net]
offline = true

[profile.dev]
debug = 0
EOF
chown "$TEST_UID:$TEST_GID" /cache/cargo /cache/cargo/config.toml /cache/target "$out"
umount "$WORK" 2>/dev/null || true
# A fixed path keeps cargo fingerprints stable, so only the crate rebuilds between snapshots.
mount --bind "$snapshot" "$WORK"
trap 'umount "$WORK" 2>/dev/null || true; chown -R "$TEST_UID:$TEST_GID" "$out" 2>/dev/null || true' EXIT

headless() {
  local tmpdir=$1 logfile=$2
  shift 2
  (cd "$WORK" && as_test_user env -i \
      PATH=/usr/local/bin:/usr/bin \
      HOME=/home/strata CARGO_HOME=/cache/cargo RUSTUP_HOME=/opt/rustup \
      STRATA_TEST_TMPDIR="$tmpdir" \
      timeout -k 60 "${FSLAB_TEST_TIMEOUT:-3600}" \
      python3 "$WORK/scripts/test-headless.py" "$@") > "$logfile" 2>&1
}

log "building (cargo test --all-targets --all-features --no-run)"
start=$SECONDS
if ! headless "" "$out/build.log" --no-run; then
  tail -40 "$out/build.log" >&2
  die "build failed; see $out/build.log"
fi
log "build finished in $((SECONDS - start))s"

for name in "${mounts[@]}"; do
  tmp=$(tmpdir_of "$name")
  if ! is_mounted "$(mountpoint_of "$name")"; then
    log "$name: not mounted, skipping"
    echo "not mounted" > "$out/$name.skipped"
    continue
  fi
  log "$name: running test-headless.py --no-fail-fast $filter (TMPDIR=$tmp)"
  start=$SECONDS
  status=0
  kernel=$(python3 -I "$LAB/in/kmsg.py" last 2>/dev/null) || kernel=
  headless "$tmp" "$out/$name.log" --no-fail-fast "$filter" || status=$?
  echo "$status $((SECONDS - start))" > "$out/$name.status"
  # Kernel messages logged during this mount's run, such as cifs turning server inode numbers off.
  if [[ -n $kernel ]]; then
    python3 -I "$LAB/in/kmsg.py" since "$kernel" "$SMB_SERVER" > "$out/$name.kmsg" 2>/dev/null || rm -f "$out/$name.kmsg"
  fi
  find "$tmp" -mindepth 1 -maxdepth 3 -printf '%P\n' 2>/dev/null | head -50 > "$out/$name.leftovers" || true
  find "$tmp" -mindepth 1 -delete 2>/dev/null || rm -rf "${tmp:?}"/* "${tmp:?}"/.[!.]* 2>/dev/null || true
  log "$name: exit $status in $((SECONDS - start))s"
done
python3 -I "$LAB/in/summary.py" "$out" "${mounts[@]}" > "$out/summary.md"
cat "$out/summary.md" >&2
