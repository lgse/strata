#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Shared, non-publishing release producer. Never strip Cargo's cached executables.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
: "${TARGET:?Set the native release target}"
: "${VERSION:?Set the release version without v}"
: "${SOURCE_SHA:?Set the full source commit}"
[[ "$TARGET" == x86_64-unknown-linux-gnu || "$TARGET" == aarch64-unknown-linux-gnu ]]
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc|nightly)\.[0-9.]+)?$ ]]
[[ "$SOURCE_SHA" =~ ^[a-f0-9]{40}$ ]]
out=${STRATA_RELEASE_OUTPUT:-dist}
mkdir -p "$out"
out=$(cd "$out" && pwd -P)
package="strata-${VERSION}-${TARGET}"
[[ ! -e "$out/$package" ]] || { echo 'Use a fresh release output directory.' >&2; exit 1; }
export STRATA_BUILD_COMMIT="$SOURCE_SHA" STRATA_RELEASE_TAG="v$VERSION"
case "$VERSION" in
    *-alpha.*) export STRATA_BUILD_KIND=alpha ;;
    *-beta.*) export STRATA_BUILD_KIND=beta ;;
    *-rc.*) export STRATA_BUILD_KIND=rc ;;
    *-nightly.*) export STRATA_BUILD_KIND=nightly ;;
    *) export STRATA_BUILD_KIND=stable ;;
esac
export CARGO_PROFILE_RELEASE_DEBUG=line-tables-only
binaries="${CARGO_TARGET_DIR:-target}/$TARGET/release"
cargo build --release --locked -p strata-media-helper --target "$TARGET"
helper="$out/strata-media-helper-${VERSION}-${TARGET}"
install -m 755 "$binaries/strata-media-helper" "$helper"
symbols="$helper.debug"
objcopy --only-keep-debug --compress-debug-sections=zlib "$helper" "$symbols"
strip --strip-unneeded "$helper"
objcopy --add-gnu-debuglink="$symbols" "$helper"
STRATA_RELEASE_BUILD=1 STRATA_MEDIA_HELPER_BUNDLE="$helper" \
    cargo build --release --locked -p strata --target "$TARGET"
mkdir "$out/$package"
install -m 755 "$binaries/strata" "$out/$package/strata"
install -m 755 "$helper" "$out/$package/strata-media-helper"
objcopy --only-keep-debug --compress-debug-sections=zlib "$out/$package/strata" "$out/$package.debug"
strip --strip-unneeded "$out/$package/strata"
objcopy --add-gnu-debuglink="$out/$package.debug" "$out/$package/strata"
printf '%s\n' "$SOURCE_SHA" > "$out/$package/SOURCE_COMMIT"
cp README.md LICENSE THIRD_PARTY_LICENSES.md data/licenses/UnRAR.txt "$out/$package/"
mkdir "$out/$package/docs" "$out/$package/portal"
cp docs/portal-file-chooser.md docs/preview-sandbox.md docs/media-helper-bundles.md "$out/$package/docs/"
cp data/portal/* "$out/$package/portal/"
install -m 644 data/io.github.lgse.Strata.desktop data/io.github.lgse.Strata.FileManager1.service \
    data/icons/scalable/apps/io.github.lgse.Strata.svg "$out/$package/"
python3 -I scripts/release_bundle.py "$out/$package" --release-tag "v$VERSION" --target "$TARGET" --commit "$SOURCE_SHA"
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -C "$out" -czf "$out/$package.tar.gz" "$package"
(cd "$out" && sha256sum "$package.tar.gz" > "$package.tar.gz.sha256")
(cd "$out" && sha256sum "$package.debug" > "$package.debug.sha256")
(cd "$out" && sha256sum "$(basename "$symbols")" > "$(basename "$symbols").sha256")
