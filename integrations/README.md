# Bundled integrations

Integrations are [external file providers](../docs/file-providers.md) that ship
with Strata but stay off until a user installs them from **Settings →
Integrations**. Each subdirectory here is one catalog entry. Strata embeds the
directory at build time, so adding an integration needs no packaging change; the
Settings page copies it into `~/.config/strata/providers/<id>/` on install and
removes it on request.

Providers run with the user's permissions, outside any sandbox. Review every
contribution as you would a program the user is asked to run.

## Layout

```text
integrations/
  kdeconnect/
    catalog.toml       required: the catalog entry
    provider.py        the program Strata starts; named by `entry`
    messages.json      the provider's own translations, if any
    logo.png           optional artwork declared under [icons]
    README.md          documentation; not installed
    tests/             the provider's tests; not installed
```

Every regular file in the directory except `README.md` is installed, so keep
fixtures and tests under `tests/`. Directories starting with `.` or `_` are
ignored. Strata writes `provider.json` and `catalog-install.json` itself; do not
ship them.

## catalog.toml

```toml
schema_version = 1
id = "kdeconnect"                 # the directory name, a slug
name = "KDE Connect"              # at most 64 bytes
description = "Send files to paired phones and computers from the Send to… menu."
category = "devices"              # cloud | devices | version-control | other
maintainer = "github-username"    # answers issues; not shown in Strata
runtime = "python3"               # interpreter basename, resolved on install
entry = "provider.py"             # started as [runtime, entry]
icon = "logo"                     # optional; drawn beside the menu heading

[icons]
logo = "logo.png"                 # PNG, at most 256×256 and 64 KiB

[[requirements]]
command = "kdeconnectd"           # looked up in trusted system directories
packages = { pacman = "kdeconnect", apt = "kdeconnect", dnf = "kdeconnect", zypper = "kdeconnect" }

[[requirements]]
module = "gi"                     # top-level; asked of the interpreter, never imported
packages = { pacman = "python-gobject", apt = "python3-gi", dnf = "python3-gobject", zypper = "python3-gobject" }

[[requirements]]
command = "localsend"
alternatives = ["localsend_app"]  # up to four other names for the same program
```

A command is found when one of its names sits in a trusted system directory
(`/usr/bin`, `/bin` and the NixOS profile directories). Its real path must lie
under one of those directories or a Nix or Guix store, or be owned by root, with
every directory above it, none writable by group or others, as with programs
that packages install under `/opt` or `/usr/lib` and link into `/usr/bin`. A
provider that runs the command should resolve it the same way; the LocalSend
provider shows how.

`name` and `description` are Strata interface text: add them to every catalog
under `data/locales/` in all thirteen languages (see
[Internationalization](../docs/internationalization.md)). Messages the provider
itself returns stay with the provider; follow the system locale as the protocol
documentation describes.

The Settings page shows each requirement with the install command for the
detected distribution, which the user can copy or run in a terminal, so list
the packages for all four managers when they exist. The runtime is checked automatically; declare it only to attach
packages.

Strata records a content hash of the installed files. When the bundled files
change in a release, installed copies show **Update available**; the user
chooses when to update, and local edits are detected before anything is
replaced. There is no version field to bump.

## Policy

- **Maintainer.** Every entry names a GitHub user who answers issues about it.
  When an integration breaks and nobody fixes it within a release cycle, it is
  marked unmaintained in its description and removed in the following release.
- **Working on Linux.** An entry needs a working local client, daemon or API
  on Linux before it is listed. Candidates without one belong in an issue.
- **Protocol first.** Use the documented provider protocol only. Do not inspect
  file contents to draw menus or badges, and keep selected paths out of logs.
- **Tests.** Keep unit tests under `tests/`, runnable with the interpreter the
  entry names from the integration directory (`python3 -m unittest discover -s
  tests`), or for every entry with `python3 scripts/integration_tests.py`. List
  Ubuntu packages the tests need in `tests/apt-packages.txt`. They run in the
  `Integrations` workflow when files under `integrations/` change. The
  Rust suite checks that every bundled entry parses and stays within the
  manifest bounds; `./scripts/test-headless.py integrations` covers the
  Settings page.
- **No network at review time.** Reviewers run the provider against a mock or
  local service, as the KDE Connect tests do with a private D-Bus daemon.

## Developing an entry

Point Strata at a checkout instead of the bundled catalog while iterating:

```bash
STRATA_INTEGRATION_CATALOG=integrations cargo run
```

Settings → Integrations then lists the directory's entries. Install from there
and exercise the provider through the context menu; **Refresh** reloads the
running providers after you edit the installed copy.
