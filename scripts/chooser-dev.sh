#!/usr/bin/env bash
set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repository"

chooser_case="${CHOOSER_CASE:-save}"
if [[ -n "${CHOOSER_ARGS+x}" ]]; then
  read -r -a chooser_args <<< "$CHOOSER_ARGS"
else
  chooser_args=(--choices)
fi

cargo build
GTK_A11Y=none python3 scripts/portal-test.py "$chooser_case" \
  --binary target/debug/strata "${chooser_args[@]}"
