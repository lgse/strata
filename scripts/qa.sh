#!/usr/bin/env bash
# Runs the `--qa` scenarios in tests/qa/scenarios/<folder>/ inside the pinned test image.
#   ./scripts/qa.sh                                  every scenario, every mode
#   ./scripts/qa.sh arrow-keys-move-focus            one scenario (hyphens or underscores)
#   ./scripts/qa.sh arrow-keys-move-focus/columns    one run of it
# STRATA_QA_UPDATE_BASELINES=1 writes each `baseline` capture to tests/qa/baselines.
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
      scenario="${1%%/*}"
      scenario="${scenario//_/-}"
      if ! compgen -G "$scenarios/*/$scenario.jsonl" >/dev/null; then
        echo "No QA scenario named ${1%%/*} in tests/qa/scenarios:" >&2
        basename -a -s .jsonl "$scenarios"/*/*.jsonl | sort >&2
        exit 2
      fi
      arguments+=("$scenario${1#"${1%%/*}"}")
      named=1
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
  --env "STRATA_QA_UPDATE_BASELINES=${STRATA_QA_UPDATE_BASELINES:-0}" \
  --env "STRATA_QA_NAMED=${named:-0}" \
  --env "STRATA_QA_MAIN_IO=${STRATA_QA_MAIN_IO:-}" \
  --env "STRATA_UI_STALL_MS=${STRATA_UI_STALL_MS:-}" \
  "$image" bash -euc '
    mkdir -p "$HOME" "$CARGO_HOME"
    # The tmpfs hides the X socket directory of the image; the xvfb.log owner warning is harmless.
    mkdir -p -m 1777 /tmp/.X11-unix
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
