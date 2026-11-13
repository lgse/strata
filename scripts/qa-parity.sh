#!/usr/bin/env bash
# Runs the runs of the parity ledger, tests/qa/parity/<folder>.toml, on both QA targets
# inside the pinned test image: the GTK application and strata-rune (docs/testing.md, "The
# Rune target and the parity ledger"). A selection is a gate of the slice, a scenario
# folder, a work package or all:
#   ./scripts/qa-parity.sh selection          the required runs of tests/qa/scenarios/selection,
#                                             on each target
#   ./scripts/qa-parity.sh A7                 the required runs package A7 owns
#   ./scripts/qa-parity.sh all                every required run of the ledger
#   ./scripts/qa-parity.sh g1                 gate G1's required runs, on each target
#   ./scripts/qa-parity.sh slice              the slice's exit check: every gate's required
#                                             runs on each target, the runner's own tests and
#                                             the whole GTK suite, then the Rune mutation
#                                             patches and the GTK ones
#   ./scripts/qa-parity.sh A7 --counts        the selection's runs by status and reason, by
#                                             host, view and package; nothing is run
#   ./scripts/qa-parity.sh g2 --target rune   one target only: gtk or rune
#   ./scripts/qa-parity.sh A7 --twins         the selection's twins: exception, reference,
#                                             what differs and the step counts; nothing is run
#   ./scripts/qa-parity.sh A7 --pending       a survey: the selection's pending runs on the
#                                             Rune target, listing which pass and which fail
#   ./scripts/qa-parity.sh A7 --parallel      a survey: the selection's parallel runs from the
#                                             shared file on the Rune target; one that passes
#                                             has an exception that lapsed
#   ./scripts/qa-parity.sh g2 --scenario shift-click-range
#                                             only that scenario's runs of the selection
#   ./scripts/qa-parity.sh mutations          each patch under tests/qa/parity/mutations must
#                                             make the scenario it is named for fail on Rune;
#                                             <scenario>-rune.patch is a twin's
#   ./scripts/qa-parity.sh mutations shift-click-range
#                                             only the patches named, by scenario,
#                                             <scenario>.patch or path: one proven alone
#   ./scripts/qa-parity.sh gtk-mutations      each patch under tests/qa/mutations must make its
#                                             scenario fail on GTK, but the ones the ledger
#                                             names as stale, which must be stale still
# A required run passes on both targets from one file. A parallel run is at parity by another
# route, and is run with the required ones: its scenario must pass on GTK, and its twin,
# <scenario>-rune/<run>, on Rune (docs/testing.md, "Twins"). --scenario takes either name.
#
# A run's artifacts are kept whether it passes or fails, in target/qa-artifacts (GTK) and
# target/qa-artifacts-rune (Rune), under <scenario>/<run>; STRATA_QA_KEEP_ARTIFACTS=0 keeps
# only a failed run's. Shares target/qa-container with qa.sh: one QA script at a time.
#
# The ledger's allowances, tests/qa/parity/slice.toml, are slice's alone. A run it names as
# flaky may fail in the whole GTK suite: slice says so and keeps the failed run's artifacts
# in target/qa-parity/allowance. Any other failure there fails slice. Every allowance used
# or refused is listed when slice ends.
set -euo pipefail

unset DISPLAY WAYLAND_DISPLAY NOTIFY_SOCKET
repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
usage='Usage: scripts/qa-parity.sh <selection> [--target gtk|rune] [--pending|--parallel] [--scenario <name>]
       scripts/qa-parity.sh <selection> --counts
       scripts/qa-parity.sh <selection> --twins
       scripts/qa-parity.sh mutations [<patch>...]
       scripts/qa-parity.sh gtk-mutations
A selection is slice, g1, g2, g3, all, a folder of tests/qa/scenarios or a package id.'
selection="${1:-}"
# Each patch is applied to this checkout, run and reverted: scripts/qa_parity_mutation.py.
if [[ "$selection" == mutations ]]; then
  shift
  exec python3 "$repository/scripts/qa_parity_mutation.py" rune "$@"
