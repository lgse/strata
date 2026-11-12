# Distribution packaging

Strata's proposed AUR packages are generated from the same template and both install `/usr/bin/strata`. They are not published yet; AUR publication remains disabled until a maintainer account and both package bases are available:

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

Strata reads it once at start-up, relative to its own install prefix (`<prefix>/share/strata/install-source.toml`). When it is present, Strata checks the AUR first and reports a release only after that package version is available. **Settings → Updates** opens the installed AUR helper in a terminal with a full-system update command, or opens the package’s AUR page when no supported helper is available. Strata looks for the helpers on `PATH` once per run, off the interface thread, when it first needs the update method; a helper installed while Strata runs is offered after a restart. `services::install_update` still refuses to replace the package-owned binary outright. Every field is optional and unknown keys are ignored, so the parser can evolve without breaking an older binary. A marker that exists but cannot be parsed still counts as packaged, which fails safe.

This is currently an official Strata metadata format for these two AUR packages, not a public cross-distribution packaging API. Supporting Debian, RPM, Flatpak, or other providers requires an explicit design for ownership, availability checks, channels, and update actions rather than assuming AUR semantics. The [COPR package](#fedora-copr) avoids the question by compiling the updater out and installing no marker.

The [Nix package](#nix) is the one other marker. It installs `nix/install-source.toml` (`manager = "Nix"`, `package = "strata"`, nothing else) so Diagnostics reports `Installed by Nix as strata.` It is safe only because that package compiles with `STRATA_UPDATER=disabled`: `update_method()` treats every marker as an AUR install, so a Nix build with the updater enabled would query the AUR for a package named `strata`. Give the marker a provider field before enabling the updater for Nix.

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

When AUR publication is enabled, the release workflow performs steps 2 and 4 automatically for every non-nightly release. Stable releases regenerate the stable package and update the Preview package only when that would not downgrade it; staged prereleases update the Preview package under the same ordering rule. Its protected `package-automation` environment must provide a fine-grained `PACKAGE_PR_TOKEN` with Contents and Pull requests read/write access. Using a token rather than the workflow's built-in token ensures the generated pull request runs the normal CI workflows. Package regeneration runs as an unprivileged user because `makepkg` refuses to run as root.

Publishing uses the protected `aur-production` environment and its `AUR_SSH_PRIVATE_KEY` secret. The key's AUR account must maintain both package bases. The release preparation and publish jobs remain disabled unless the repository variable `AUR_PUBLISH_ENABLED` is exactly `true`; keep it disabled until both package bases and their maintainer account exist. A manual workflow dispatch can safely retry an interrupted or already-current publication after enabling it.

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

The archive also ships `io.github.lgse.Strata.FileManager1.service`, which users can install under `$XDG_DATA_HOME/dbus-1/services` to make Strata the D-Bus-activatable owner of `org.freedesktop.FileManager1`. Packages must not install it into a system service directory: multiple files providing that name in one directory are selected arbitrarily, and merely installing Strata must not change the user's preferred file manager. They may install the inactive template under `/usr/share/strata` so users can opt in without downloading the archive. Keep activation an explicit per-user choice as documented in the README. Packages must not run `--install-udiskie-unlock` in `package()` or a pacman hook.

### D-Bus activation and desktop portals

The FileManager1 service starts Strata with `--gapplication-service`, and the
FileChooser portal backend service starts it with `--portal`. For both launch
modes, Strata re-executes before GTK initialization with `no-portals` added to
`GDK_DEBUG`, preserving other debug flags. Keep those service arguments when
adapting the activation commands for a package.

This avoids a circular wait with desktop portals that synchronously dispatch
`OpenDirectory` to `FileManager1.ShowItems` or `OpenFile` to the FileChooser
backend: GTK can query the same portal's host registry during startup, before
Strata can answer. With GTK 4.24.1 and xdg-desktop-portal 1.22.1, that cycle
delays cold activation for approximately 25 seconds and stalls other portal
consumers.

This is a compatibility mitigation for the activated process, not a replacement
for asynchronous dispatch in the desktop portal. GTK portal integration is
disabled for that process's lifetime, and later launches of Strata reuse that
process until it exits. Programs it launches, file providers, and a restart from
Settings get the original `GDK_DEBUG` back; ordinary application launches
never acquire the override. Do not set `GDK_DEBUG=no-portals` globally to
address this problem.

## Nix

`flake.nix` exposes `packages.<system>.default` for `x86_64-linux` and `aarch64-linux`, built from `package.nix` with `nix build`, `nix run`, or a flake input in a NixOS or Home Manager configuration; the README shows the user-facing steps. Unlike the AUR packages it compiles from source, with `STRATA_UPDATER=disabled`, without the unfree `rar` feature unless `rarSupport = true`, and with the [sandbox runtime paths](preview-sandbox.md#packaging-non-fhs-runtimes) and `STRATA_TRUSTED_COMMAND_PATH` embedded so bubblewrap and the preview helpers come from the package closure rather than a system profile. `nix flake check` builds the package; the Rust test suite is not run inside the Nix build because it needs the private display and D-Bus session described in [testing.md](testing.md).

`wrapGAppsHook4` turns `bin/strata` into a wrapper around `bin/.strata-wrapped`, and every store path changes on update. The runtime-installed integrations (file chooser portal, FileManager1 service, Omarchy setup, udiskie unlock hook) record `/proc/self/exe`, which on Nix is the wrapped binary of one build: a D-Bus-activated instance would start without the wrapper's GStreamer, gdk-pixbuf, and XDG environment, and the next update or garbage collection would leave the files pointing at a stale or missing store path. The package therefore compiles with [`STRATA_INTEGRATIONS=package`](#building-with-package-provided-integrations) and ships the files itself: `passthru.fileChooserPortal` holds `share/xdg-desktop-portal/portals/strata.portal` and the `org.freedesktop.impl.portal.desktop.strata` D-Bus service, and `passthru.fileManagerService` holds the `org.freedesktop.FileManager1` service, each with `Exec` pointing at the package's wrapper. They are separate derivations rather than part of `$out` because NixOS registers every system package's `share/dbus-1/services` with the session bus, so shipping the FileManager1 service in the main package would make every install claim that bus name ahead of or behind the desktop's own file manager, depending on directory order. The README shows the NixOS and Home Manager options that enable them; `nix flake check` builds both derivations. The inactive template under `share/strata` remains for the manual copy the README describes for other installs.

The default folder handler stays a user-level `xdg-mime` association because it stores only the desktop ID, which does not change between builds; Home Manager users set it through `xdg.mimeApps.defaultApplications`. The Omarchy shortcut block has no Nix equivalent, and the udiskie hook is a `services.udiskie.settings` entry.

## Fedora COPR

`packaging/rpm/strata.spec.in` builds Strata from source for [COPR](https://copr.fedorainfracloud.org), which serves it as a dnf repository for Fedora on `x86_64` and `aarch64`. `scripts/build_srpm.py` renders the spec for a release tag and writes a source RPM holding three files: a `git archive` of the tag, a tarball of vendored crates, and the rendered spec.

```
packaging/rpm/
  strata.spec.in              the template; never commit a rendered spec
scripts/
  build_srpm.py               renders the spec and builds the source RPM
  test_build_srpm.py
.github/workflows/
  publish-copr.yml            submits a release to COPR
  rpm.yml                     builds, lints, and installs the RPM for every change it depends on
```

Unlike the AUR packages, nothing rendered is committed: the source RPM is built from the tagged commit itself, so the only reviewed inputs are the template and the script.

### What the build changes

- **No RAR support.** UnRAR's license is not on Fedora's list of allowed licenses, and COPR accepts only material under those licenses, including unused sources in a source RPM. The only UnRAR source code reaches the build through the `unrar_sys` crate. The spec builds with `--no-default-features`, and `cargo vendor-filterer` vendors only the crates that build needs for Linux on the two supported architectures, replacing every other crate in `Cargo.lock` with an empty stub so Cargo can still resolve it offline. `build_srpm.py` refuses to write a source RPM unless `unrar` and `unrar_sys` are such stubs, and it lists every stub so the package does not claim to bundle it. The repository archive still carries `data/licenses/UnRAR.txt` and the MIT-licensed RAR test fixtures; neither is UnRAR code, and the package installs neither.
- **No in-app updater.** The spec builds with [`STRATA_UPDATER=disabled`](#building-without-the-in-app-updater), so dnf owns updates and Diagnostics reports in-app updates as disabled. It installs no `install-source.toml`: the marker means an AUR install to the updater code.
- **Runtime integrations stay enabled.** `/usr/bin/strata` does not move between updates, so the runtime installers work as they do for a release archive. The FileManager1 service is installed only as the inactive template under `/usr/share/strata`, as the AUR packages do.
- **Offline, vendored build.** COPR builders have no network access by default. `%cargo_prep -v vendor` points Cargo at the vendored crates and sets it offline.

`STRATA_BUILD_COMMIT`, `STRATA_RELEASE_TAG`, and `STRATA_BUILD_KIND` are set from the tag, so `strata --version` and Diagnostics identify the release exactly as a release-archive build does.

### Versions and channels

RPM sorts a `~` suffix before the version it is attached to, so prereleases map onto versions that order before their final release:

```
1.2.1 < 1.3.0~alpha.1 < 1.3.0~beta.1 < 1.3.0~rc.1 < 1.3.0~rc.2 < 1.3.0
```

Nightlies are not packaged, for the reason given for the AUR packages, and `build_srpm.py` rejects them.

The two channels are separate COPR projects, both with a package named `strata`:

| Project variable | Receives |
| --- | --- |
| `COPR_PROJECT` | Stable releases |
| `COPR_PREVIEW_PROJECT` | Alpha, beta, RC, and stable releases |

A stable release also goes to the Preview project so that channel never stays on a prerelease older than the latest stable release. Users enable one of the two.

### Licensing metadata

The `License` tag is generated: `cargo2rpm license-summary` lists the licenses of the crates linked into the binary, and `build_srpm.py` adds the licenses of the assets compiled into it (`ASSET_LICENSES`; see [third-party notices](../THIRD_PARTY_LICENSES.md)). Update `ASSET_LICENSES` whenever a bundled asset with a new license is added. The package installs the per-crate breakdown as `LICENSE.dependencies` and `cargo-vendor.txt`, from which RPM generates a `bundled(crate(…))` provide for every vendored crate.

### Building locally

On Fedora, or in a Fedora container:

```bash
sudo dnf install git rpm-build rpmlint cargo-rpm-macros cargo-vendor-filterer 'dnf-command(builddep)'
python3 scripts/build_srpm.py --tag v1.2.1                    # a published tag
python3 scripts/build_srpm.py --tag v1.2.1 --ref HEAD         # the working commit, named as v1.2.1
sudo dnf builddep target/rpm/strata-*.src.rpm
rpmbuild --rebuild target/rpm/strata-*.src.rpm
rpmlint target/rpm/strata-*.src.rpm ~/rpmbuild/RPMS/*/strata-*.rpm
```

`--ref` packages any commit under a release tag's identity, which `.github/workflows/rpm.yml` uses to build pull requests. That workflow runs on every change to the package's inputs, including the workspace's `Cargo.toml` files and `Cargo.lock`, because a dependency update can add a crate under a license COPR does not accept. It builds on `x86_64` with the newest Fedora only; the COPR builds at release time cover the other releases and `aarch64`, and a failed one can be retried with `publish-copr.yml`. `--release` raises the RPM release number to rebuild a version that is already published.

### Publishing

`.github/workflows/publish-copr.yml` builds the source RPM for a published release, uploads it as a workflow artifact, submits it to the projects that release belongs to, and waits for every COPR build. The Release workflow calls it after publishing every non-nightly release. Run it manually to retry a failed submission, rebuild with a higher `release` number, or backfill an older release. It always uses the packaging tools from the workflow's own revision and only the source archive from the release tag, so tags older than the spec can be packaged too, back to `v1.2.0-rc.1`. `build_srpm.py` refuses earlier tags: their `build.rs` ignores `STRATA_UPDATER`, so the package would keep the in-app updater.

It stays disabled unless the repository variable `COPR_PUBLISH_ENABLED` is exactly `true`. To set it up:

1. Create the two COPR projects with the `fedora-*` chroots for `x86_64` and `aarch64` that should be supported. Leave **Enable internet access during builds** off; the build must not need it.
2. Set the repository variables `COPR_PROJECT` and `COPR_PREVIEW_PROJECT` to `<owner>/<project>`. Leave `COPR_PREVIEW_PROJECT` empty to publish stable releases only.
3. Create the protected `copr-production` environment, restricted to the default branch, with a `COPR_CONFIG` secret holding the contents of the `~/.config/copr` file shown at <https://copr.fedorainfracloud.org/api/>.
4. Set `COPR_PUBLISH_ENABLED` to `true`.

COPR API tokens expire after 180 days. Renew the token on the same page and replace `COPR_CONFIG` before it expires; an expired token fails the publish job, which can then be rerun manually.

## Other distributions

Debian/Ubuntu, Flatpak, and AppImage each have their own dependency, sandboxing, update, and review requirements and are tracked as separate issues. The AUR marker must not be reused for them until those provider-specific semantics are designed; the Nix marker above and the COPR package rely on the updater being compiled out, not on such a design.

### Building without the in-app updater

A distribution whose own packages deliver Strata updates builds with the updater compiled out:

```bash
STRATA_UPDATER=disabled cargo build --locked --release
```

Such a build never checks for, downloads, or installs a release. Settings has no Updates page and its search finds no update settings, the update notice never appears, and `services::install_update` refuses to run. Diagnostics reports the installation as having in-app updates disabled, which tells a downstream build apart in bug reports. `STRATA_UPDATER` accepts only `enabled` (the default) and `disabled`; any other value fails the build, so a typo cannot ship an updater. It combines with `--no-default-features` for builds without RAR support.

### Building with package-provided integrations

A package that cannot let Strata write its own integration files at runtime, because the executable path changes on every update (Nix, Guix, and other store-based or self-contained layouts), builds with the runtime installers compiled out:

```bash
STRATA_INTEGRATIONS=package cargo build --locked --release
```

Such a build never writes, removes, or refreshes the file chooser portal, the FileManager1 service, the Omarchy shortcut block, or the udiskie unlock hook. Settings → General → Desktop integration keeps the **System file manager** and **Unlock encrypted volumes** rows, so search still finds them, but each explains that the package provides the integration instead of showing status indicators and setup actions. The command-line modes `--install-portal`, `--uninstall-portal`, `--dismiss-portal-prompt`, `--install-udiskie-unlock`, and `--uninstall-udiskie-unlock` exit with the same explanation, and the start-up check that restarts a stale per-user portal backend is skipped. The `--portal`, `--gapplication-service`, and `--udiskie-hook` modes that the shipped files invoke are unaffected. `STRATA_INTEGRATIONS` accepts only `runtime` (the default) and `package`; any other value fails the build. The package must then install the files `crates/strata-desktop/src/portal_setup.rs` would have written, with `Exec` lines that point at the executable users launch (the wrapper on Nix), and document how to enable them; the [Nix section](#nix) is the reference implementation.
