# Distribution packaging

Strata is packaged for Arch Linux through the AUR. Two packages are published from the same template, and both install `/usr/bin/strata`:

| Package | Tracks | Example `pkgver` |
| --- | --- | --- |
| `strata-bin` | Stable releases | `0.7.0` |
| `strata-preview-bin` | The newest preview-channel release | `0.7.1rc2` |

They declare `provides=('strata')` and conflict with each other, reserving the `strata` name for a future source-built package. Both install the prebuilt release archive rather than compiling: `options=('!strip')` keeps the binary byte-identical to the published artifact, so `gh attestation verify` still matches the file pacman installed.

## Layout

```
packaging/aur/
  PKGBUILD.in                 the single template both packages render from
  strata-bin/                 PKGBUILD and .SRCINFO, committed and reviewable
  strata-preview-bin/
scripts/
  update_aur.py               renders both packages for a published release
  test_update_aur.py
```

The AUR requires a self-contained `PKGBUILD`, so the rendered files are committed here and copied to the AUR git remotes. This repository is the source of truth; never edit an AUR checkout directly.

## Why a package suppresses the in-app updater

Pacman owns `/usr/bin/strata`. If the in-app updater replaced it, `pacman -Qkk strata-bin` would report the package as modified and the next system update would silently overwrite the downloaded binary.

`package()` therefore installs `/usr/share/strata/install-source.toml`:

```toml
manager = "pacman"
package = "strata-bin"
channel = "stable"
update_command = "sudo pacman -Syu strata-bin"
alternate_package = "strata-preview-bin"
```

Strata reads it relative to its own install prefix (`<prefix>/share/strata/install-source.toml`, falling back to `/usr/share`). When it is present, Strata still checks for and reports new releases, but **Settings → Updates** shows the owning package and its update command instead of an install action, and `services::install_update` refuses outright. Every field is optional; unknown keys are ignored, so a marker written by a newer package never breaks an older binary. A marker that exists but cannot be parsed still counts as a packaged install, which fails safe.

Any future `.deb`, `.rpm`, or Flatpak package opts into the same behavior by installing this file.

## Updating the packages for a release

1. Publish the release (see the release workflow) and confirm both architectures uploaded their `.tar.gz` and `.sha256`.
2. Render the packages. Checksums are downloaded from the release, so a wrong version fails instead of pinning a bad digest:

   ```bash
   python3 scripts/update_aur.py --stable 0.7.0                      # stable only
   python3 scripts/update_aur.py --stable 0.7.0 --preview 0.7.1-rc.2 # both
   ```

   Pass `--pkgrel` when repackaging the same release, and `--skip-srcinfo` on a machine without `makepkg` (regenerate `.SRCINFO` before publishing).
3. Validate in a clean chroot:

   ```bash
   cd packaging/aur/strata-bin
   makepkg --cleanbuild --syncdeps --force
   namcap PKGBUILD
   namcap strata-bin-*.pkg.tar.zst
   sudo pacman -U strata-bin-*.pkg.tar.zst
   strata --version
   ```
4. Commit the rendered `PKGBUILD` and `.SRCINFO` through a pull request.
5. Publish to the AUR:

   ```bash
   git clone ssh://aur@aur.archlinux.org/strata-bin.git aur-strata-bin
   cp packaging/aur/strata-bin/{PKGBUILD,.SRCINFO} aur-strata-bin/
   cd aur-strata-bin && git commit -am "upgpkg: strata-bin 0.7.0-1" && git push
   ```

AUR pushes are deliberately manual. Automating them would put an AUR SSH key with push rights into repository secrets, next to the release workflow that already produces the binaries those packages ship.

### `pkgver` mangling

`pkgver` may not contain a hyphen, so `update_aur.py` strips semver prerelease punctuation: `0.8.0-rc.1` becomes `0.8.0rc1`. This preserves `vercmp` ordering, so pacman upgrades in release order:

```
0.7.0 < 0.8.0nightly20260901 < 0.8.0rc1 < 0.8.0rc2 < 0.8.0rc10 < 0.8.0
```

The download URL always uses the real release tag, never the mangled `pkgver`.

### Expected `namcap` output

`namcap` reports `bubblewrap` as an unnecessary dependency. It is required: Strata executes `bwrap` to sandbox every preview and thumbnail render, and `namcap` only inspects ELF linkage, not processes a program spawns.

## Desktop metadata

The launcher and application icon are installed from the release archive, not from this repository, so the package always matches the binary it ships. Releases published before desktop metadata was added to the archive produce a package with a working `strata` command and no launcher; `package()` prints a warning rather than failing, so the package stays installable.

## Other distributions

Debian/Ubuntu, Fedora, Flatpak, and AppImage each have their own dependency, sandboxing, update, and review requirements and are tracked as separate issues. The install layout and the `install-source.toml` contract above are intended to be reused by all of them.
