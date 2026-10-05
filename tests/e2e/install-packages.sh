#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu

root=${STRATA_E2E_PACMAN_ROOT:-}
snapshot=${STRATA_E2E_ARCH_SNAPSHOT:?the dated Arch snapshot is required}
mirrorlist="$root/etc/pacman.d/mirrorlist"
configuration="$root/etc/pacman.conf"
sync="$root/var/lib/pacman/sync"

fail() { printf 'E2E packages: %s\n' "$1" >&2; exit 1; }
case "$snapshot" in
    ????/??/??) ;;
    *) fail 'snapshot must use YYYY/MM/DD' ;;
esac
case "$snapshot" in *[!0-9/]*) fail 'snapshot contains invalid characters' ;; esac
[ -f "$configuration" ] || fail 'pacman configuration is missing'
grep -Eq '^[[:space:]]*SigLevel[[:space:]]*=[[:space:]]*Required([[:space:]]+DatabaseOptional)?[[:space:]]*$' \
    "$configuration" || fail 'pacman must require package signatures'

server="Server = https://archive.archlinux.org/repos/$snapshot/\$repo/os/\$arch"
case "${1:-}" in
    indexes)
        [ "$#" -eq 1 ] || fail 'indexes takes no package names'
        mkdir -p "$(dirname "$mirrorlist")"
        printf '%s\n' "$server" > "$mirrorlist"
        pacman -Syy --noconfirm
        ;;
    install)
        shift
        [ "$#" -gt 0 ] || fail 'install requires package names'
        [ "$(cat "$mirrorlist" 2>/dev/null)" = "$server" ] || fail 'mirror does not use the dated snapshot'
        [ -f "$sync/core.db" ] && [ -f "$sync/extra.db" ] || fail 'fetch signed snapshot indexes before installing packages'
        pacman -Su --noconfirm --needed "$@"
        ;;
    *) fail 'usage: install-packages.sh indexes|install [packages...]' ;;
esac
