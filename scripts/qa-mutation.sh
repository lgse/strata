#!/usr/bin/env bash
# Proves each mutation patch under tests/qa/mutations breaks a behaviour its QA scenario
# catches; reports go to target/qa-mutation.
#
#   ./scripts/qa-mutation.sh                                 every patch under tests/qa/mutations
#   ./scripts/qa-mutation.sh shift-click-range               every patch for a scenario
#   ./scripts/qa-mutation.sh pdf-zoom-keeps-text-size.window-wheel  one labelled patch
set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$repository/scripts/qa_mutation.py" "$@"
