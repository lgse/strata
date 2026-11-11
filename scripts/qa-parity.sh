#!/usr/bin/env bash
# Runs the runs of the parity ledger, tests/qa/parity/slice.toml, on both QA targets inside
# the pinned test image: the GTK application and strata-rune (docs/rune/slice/README.md).
#   ./scripts/qa-parity.sh g1                 gate G1's required runs, on each target
#   ./scripts/qa-parity.sh slice              every gate's required runs on each target, then
#                                             the runner's own tests and the whole GTK suite
#                                             and the Rune mutation patches
#   ./scripts/qa-parity.sh g2 --target rune   one target only: gtk or rune
#   ./scripts/qa-parity.sh g2 --pending       a survey: gate G2's pending runs on the Rune
#                                             target, listing which pass and which fail
#   ./scripts/qa-parity.sh g2 --scenario shift-click-range
#                                             only that scenario's runs of the selection
#   ./scripts/qa-parity.sh mutations          each patch under tests/qa/parity/mutations must
#                                             make the scenario it is named for fail on Rune
# A run's artifacts are kept whether it passes or fails, in target/qa-artifacts (GTK) and
# target/qa-artifacts-rune (Rune), under <scenario>/<run>; STRATA_QA_KEEP_ARTIFACTS=0 keeps
# only a failed run's. Shares target/qa-container with qa.sh: one QA script at a time.
set -euo pipefail

unset DISPLAY WAYLAND_DISPLAY NOTIFY_SOCKET
repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
usage='Usage: scripts/qa-parity.sh <slice|g1|g2|g3> [--target gtk|rune] [--pending] [--scenario <name>]
       scripts/qa-parity.sh mutations'
selection="${1:-}"
if [[ "$selection" == mutations && $# -eq 1 ]]; then
  # Each patch is applied to this checkout, run and reverted: scripts/qa_parity_mutation.py.
  exec python3 "$repository/scripts/qa_parity_mutation.py"
fi
case "$selection" in slice|g1|g2|g3) shift ;; *) echo "$usage" >&2; exit 2 ;; esac
targets="gtk rune"
status=required
scenario=()
whole=1
while (($#)); do
  case "$1" in
    --target)
      case "${2:-}" in gtk|rune) targets="$2" ;; *) echo "$usage" >&2; exit 2 ;; esac
      shift 2
      ;;
    --pending)
      status=pending
      shift
      ;;
    --scenario)
      [[ -n "${2:-}" ]] || { echo "$usage" >&2; exit 2; }
      scenario=(--scenario "$2")
      whole=0
      shift 2
      ;;
    *) echo "$usage" >&2; exit 2 ;;
  esac
done
if [[ "$status" == pending ]]; then
  # Pending runs pass on GTK already; the survey asks what they do on Rune.
  targets=rune
fi

if [[ "$targets" != "gtk rune" || "$status" != required ]]; then whole=0; fi
mapfile -t runs < <(python3 "$repository/scripts/qa_parity.py" "$selection" --status "$status" "${scenario[@]}")
if ((${#runs[@]} == 0)); then
  if ((${#scenario[@]})); then
    # A scenario is named to run it: a name with no run is a mistake, not an empty gate.
    echo "The ledger has no $status run of ${scenario[1]} for $selection." >&2
    exit 2
  fi
  echo "The ledger has no $status run for $selection: nothing was run."
  exit 0
fi

# shellcheck source=lib/qa-container.sh
source "$repository/scripts/lib/qa-container.sh"
qa_container_prepare
qa_container_options --rm
qa_container_mounts
code=0
"$engine" run "${options[@]}" "${mounts[@]}" \
  --env "STRATA_QA_KEEP_ARTIFACTS=${STRATA_QA_KEEP_ARTIFACTS:-1}" \
  --env "STRATA_QA_MAIN_IO=${STRATA_QA_MAIN_IO:-}" \
  --env "STRATA_UI_STALL_MS=${STRATA_UI_STALL_MS:-}" \
  "$image" bash -euc '
    selection="$1" status="$2" targets="$3" whole="$4"
    shift 4
    mkdir -p "$HOME" "$CARGO_HOME"
    # The tmpfs hides the X socket directory of the image; the xvfb.log owner warning is harmless.
    mkdir -p -m 1777 /tmp/.X11-unix
    rustc --version
    # The Rune side builds in a cargo run of its own (docs/workspace.md).
    cargo build --locked -p strata-rune --features qa
    export STRATA_QA_RUNE_BINARY="$CARGO_TARGET_DIR/debug/strata-rune"
    runner=(cargo test --locked --features qa,invariant-checks --test qa)
    "${runner[@]}" --no-run
    failed=0
    for target in $targets; do
      echo "== $selection on $target: $# $status run(s) =="
      output="$(mktemp)"
      STRATA_QA_TARGET="$target" "${runner[@]}" -- --include-ignored --exact "$@" 2>&1 \
        | tee "$output"
      code="${PIPESTATUS[0]}"
      passed="$(grep -Ec "^(PASS|KNOWN BUG) " "$output" || true)"
      planned="$(grep -c "^RUN " "$output" || true)"
      echo "== $target: $passed of $# passed =="
      if [[ "$status" == pending ]]; then
        # A survey fails only when it could not run: its failures are what it is for.
        if ((planned == 0)); then failed=1; fi
        grep -E "^PASS " "$output" | sed "s/^PASS /PASSES ON RUNE, STILL PENDING: /" || true
      elif ((code != 0 || passed != $#)); then
        failed=1
      fi
    done
    if [[ "$selection" == slice && "$whole" == 1 ]]; then
      echo "== the runner tests, the ledger check among them, and the whole GTK suite =="
      output="$(mktemp)"
      STRATA_QA_KEEP_ARTIFACTS=0 STRATA_QA_TARGET=gtk "${runner[@]}" -- --include-ignored 2>&1 \
        | tee "$output"
      code="${PIPESTATUS[0]}"
      if ((code != 0)) || ! grep -Eq "^(PASS|KNOWN BUG) " "$output"; then failed=1; fi
      echo "Not run here: ./scripts/qa-mutation.sh, the GTK mutation patches of the exit check."
    fi
    exit "$failed"
  ' bash "$selection" "$status" "$targets" "$whole" "${runs[@]}" || code=$?
if [[ "$selection" == slice && "$whole" == 1 ]]; then
  echo "== the Rune mutation patches =="
  python3 "$repository/scripts/qa_parity_mutation.py" || code=1
fi
exit "$code"
