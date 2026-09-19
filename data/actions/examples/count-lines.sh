# SPDX-License-Identifier: MIT

set -euo pipefail
command -v wc >/dev/null 2>&1 || {
    printf 'wc was not found on your PATH. Install coreutils.\n' >&2
    exit 1
}

# STRATA_ACTION_PATHS contains NUL-delimited absolute paths, not shell words.
# Whole-selection mode produces one total. wc -l counts newline characters.
total=0
files=0
while IFS= read -r -d '' path; do
    [[ -f "$path" ]] || { printf 'Not a regular file: %q\n' "$path" >&2; exit 1; }
    lines=$(wc -l < "$path")
    total=$((total + lines))
    files=$((files + 1))
    printf '%s lines\t%q\n' "$lines" "$path"
done < "$STRATA_ACTION_PATHS"
printf 'Total: %s lines across %s files\n' "$total" "$files"
