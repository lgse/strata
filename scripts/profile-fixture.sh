#!/usr/bin/env bash
set -euo pipefail

fixture="${1:?usage: $0 FIXTURE_DIRECTORY [TIMEOUT_SECONDS]}"
timeout_seconds="${2:-15}"
binary="${STRATA_BINARY:-target/debug/strata}"

if [[ ! -x "$binary" ]]; then
  echo "$binary does not exist; run cargo build first" >&2
  exit 1
fi

fixture="$(realpath "$fixture")"
log="$(mktemp --tmpdir strata-profile.XXXXXX.log)"
peak_rss_kb=0
peak_pss_kb=0

sample_memory() {
  if [[ -r "/proc/$pid/status" ]]; then
    rss_kb="$(awk '/VmRSS:/ { print $2 }' "/proc/$pid/status")"
    if (( ${rss_kb:-0} > peak_rss_kb )); then
      peak_rss_kb="${rss_kb:-0}"
    fi
  fi
  if [[ -r "/proc/$pid/smaps_rollup" ]]; then
    pss_kb="$(awk '/^Pss:/ { print $2 }' "/proc/$pid/smaps_rollup")"
    if (( ${pss_kb:-0} > peak_pss_kb )); then
      peak_pss_kb="${pss_kb:-0}"
    fi
  fi
}

cleanup() {
  if [[ -n "${pid:-}" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

env RUST_LOG=strata=debug "$binary" "$fixture" >"$log" 2>&1 &
pid=$!
started=$SECONDS

while kill -0 "$pid" 2>/dev/null; do
  sample_memory

  if grep -q "directory publication finished" "$log"; then
    sample_memory
    break
  fi
  if (( SECONDS - started >= timeout_seconds )); then
    echo "profile timed out after ${timeout_seconds}s" >&2
    break
  fi
  sleep 0.02
done

cleanup
pid=""

printf 'Sampled peak RSS: %s KB\n' "$peak_rss_kb"
printf 'Sampled peak PSS: %s KB\n' "$peak_pss_kb"
grep -E "window presented|first directory batch ready|first directory batch rendered|directory load finished|directory publication finished" "$log" || true
printf 'Full log: %s\n' "$log"