fi
if [[ $# -eq 1 && "$selection" == gtk-mutations ]]; then
  exec python3 "$repository/scripts/qa_parity_mutation.py" gtk
fi
case "$selection" in ""|-*|mutations|gtk-mutations) echo "$usage" >&2; exit 2 ;; *) shift ;; esac
targets="gtk rune"
status=required
scenario=()
whole=1
counts=0
narrowed=0
while (($#)); do
  case "$1" in
    --target)
      case "${2:-}" in gtk|rune) targets="$2" ;; *) echo "$usage" >&2; exit 2 ;; esac
      narrowed=1
      shift 2
      ;;
    --pending|--parallel)
      status="${1#--}"
      narrowed=1
      shift
      ;;
    --scenario)
      [[ -n "${2:-}" ]] || { echo "$usage" >&2; exit 2; }
      scenario=(--scenario "$2")
      whole=0
      narrowed=1
      shift 2
      ;;
    --counts|--twins)
      counts=1
      report="$1"
      shift
      ;;
    *) echo "$usage" >&2; exit 2 ;;
  esac
done
if ((counts)); then
  if ((narrowed)); then
    echo "$report is of the whole selection: it takes no --target, --pending, --parallel or --scenario." >&2
    exit 2
  fi
  # The ledger and the scenario files alone answer: no image, no build.
  exec python3 "$repository/scripts/qa_parity.py" "$selection" "$report"
fi
if [[ "$status" != required ]]; then
  # A survey asks what runs do on Rune from the shared file: pending runs, which pass on GTK
  # already, or parallel runs, whose twin stands in for them there.
  targets=rune
fi

if [[ "$targets" != "gtk rune" || "$status" != required ]]; then whole=0; fi
# The reader checks the whole ledger and its twins first and refuses one with a fault, as the
# runner's tests would; then a selection it does not have, and a run it cannot start here.
list() { python3 "$repository/scripts/qa_parity.py" "$selection" "$@" "${scenario[@]}"; }
gtk_runs="" rune_runs="" parallel=0 kind="$status"
if [[ "$status" != required ]]; then
  rune_runs="$(list --status "$status")" || exit 2
else
  # What must pass differs by target: a parallel run is its scenario on GTK, its twin on Rune.
  kind="required or parallel"
  for target in $targets; do
    listed="$(list --on "$target")" || exit 2
    printf -v "${target}_runs" '%s' "$listed"
  done
  listed="$(list --status parallel)" || exit 2
  if [[ -n "$listed" ]]; then parallel="$(wc -l <<< "$listed")"; fi
