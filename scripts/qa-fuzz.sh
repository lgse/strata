#!/usr/bin/env bash
# The fuzz driver, inside the pinned test image: a seed picks a sequence of actions from an
# area's alphabet, the runner replays it on a fresh fixture, and what a user could observe
# after each step is compared between replays (docs/testing.md, "Fuzzing").
#   ./scripts/qa-fuzz.sh run --alphabet calibration --seeds 1..20
#                                             generate each seed's sequence, replay it twice
#                                             and compare the replays: the smoke tier
#   ./scripts/qa-fuzz.sh run --alphabet calibration --seeds 7 --length 80 --repeats 3
#   ./scripts/qa-fuzz.sh replay               every kept sequence of tests/qa/fuzz/corpus
#                                             that is not parked
#   ./scripts/qa-fuzz.sh replay --parked --repeats 24
#                                             a survey: the parked ones, on purpose
#   ./scripts/qa-fuzz.sh shrink --alphabet calibration --seeds 7
#                                             shrink one seed's divergence and confirm it
#                                             three times out of three; --corpus keeps it
#   ./scripts/qa-fuzz.sh generate --alphabet calibration --seeds 7
#                                             print the sequence, a scenario file
#   ./scripts/qa-fuzz.sh scenario <file>      run a sequence file as the scenario it is
#   ./scripts/qa-fuzz.sh compare <stream> <stream>
#                                             compare two kept streams of one sequence
#   ./scripts/qa-fuzz.sh check                read the alphabets, the map, the mask and the
#                                             corpus, and print their counts
#   --target gtk|rune                         the target every replay starts (default gtk)
# The commands and their other selections are the runner's (tests/qa/fuzz/options.rs): it
# refuses what it does not know and prints them. A file is named by its path under the
# checkout. Artifacts go to target/qa-fuzz/<area>/<sequence>.
# Shares target/qa-container with qa.sh, so one QA script at a time. The script takes no
# lock of its own: on a machine several sessions share, the caller holds theirs.
set -euo pipefail

unset DISPLAY WAYLAND_DISPLAY NOTIFY_SOCKET
repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
usage='Usage: scripts/qa-fuzz.sh <command> [selections] [--target gtk|rune]
See the head of this script, and docs/testing.md, "Fuzzing".'
case "${1:-}" in
  ""|-*) echo "$usage" >&2; exit 2 ;;
esac
target=gtk
arguments=()
while (($#)); do
  case "$1" in
    --target)
      case "${2:-}" in gtk|rune) target="$2" ;; *) echo "$usage" >&2; exit 2 ;; esac
      shift 2
      ;;
    --target=*)
      target="${1#--target=}"
      case "$target" in gtk|rune) ;; *) echo "$usage" >&2; exit 2 ;; esac
      shift
      ;;
    *)
      arguments+=("$1")
      shift
      ;;
  esac
done

# shellcheck source=lib/qa-container.sh
source "$repository/scripts/lib/qa-container.sh"
qa_container_prepare
qa_container_options --rm
qa_container_mounts
exec "$engine" run "${options[@]}" "${mounts[@]}" \
  --env "STRATA_QA_TARGET=$target" \
  "$image" bash -euc '
    mkdir -p "$HOME" "$CARGO_HOME"
    # An empty cache takes the crates the image holds; a used one is left alone.
    python3 scripts/dependency_seed.py
    # The tmpfs hides the X socket directory of the image; the xvfb.log owner warning is harmless.
    mkdir -p -m 1777 /tmp/.X11-unix
    rustc --version
    if [[ "$STRATA_QA_TARGET" == rune ]]; then
      # The Rune side builds in a cargo run of its own (docs/workspace.md).
      cargo build --locked -p strata-rune --features qa
      export STRATA_QA_RUNE_BINARY="$CARGO_TARGET_DIR/debug/strata-rune"
    fi
    # --fuzz first sends the runner to the fuzz driver; it is an option, so that no test
    # filter can be taken for it.
    cargo test --locked --features qa,invariant-checks --test qa -- --fuzz "$@"
  ' bash "${arguments[@]}"
