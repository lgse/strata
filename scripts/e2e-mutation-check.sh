#!/usr/bin/env bash
# Proves the end-to-end suite would catch a regression.
#
# Applies one deliberate defect at a time from tests/e2e/mutations, rebuilds,
# and asserts that the scenarios covering that workflow fail. The working tree
# is restored afterwards.
#
#   ./scripts/e2e-mutation-check.sh              # every mutation
#   ./scripts/e2e-mutation-check.sh clipboard    # one of them
set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mutations="$repository/tests/e2e/mutations"

declare -A SCENARIOS=(
  [drag-and-drop]="tests/e2e/scenarios/test_drag_and_drop.py"
  [clipboard]="tests/e2e/scenarios/test_clipboard.py"
  [keyboard-navigation]="tests/e2e/scenarios/test_keyboard_navigation.py"
  [click-modes]="tests/e2e/scenarios/test_click_modes.py"
  [view-switching]="tests/e2e/scenarios/test_view_switching.py"
)

selected=("$@")
if ((${#selected[@]} == 0)); then
  mapfile -t selected < <(printf '%s\n' "${!SCENARIOS[@]}" | sort)
fi

if [[ -n "$(git -C "$repository" status --porcelain -- src)" ]]; then
  echo "src/ has uncommitted changes; commit or stash them first" >&2
  exit 1
fi

if [[ -n "${STRATA_BINARY:-}" ]]; then
  echo "STRATA_BINARY must be unset so mutations exercise the rebuilt binary" >&2
  exit 1
fi

cd "$repository"
reports="$repository/target/e2e-mutations"
mkdir -p "$reports"

restore() {
  git restore --worktree -- src
}
trap restore EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

failures=0
for name in "${selected[@]}"; do
  patch="$mutations/$name.patch"
  scenario="${SCENARIOS[$name]:-}"
  if [[ -z "$scenario" || ! -f "$patch" ]]; then
    echo "unknown mutation: $name" >&2
    exit 2
  fi

  echo "== $name: checking the unmodified scenario"
  "$repository/scripts/e2e.sh" -q "$repository/$scenario" >"$reports/$name-baseline.log" 2>&1 || {
    echo "   baseline failed; see $reports/$name-baseline.log" >&2
    exit 1
  }

  echo "== $name: applying $patch"
  git -C "$repository" apply -p1 "$patch"
  if ! cargo build --manifest-path "$repository/Cargo.toml" --bin strata >/dev/null; then
    echo "   the mutated tree does not build" >&2
    restore
    exit 1
  fi

  echo "== $name: expecting $scenario to fail"
  report="$reports/$name.xml"
  rm -f "$report"
  result=0
  "$repository/scripts/e2e.sh" -q -x --junitxml="$report" "$repository/$scenario" \
    >"$reports/$name.log" 2>&1 || result=$?
  if python3 "$repository/scripts/e2e_mutation_result.py" "$report" "$result"; then
    echo "   detected"
  else
    echo "   NOT DETECTED or infrastructure failure; see $reports/$name.log" >&2
    failures=$((failures + 1))
  fi
  restore
done

cargo build --manifest-path "$repository/Cargo.toml" --bin strata >/dev/null

if ((failures)); then
  echo "$failures mutation(s) went undetected" >&2
  exit 1
fi
echo "every mutation was detected"
