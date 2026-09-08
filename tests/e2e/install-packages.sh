#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

root=${STRATA_E2E_APT_ROOT:-}
sources="$root/etc/apt/sources.list"
lists="$root/var/lib/apt/lists"
snapshot=${STRATA_E2E_SNAPSHOT_URL:?the dated snapshot URL is required}
mirror=https://archive.ubuntu.com/ubuntu
prefix=$(printf '%s' "${snapshot#*://}" | tr / _)
mirror_prefix=archive.ubuntu.com_ubuntu

fail() { printf 'E2E packages: %s\n' "$1" >&2; exit 1; }
[ "$#" -gt 0 ] || fail 'no packages requested'
grep -F "$snapshot" "$sources" >/dev/null || fail 'sources do not use the dated snapshot'

release=false
packages=false
for file in "$lists/${prefix}"_*; do
    [ -f "$file" ] || continue
    case "$file" in
        *_InRelease) release=true ;;
        *_Packages*) packages=true ;;
    esac
done
$release && $packages || fail 'fetch the signed snapshot indexes before installing packages'
for file in "$lists/${mirror_prefix}"_*; do
    [ ! -e "$file" ] || fail 'refusing to replace existing mirror indexes'
done

backup=$(mktemp "$sources.strata-XXXXXX")
cp "$sources" "$backup"
restore() {
    cp "$backup" "$sources"
    rm -f "$backup" "$lists/${mirror_prefix}"_*
}
trap restore EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

# Only pool downloads move: APT still resolves and verifies against the exact
# signed snapshot indexes. Never refresh indexes from the moving mirror.
for file in "$lists/${prefix}"_*; do
    [ -f "$file" ] || continue
    cp "$file" "$lists/$mirror_prefix${file#"$lists/$prefix"}"
done
while IFS= read -r line; do
    case "$line" in
        *"$snapshot"*) printf '%s%s%s\n' "${line%%"$snapshot"*}" "$mirror" "${line#*"$snapshot"}" ;;
        *) printf '%s\n' "$line" ;;
    esac
done < "$backup" > "$sources"

if ! apt-get --download-only install --yes --no-install-recommends "$@"; then
    printf 'Mirror lacks pinned packages; completing downloads from the snapshot.\n' >&2
fi
# Superseded versions may exist only in the snapshot. Install once, retaining
# verified mirror downloads but never retrying a failed maintainer script.
cp "$backup" "$sources"
apt-get install --yes --no-install-recommends "$@"
