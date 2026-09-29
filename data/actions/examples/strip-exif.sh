# SPDX-License-Identifier: MIT

set -euo pipefail
umask 077
command -v exiftool >/dev/null 2>&1 || {
    printf 'exiftool was not found on your PATH. Install ExifTool.\n' >&2
    exit 1
}

# Removing all writable metadata can remove orientation and color profiles.
while IFS= read -r -d '' path; do
    [[ -f "$path" && ! -L "$path" ]] || {
        printf 'Not a regular, non-symlink file: %q\n' "$path" >&2
        exit 1
    }
    # Retain a complete, private original before allowing in-place modification.
    backup_dir=$(mktemp -d -- "${path%/*}/strata-original-XXXXXX")
    backup="$backup_dir/${path##*/}"
    cat -- "$path" > "$backup"
    printf 'Original retained at %q\n' "$backup"
    exiftool -overwrite_original -all= -- "$path"
    printf 'Removed writable metadata from %q\n' "$path"
done < "$STRATA_ACTION_PATHS"
