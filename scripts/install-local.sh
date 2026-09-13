#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail
export PATH=/usr/bin:/bin
cd "$(dirname "$0")/.."
STRATA_INSTALLER_TESTING=1 source ./install.sh
BIN_PATH="${BIN_DIR:-$HOME/.local/bin}/strata"
export XDG_DATA_HOME="${DATA_HOME:-${XDG_DATA_HOME:-$HOME/.local/share}}"
if command -v pacman >/dev/null 2>&1 && pacman --query --owns --quiet -- "$BIN_PATH" >/dev/null 2>&1; then
  die "This installation belongs to a package manager. Use its update command instead."
fi
TEMP_DIR=$(private_install_tempdir)
trap 'rm -rf -- "$TEMP_DIR"' EXIT
release_tag=${STRATA_RELEASE_TAG:-v$(python3 -I -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])')}
version=${release_tag#v}
case "$(uname -m)" in
  x86_64) target=x86_64-unknown-linux-gnu ;;
  aarch64) target=aarch64-unknown-linux-gnu ;;
  *) die "Unsupported local installation architecture" ;;
esac
build="${CARGO_TARGET_DIR:-target}${CARGO_BUILD_TARGET:+/$CARGO_BUILD_TARGET}/release"
package="strata-$version-$target"
staging="$TEMP_DIR/$package"
mkdir -p "$staging/portal"
install -m 755 "$build/strata" "$build/strata-media-helper" "$staging/"
cp README.md LICENSE THIRD_PARTY_LICENSES.md data/licenses/UnRAR.txt "$staging/"
cp data/io.github.lgse.Strata.desktop data/io.github.lgse.Strata.FileManager1.service data/icons/scalable/apps/io.github.lgse.Strata.svg "$staging/"
cp data/portal/* "$staging/portal/"
commit=$(git rev-parse HEAD)
printf '%s\n' "$commit" > "$staging/SOURCE_COMMIT"
python3 -I scripts/release_bundle.py "$staging" --release-tag "$release_tag" --target "$target" --commit "$commit"
tar -C "$TEMP_DIR" --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -cf - "$package" | gzip -n > "$TEMP_DIR/bundle.tar.gz"
installed=$(install_bundle "$TEMP_DIR/bundle.tar.gz" "$version" "$target" "$BIN_PATH")
install_desktop_entry "$installed" no
printf 'Installed the matching local bundle at %s\n' "$BIN_PATH"