fi
first="${targets%% *}_runs"
if [[ -z "${!first}" ]]; then
  if ((${#scenario[@]})); then
    # A scenario is named to run it: a name with no run is a mistake, not an empty gate.
    echo "The ledger has no $kind run of ${scenario[1]} for $selection." >&2
    exit 2
  fi
  echo "The ledger has no $kind run for $selection: nothing was run."
  exit 0
fi

# shellcheck source=lib/qa-container.sh
source "$repository/scripts/lib/qa-container.sh"
qa_container_prepare
qa_container_options --rm
qa_container_mounts
# Every allowance slice uses is noted here, and listed again when it ends.
notes=target/qa-parity/allowances.txt
if [[ "$selection" == slice && "$whole" == 1 ]]; then
  # An earlier run's notes and kept bundles would read as this one's.
  rm -rf "$repository/target/qa-parity"
  mkdir -p "$repository/target/qa-parity"
  : > "$repository/$notes"
fi
code=0
"$engine" run "${options[@]}" "${mounts[@]}" \
  --env "STRATA_QA_KEEP_ARTIFACTS=${STRATA_QA_KEEP_ARTIFACTS:-1}" \
  --env "STRATA_QA_MAIN_IO=${STRATA_QA_MAIN_IO:-}" \
  --env "STRATA_UI_STALL_MS=${STRATA_UI_STALL_MS:-}" \
  "$image" bash -euc '
    selection="$1" status="$2" targets="$3" whole="$4" notes="/workspace/$5" parallel="$6"
    declare -A listed=([gtk]="$7" [rune]="$8")
    mkdir -p "$HOME" "$CARGO_HOME"
    # An empty cache takes the crates the image holds; a used one is left alone.
    python3 scripts/dependency_seed.py
    # The tmpfs hides the X socket directory of the image; the xvfb.log owner warning is harmless.
    mkdir -p -m 1777 /tmp/.X11-unix
    rustc --version
    # The Rune side builds in a cargo run of its own (docs/workspace.md).
    cargo build --locked -p strata-rune --features qa
    export STRATA_QA_RUNE_BINARY="$CARGO_TARGET_DIR/debug/strata-rune"
    runner=(cargo test --locked --features qa,invariant-checks --test qa)
    "${runner[@]}" --no-run
    failed=0
    # A run the ledger names as flaky failed in the whole GTK suite. It does not fail slice
    # and is not run again: alone it fails more often than in the suite, so a rerun says
    # nothing (docs/rune/slice/README.md, decision 27). Its bundle is kept and it is noted.
    allowed() {
      local kept="/workspace/target/qa-parity/allowance/$1"
      mkdir -p "$(dirname "$kept")"
      cp -a "/workspace/target/qa-artifacts/$1" "$kept" 2>/dev/null || true
      echo "ALLOWANCE USED: $1 failed in the whole GTK suite; the ledger names it flaky before the slice." \
        | tee -a "$notes"
    }
    for target in $targets; do
      mapfile -t runs <<< "${listed[$target]}"
      count="${#runs[@]}"
      heading="$count $status run(s)"
      if ((parallel)) && [[ "$status" == required ]]; then
        heading="$count run(s): $((count - parallel)) required and $parallel parallel, the scenario on gtk and its twin on rune"
      fi
      echo "== $selection on $target: $heading =="
      output="$(mktemp)"
      STRATA_QA_TARGET="$target" "${runner[@]}" -- --include-ignored --exact "${runs[@]}" 2>&1 \
        | tee "$output"
      code="${PIPESTATUS[0]}"
      passed="$(grep -Ec "^(PASS|KNOWN BUG) " "$output" || true)"
      planned="$(grep -c "^RUN " "$output" || true)"
      echo "== $target: $passed of $count passed =="
      if [[ "$status" != required ]]; then
        # A survey fails only when it could not run: its failures are what it is for.
        if ((planned == 0)); then failed=1; fi
        found="PASSES ON RUNE, STILL PENDING: "
        if [[ "$status" == parallel ]]; then found="PASSES ON RUNE FROM THE SHARED FILE, STILL PARALLEL: "; fi
        grep -E "^PASS " "$output" | sed "s/^PASS /$found/" || true
      elif ((code != 0 || passed != count)); then
        failed=1
      fi
    done
    if [[ "$selection" == slice && "$whole" == 1 ]]; then
      echo "== the runner tests, the ledger check among them, and the whole GTK suite =="
      output="$(mktemp)"
      STRATA_QA_KEEP_ARTIFACTS=0 STRATA_QA_TARGET=gtk "${runner[@]}" -- --include-ignored 2>&1 \
        | tee "$output"
      code="${PIPESTATUS[0]}"
      if ! grep -Eq "^(PASS|KNOWN BUG) " "$output"; then
        failed=1
      elif ((code != 0)); then
        # Only the runs the ledger names as flaky may fail here.
        refusal="$(mktemp)"
        if flaky="$(python3 scripts/qa_parity.py slice --failed-in "$output" 2> "$refusal")"; then
          for name in $flaky; do allowed "$name"; done
        else
          tee -a "$notes" < "$refusal"
          failed=1
        fi
      fi
    fi
    exit "$failed"
  ' bash "$selection" "$status" "$targets" "$whole" "$notes" "$parallel" "$gtk_runs" "$rune_runs" \
  || code=$?
if [[ "$selection" == slice && "$whole" == 1 ]]; then
  echo "== the Rune mutation patches =="
  python3 "$repository/scripts/qa_parity_mutation.py" rune || code=1
  echo "== the GTK mutation patches =="
  python3 "$repository/scripts/qa_parity_mutation.py" gtk --notes "$repository/$notes" || code=1
  if [[ -s "$repository/$notes" ]]; then
    echo "== allowances, from tests/qa/parity/slice.toml =="
    cat "$repository/$notes"
  else
    echo "== no allowance was used =="
  fi
  echo "== slice exits $code =="
fi
exit "$code"
