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
uris=
restore() {
    cp "$backup" "$sources"
    rm -f "$backup" "$lists/${mirror_prefix}"_*
    [ -z "$uris" ] || rm -f "$uris"
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
    printf 'Mirror lacks pinned packages; trying the authenticated Launchpad archive fallback.\n' >&2
    uris=$(mktemp "$sources.strata-uris-XXXXXX")
    if apt-get --print-uris --download-only -o Acquire::ForceHash=SHA256 \
        install --yes --no-install-recommends "$@" > "$uris"; then
        archives="$root/var/cache/apt/archives"
        mkdir -p "$archives/partial"
        while read -r uri cached _size hash _extra; do
            case "$uri" in
                "'https://archive.ubuntu.com/ubuntu/pool/"*".deb'") ;;
                *) continue ;;
            esac
            remote=${uri##*/}
            remote=${remote%\'}
            for name in "$remote" "$cached"; do
                case "$name" in
                    ''|*[!a-zA-Z0-9._+~:%-]*) fail 'invalid package download filename' ;;
                esac
            done
            case "$hash" in
                SHA256:*) checksum=${hash#SHA256:} ;;
                *) fail 'APT did not provide a SHA256 package identity' ;;
            esac
            [ "${#checksum}" -eq 64 ] || fail 'invalid package SHA256 length'
            case "$checksum" in *[!0-9a-f]*) fail 'invalid package SHA256' ;; esac
            if [ -f "$archives/$cached" ] && \
                printf '%s  %s\n' "$checksum" "$archives/$cached" | sha256sum --check --status; then
                continue
            fi
            # Launchpad retains superseded binaries. The signed snapshot, not
            # the mirror, supplies the mandatory hash for every downloaded byte.
            temporary="$archives/partial/$cached"
            if "$root/usr/lib/apt/apt-helper" -o Acquire::Retries=0 \
                -o Acquire::https::Timeout=20 -o Acquire::http::Timeout=20 download-file \
                "https://launchpad.net/ubuntu/+archive/primary/+files/$remote" \
                "$temporary" "$hash"; then
                install -m 0644 "$temporary" "$archives/$cached"
                rm -f "$temporary"
            else
                printf 'Archive fallback unavailable for %s; completing downloads from the snapshot.\n' "$cached" >&2
            fi
        done < "$uris"
    fi
fi
# Install once against the original snapshot, retaining only downloads that APT
# verifies against its signed indexes; never retry a failed maintainer script.
cp "$backup" "$sources"
apt-get install --yes --no-install-recommends "$@"
