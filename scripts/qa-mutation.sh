#!/usr/bin/env bash
# Checks each `--qa` scenario with a `mutation` passes unmodified and fails under its
# patch from tests/qa/mutations; reports go to target/qa-mutation.
#
#   ./scripts/qa-mutation.sh                     every scenario with a mutation
#   ./scripts/qa-mutation.sh clipboard           the scenario a patch belongs to
#   ./scripts/qa-mutation.sh copy-leaves-source  a scenario by its name
set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$repository/scripts/qa_mutation.py" "$@"
