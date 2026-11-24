#!/usr/bin/env bash
# Runs the `--qa` scenarios in tests/qa/scenarios/<folder>/ inside the pinned test image.
#   ./scripts/qa.sh                                  every scenario, every mode
#   ./scripts/qa.sh --smoke                          critical-path smoke list (tests/qa/smoke.txt)
#   ./scripts/qa.sh arrow-keys-move-focus            one scenario (hyphens or underscores)
#   ./scripts/qa.sh arrow-keys-move-focus/columns    one run of it
set -euo pipefail

unset DISPLAY WAYLAND_DISPLAY NOTIFY_SOCKET
repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=lib/qa-container.sh
source "$repository/scripts/lib/qa-container.sh"
qa_container_prepare

# A --skip naming a scenario or run becomes --skip-exact, so it never skips
# another scenario that the name prefixes.
scenarios="$repository/tests/qa/scenarios"
arguments=()
add_scenario() {
  local raw="$1"
  local scenario="${raw%%/*}"
  scenario="${scenario//_/-}"
  if ! compgen -G "$scenarios/*/$scenario.jsonl" >/dev/null; then
    echo "No QA scenario named ${raw%%/*} in tests/qa/scenarios:" >&2
    basename -a -s .jsonl "$scenarios"/*/*.jsonl | sort >&2
    exit 2
  fi
  arguments+=("$scenario${raw#"${raw%%/*}"}")
  named=1
}
skip() {
  local scenario="${1%%/*}"
  scenario="${scenario//_/-}"
  if [[ -n "$scenario" ]] && compgen -G "$scenarios/*/$scenario.jsonl" >/dev/null; then
    arguments+=(--skip-exact "$scenario${1#"${1%%/*}"}")
  else
    arguments+=(--skip "$1")
  fi
}
while (($#)); do
  case "$1" in
    --smoke)
      smoke_file="$repository/tests/qa/smoke.txt"
      if [[ ! -f "$smoke_file" ]]; then
        echo "Missing smoke scenario list: $smoke_file" >&2
        exit 2
      fi
      smoke_count=0
      while IFS= read -r line || [[ -n "$line" ]]; do
        line="${line%%#*}"
        line="${line#"${line%%[![:space:]]*}"}"
        line="${line%"${line##*[![:space:]]}"}"
        [[ -z "$line" ]] && continue
        add_scenario "$line"
        smoke_count=$((smoke_count + 1))
      done < "$smoke_file"
      if ((smoke_count == 0)); then
        echo "Smoke scenario list is empty: $smoke_file" >&2
        exit 2
      fi
      shift
      ;;
    --skip)
      skip "${2:?$1 needs a value}"
      shift 2
      ;;
    --skip=*)
      skip "${1#--skip=}"
      shift
      ;;
    --test-threads|--format|--color|-Z)
      arguments+=("$1" "${2:?$1 needs a value}")
      shift 2
      ;;
    -*)
      arguments+=("$1")
      shift
      ;;
    *)
      add_scenario "$1"
      shift
      ;;
  esac
done
if ((${named:-0})); then
  arguments+=(--exact)
fi

qa_container_options --rm
qa_container_mounts
exec "$engine" run "${options[@]}" "${mounts[@]}" \
  --env "STRATA_QA_KEEP_ARTIFACTS=${STRATA_QA_KEEP_ARTIFACTS:-0}" \
  --env "STRATA_QA_NAMED=${named:-0}" \
  --env "STRATA_QA_MAIN_IO=${STRATA_QA_MAIN_IO:-}" \
  --env "STRATA_UI_STALL_MS=${STRATA_UI_STALL_MS:-}" \
  "$image" bash -euc '
    mkdir -p "$HOME" "$CARGO_HOME"
    # An empty cache takes the crates the image holds; a used one is left alone.
    python3 scripts/dependency_seed.py
    rustc --version
    output="$(mktemp)"
    # Runner unit tests alone print no PASS or KNOWN BUG line, so require one.
    cargo test --locked --features qa,invariant-checks --test qa -- --include-ignored "$@" 2>&1 | tee "$output"
    status="${PIPESTATUS[0]}"
    if ((status != 0)); then exit "$status"; fi
    if ! grep -Eq "^(PASS|KNOWN BUG) " "$output"; then
      echo "The selection ran no QA scenario." >&2
      exit 1
    fi
  ' bash "${arguments[@]}"
