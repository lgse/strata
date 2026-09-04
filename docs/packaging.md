# Distribution packaging

Strata is packaged for Arch Linux through the AUR. Two packages are published from the same template, and both install `/usr/bin/strata`:

| Package | Tracks | Example `pkgver` |
| --- | --- | --- |
| `strata-bin` | Latest stable release | `0.9.0` |
| `strata-rc-bin` | Newest non-nightly Preview release | `0.9.0` or `0.10.0rc.1` |

They declare `provides=("strata=${pkgver}")` and conflict with each other, reserving the `strata` name for a future source-built package.

Strata's third channel, nightly, is deliberately not packaged. Every nightly has its own release tag in the download URL, and `makepkg` fetches `source` before `pkgver()` runs, so no package can discover the newest nightly and download it in one build. Pinning each nightly instead would mean an AUR push per nightly. Nightly users install manually and update in-app, which already supports the channel. Both install the prebuilt release archive rather than compiling: `options=('!strip')` keeps the binary byte-identical to the published artifact, so `gh attestation verify` still matches the file pacman installed.

## Layout

```
packaging/aur/
  PKGBUILD.in                 the single template both packages render from
  strata-bin/                 PKGBUILD and .SRCINFO, committed and reviewable
  strata-rc-bin/
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
aur_helpers = ["yay", "paru", "pikaur", "trizen"]
alternate_package = "strata-rc-bin"
```

There is no `update_command`. pacman cannot update an AUR package -- no configured repository carries `strata-bin`, so `pacman -Syu strata-bin` fails with `target not found` -- and which helper to name depends on what the user has installed. The marker lists candidates and Strata names the first one on `PATH`, falling back to generic AUR-helper wording when none is.

`channel` is the packaging channel, named after the package (`stable`, `rc`). Strata maps it onto its own persisted channel -- `rc` becomes `preview`, the in-app channel that accepts alpha, beta, and RC builds -- and then locks the **Settings -> Updates** channel selector to it. The channel is a property of the installed package, not a preference: switching channels means installing the other package.

Strata reads it relative to its own install prefix (`<prefix>/share/strata/install-source.toml`). When it is present, Strata checks the AUR first and reports a release only after that package version is available. **Settings → Updates** shows the owning package and its update command instead of an install action, and `services::install_update` refuses outright. Every field is optional and unknown keys are ignored, so the parser can evolve without breaking an older binary. A marker that exists but cannot be parsed still counts as packaged, which fails safe.

This is currently an official Strata metadata format for these two AUR packages, not a public cross-distribution packaging API. Supporting Debian, RPM, Flatpak, or other providers requires an explicit design for ownership, availability checks, channels, and update actions rather than assuming AUR semantics.

## Updating the packages for a release

1. Publish the release (see the release workflow) and confirm both architectures uploaded their `.tar.gz` and `.sha256`.
2. Render the packages. Checksums are downloaded from the release, so a wrong version fails instead of pinning a bad digest:

   ```bash
   python3 scripts/update_aur.py --stable 0.9.0                    # stable only
   python3 scripts/update_aur.py --preview 0.10.0-rc.1             # preview only
   python3 scripts/update_aur.py --stable 0.9.0 --preview 0.9.0    # both
   ```

   Each package is rendered only when its own flag is passed. Stable releases update `strata-bin` and also update `strata-rc-bin` when they are not older than its current version. Alpha, beta, and RC releases update only `strata-rc-bin`, again without allowing a downgrade. Nightlies update neither package.

   Pass `--pkgrel` when repackaging the same release, and `--skip-srcinfo` on a machine without `makepkg` (regenerate `.SRCINFO` before publishing). `--check` verifies that committed files still match the template.
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
5. Merge the reviewed package update. `.github/workflows/publish-aur.yml` then pushes both package repositories to the AUR.

The release workflow performs steps 2 and 4 automatically for every non-nightly release. Stable releases regenerate the stable package and update the Preview package only when that would not downgrade it; staged prereleases update the Preview package under the same ordering rule. The repository must allow GitHub Actions to create pull requests.

Publishing uses the protected `aur-production` environment and its `AUR_SSH_PRIVATE_KEY` secret. The key's AUR account must maintain both package bases. A manual workflow dispatch can safely retry an interrupted or already-current publication.

### `pkgver` mangling

`makepkg`'s `check_pkgver` rejects colons, slashes, hyphens, and whitespace, but allows dots. `update_aur.py` therefore removes only the hyphen introducing the prerelease: `0.8.0-rc.1` becomes `0.8.0rc.1`.

Dots are deliberately kept. `vercmp` splits a version on non-alphanumerics and compares segment by segment, so dropping them concatenates a prerelease's components into a single number. That is wrong for any tag with two numeric components: `0.8.0-nightly.20260901.2` would mangle to `0.8.0nightly202609012`, which `vercmp` orders *above* `0.8.0nightly20260902` -- the second nightly of one day claiming to be newer than the next day's. Keeping the separator makes each component its own segment and orders correctly:

```
0.7.0 < 0.8.0nightly.20260901 < 0.8.0nightly.20260901.2 < 0.8.0nightly.20260902 < 0.8.0rc.1 < 0.8.0rc.2 < 0.8.0rc.10 < 0.8.0
```

Verify any change to this with `vercmp` directly. The download URL always uses the real release tag, never the mangled `pkgver`.

### Expected `namcap` output

`namcap` reports `bubblewrap` as an unnecessary dependency. It is required: Strata executes `bwrap` to sandbox every preview and thumbnail render, and `namcap` only inspects ELF linkage, not processes a program spawns. The packaging CI job fails on `E:` lines only, so this warning does not break the build.

## Desktop metadata

The launcher and application icon are installed from the release archive, not from this repository, so the package always matches the binary it ships. Releases published before desktop metadata was added to the archive produce a package with a working `strata` command and no launcher; `package()` prints a warning rather than failing, so the package stays installable. Drop the conditional once the oldest release either package pins carries the metadata.

## Other distributions

Debian/Ubuntu, Fedora, Flatpak, and AppImage each have their own dependency, sandboxing, update, and review requirements and are tracked as separate issues. The AUR marker must not be reused for them until those provider-specific semantics are designed.
