#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Score Rust files with the CodeScene CLI and list their function-level smells.
#
#   scripts/code-health.sh src/ui/window.rs src/ui/window/*.rs
#   scripts/code-health.sh --changed            # files changed against origin/main
#   scripts/code-health.sh --min 8 --changed    # fail when any file scores below 8
#   scripts/code-health.sh --json <files>       # raw cs output, one JSON object per line
#
# Needs a CodeScene personal access token in CS_ACCESS_TOKEN (CODE_SCENE_TOKEN is
# accepted as a fallback). Uses `cs` from PATH or STRATA_CS_CLI, otherwise
# downloads the pinned CLI into target/cs-cli once.
set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repository"

min=""
changed=0
json=0
files=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --min) min="$2"; shift 2 ;;
    --changed) changed=1; shift ;;
    --json) json=1; shift ;;
    -h|--help) sed -n '3,13p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) files+=("$1"); shift ;;
  esac
done

if [[ "$changed" == 1 ]]; then
  base="$(git merge-base HEAD origin/main 2>/dev/null || git merge-base HEAD main)"
  while IFS= read -r path; do
    [[ -f "$path" ]] && files+=("$path")
  done < <(git diff --name-only --diff-filter=ACMR "$base" HEAD -- '*.rs'; git diff --name-only --diff-filter=ACMR -- '*.rs')
fi
if [[ ${#files[@]} -eq 0 ]]; then
  echo 'code-health: no files to review' >&2
  exit 2
fi

export CS_ACCESS_TOKEN="${CS_ACCESS_TOKEN:-${CODE_SCENE_TOKEN:-}}"
if [[ -z "$CS_ACCESS_TOKEN" ]]; then
  echo 'code-health: set CS_ACCESS_TOKEN to a CodeScene personal access token' >&2
  exit 2
fi

cs="${STRATA_CS_CLI:-}"
if [[ -z "$cs" ]]; then
  if command -v cs >/dev/null 2>&1; then
    cs="$(command -v cs)"
  else
    cs="$repository/target/cs-cli/cs"
    if [[ ! -x "$cs" ]]; then
      mkdir -p "$repository/target/cs-cli"
      archive="$repository/target/cs-cli/cs.zip"
      curl -fsSL -o "$archive" https://downloads.codescene.io/enterprise/cli/cs-linux-amd64-latest.zip
      (cd "$repository/target/cs-cli" && unzip -o -q cs.zip && sha256sum -c --quiet cs.sha256 && chmod +x cs)
    fi
  fi
fi

mapfile -t files < <(printf '%s\n' "${files[@]}" | sort -u)

python3 - "$cs" "$json" "$min" "${files[@]}" <<'PY'
import json
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

cs, as_json, minimum, files = sys.argv[1], sys.argv[2] == "1", sys.argv[3], sys.argv[4:]
minimum = float(minimum) if minimum else None


def review(path):
    result = subprocess.run(
        [cs, "review", "--output-format", "json", path],
        capture_output=True,
        text=True,
        env={**os.environ, "CS_DISABLE_VERSION_CHECK": "1"},
    )
    if result.returncode != 0:
        raise SystemExit(f"code-health: cs review failed for {path}: {result.stderr.strip()}")
    return path, json.loads(result.stdout)


failed = []
with ThreadPoolExecutor(max_workers=8) as pool:
    for path, result in pool.map(review, files):
        score = result.get("score")
        if as_json:
            print(json.dumps({"path": path, **result}))
            continue
        if score is None:
            print(f"{path}: no scorable code")
            continue
        print(f"{path}: {score}")
        for finding in result.get("review", []):
            functions = finding.get("functions") or []
            if not functions:
                print(f"  {finding['category']}")
                continue
            functions.sort(key=lambda item: item["start-line"])
            shown = ", ".join(
                f"{item['title']} L{item['start-line']}-{item['end-line']}"
                + (f" ({item['details']})" if item.get("details") else "")
                for item in functions
            )
            print(f"  {finding['category']} ({len(functions)}): {shown}")
        if minimum is not None and score < minimum:
            failed.append((path, score))
for path, score in failed:
    print(f"below {minimum}: {path} scored {score}", file=sys.stderr)
if failed:
    sys.exit(1)
PY
