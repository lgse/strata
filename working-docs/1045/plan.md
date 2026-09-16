# Plan: resolve helper binaries without PATH

Issue: [#1045](https://github.com/lgse/strata/issues/1045)
Labels: `security`, `P1` (unchanged; do not retitle, relabel, or assign a P-band)
Assignee: `wmfeht` (confirmed via GitHub MCP `issue_write` update)
No closing PR. Open [#506](https://github.com/lgse/strata/pull/506) is signed-manifest update trust (`Closes #148`), not helper lookup. [#1052](https://github.com/lgse/strata/pull/1052) is #854. One comment ([pdwaldrop](https://github.com/lgse/strata/issues/1045#issuecomment-5681617499) asking to take it) — leave assignee `wmfeht`.

This is defense-in-depth. Describe current lookup and trusted-path behavior only. Do not write exploits, attack proofs, or reproduction payloads.

## Goal

Host-side helper programs that Strata execs by bare name must be started from a trusted absolute path. Lookup may search only a fixed list of admin-managed system directories (FHS, NixOS/Guix system profiles, NixOS wrappers). It must never read inherited `PATH` or `$HOME`. Canonicalize of a search hit must land under a separate trust-root list (FHS, `/nix/store`, `/gnu/store`, wrappers). Exec the path found under the search root, not the store target. Missing security-relevant helpers fail closed (error / skip the spawn). There is still no unsandboxed preview fallback.

Replace the `sha256sum` subprocess with an in-process SHA-256 of the downloaded archive so update verification does not exec an external hasher.

## Research

### Current lookup

`std::process::Command::new(name)` with a relative `name` is resolved by the OS using `PATH`. `install.sh` prepends `$HOME/.local/bin` when installing the user-local binary (`local_bin_on_path`), so helper lookup and the install prefix share that environment.

Security-relevant bare names (host process, not inside the jail):

| Site | Name | Role |
| --- | --- | --- |
| `src/sandbox.rs` `sandbox_command` | `bwrap` | Preview/thumbnail sandbox. `parse` and `sandbox/media.rs` both build this command, then `spawn_renderer`. |
| `src/services/update_install.rs` `verify_checksum` | `sha256sum` | Compares `first_hash_token` of subprocess stdout to the published `.sha256` file. |
| `src/services/update_install.rs` `try_install` | `tar` | Extracts the release `.tar.gz`. |
| `src/ui/settings.rs` `restart` | `sh` | Detached waiter: wait for the old PID, then `exec` the replacement binary. |

Lower-impact host helpers, same lookup:

- `gtk-update-icon-cache`, `update-desktop-database` (`refresh_desktop_metadata`)
- `xdg-mime`, `gdbus`, `systemctl` (`portal_setup.rs`)
- `omarchy`, `hyprctl` (`portal_setup/omarchy.rs`)

Already pinned, keep as the precedent:

- `PACMAN` / `PACMAN_CONF` = `/usr/bin/pacman`, `/usr/bin/pacman-conf` (`update_install.rs`)
- In-sandbox argv after `bwrap --`: `/usr/bin/prlimit`, `/usr/bin/ffmpegthumbnailer` (`sandbox.rs`)
- Jail env: `--clearenv --setenv PATH /usr/bin`

**Not this bug** (issue text): `sandbox_helper.rs` / `sandbox_helper/media.rs` (`ffprobe`, `dcraw`, `simple_dcraw`, `ffmpegthumbnailer`, inner `prlimit`). Those run *inside* bubblewrap after PATH is forced to `/usr/bin`. Do not change them here.

`Command::new` on an already-absolute path (`current_exe`, user `.desktop` Exec, test fixtures) is already trusted-path or caller-supplied. Out of scope.

### Why a resolver, not one constant per helper

`PACMAN` can be a single `/usr/bin/...` constant. `bwrap` / `tar` / `sh` sit on merged-usr (`/bin` → `/usr/bin`) and split-usr layouts. The issue asks for a small allowlisted-directory resolver rather than a hardcoded path that is wrong on one family.

`sha2` `0.11.0` is already in `Cargo.lock` (transitive). Promote it to a direct dependency for in-process hashing. License is allowed by `deny.toml` (Apache-2.0 / MIT). Do not add a new hasher stack (`ring`, OpenSSL).

`tar` + `flate2` already extract the Omarchy repo db in-process (`repository_database_version`). Do **not** switch release-archive extraction to that crate in this change: archive-entry policy is [#334](https://github.com/lgse/strata/issues/334) / #506. This issue is which `tar` binary is exec'd.

[#506](https://github.com/lgse/strata/pull/506) moves verification in-process (signed manifest). An in-process SHA-256 here is complementary and must not wait on that PR. Do not merge or rebase onto `feat/148-harden-update-safety`.

### Callers after the change

- `sandbox.rs` `parse` and `sandbox/media.rs`: resolve `bwrap`, pass the `Path` into `sandbox_command`, spawn. If resolve fails, return the existing class of error (`Unable to start the preview sandbox: …`). Never build `Command::new("bwrap")`.
- `verify_checksum`: hash the archive file in-process; keep `first_hash_token` only for the published checksum text.
- `try_install`: `Command::new(trusted_command::resolve("tar")?)` with the same `-xzf` argv.
- `restart`: resolve `sh`; if missing, keep today's silent skip (`spawn().is_err() { return }`). Do not fall back to a relative `"sh"`.
- Desktop/portal/omarchy helpers: resolve or skip/warn on the same paths they already use when the spawn fails. `detected_major` already falls through from `omarchy version` to `/usr/share/omarchy/version` and `~/.local/share/omarchy/version` files — a failed resolve should take that file path, not PATH search.

## Round 2 (Nix/Guix)

William asked for Nix/Guix compatibility. Keep **two lists**. Do not collapse search and trust. Do not exec the store target.

Search roots (basename may appear; never `$HOME`, never inherited `PATH`):

- `/run/wrappers/bin` **first** (NixOS `security.wrappers`)
- `/usr/bin`, `/usr/sbin`, `/bin`, `/sbin`
- `/run/current-system/sw/bin` (NixOS system profile)
- `/run/current-system/profile/bin` (Guix)

Trust roots (canonicalize may land):

- the FHS dirs above, after canonicalize
- `/run/wrappers/bin` (wrapper binaries are regular files, not store symlinks)
- `/nix/store`
- `/gnu/store`

Algorithm:

1. Reject anything that is not a single basename (same as round 1).
2. Look only under search roots. Never read inherited `PATH`.
3. canonicalize the candidate.
4. Accept only if the final path sits under a trust root.
5. **Exec the path you found** (e.g. `/run/current-system/sw/bin/bwrap`), **not** the store target. That keeps argv0 intact for busybox/coreutils and matches how Nix profiles are meant to be invoked.

User nix/guix profiles (`~/.nix-profile`, `~/.guix-profile`) stay out.

## Approach

Smallest coherent fix: one lookup helper, then thread it through the listed host execs. In-process digest for checksums only.

1. Add `src/trusted_command.rs` (crate-root module next to `sandbox` / `services`, not under `util/` which is GTK date formatting). Adjacent tests: `src/trusted_command/tests.rs`.

   - Two lists, as in Round 2. Same `/usr/bin` preference as `PACMAN` among FHS dirs. Do not include `/usr/local/bin`, `$HOME`, user nix/guix profiles, or anything from `PATH`.
   - `resolve(name: &str) -> Result<PathBuf, String>`:
     - Accept a single basename only (`bwrap`, `tar`, `sh`). Reject empty, `.`, `..`, or any name containing a path separator.
     - For each search root, consider `dir.join(name)` if it is a regular file (symlinks to files count).
     - After following links, require the final path still sit under a trust root (canonicalize the candidate; canonicalize trust roots when they exist).
     - Return the search-root path, not the canonical store/FHS target.
     - First match wins. If none, error. Never call `Command::new` with the bare name. Never read `std::env::var_os("PATH")`.
   - Test seam: `resolve_in(name, search_roots, trust_roots)` so unit tests use fixture directories, not the live filesystem.
   - Optional `command(name) -> Result<Command, String>` that is `Command::new(resolve(name)?)`.

2. Change `sandbox_command` to take the bubblewrap executable as `&Path` and `Command::new(bwrap)`. Runtime callers resolve `"bwrap"` once. Argument-inspection tests pass `Path::new("/usr/bin/bwrap")` without requiring the file to exist (today `Command::new("bwrap")` also does not require it). Assert `get_program()` is that absolute path.

3. `verify_checksum`: stream the archive through `sha2::Sha256`, lowercase hex, compare to `first_hash_token` of the published checksum body. Drop `Command::new("sha256sum")`. Keep the `.sha256` URL fetch and the mismatch error string.

4. Remaining listed host helpers: `trusted_command::command("tar")` (and `sh`, `xdg-mime`, `gdbus`, `systemctl`, `gtk-update-icon-cache`, `update-desktop-database`, `omarchy`, `hyprctl`). Preserve existing success/ignore/warn policy; only the program path changes.

5. Docs: `docs/preview-sandbox.md` — host starts bubblewrap from a trusted absolute search hit, not `PATH`; canonical target must sit under FHS/`/nix/store`/`/gnu/store`. No preference, icon, or theme work.

No `settings.toml`. No new Lucide assets. No sandbox/media-runtime patches.

## Constraints

- GTK 4.12+ / 4.14 baseline. Rust tests that need a display use private Xvfb:

  `xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1`

  Never `DISPLAY=:1`. This change is process-spawn / hashing; most new tests need no GTK.
- Tests adjacent to production (`src/trusted_command/tests.rs`, extend `src/sandbox/tests.rs`, `src/services/update_install/tests.rs`, `src/portal_setup/tests.rs` if a helper constructor is extracted). Do not inline tests in production files. Do not add a `tests/` integration crate or a new E2E scenario.
- `cargo-deny` / `typos` are not on this Cloud image; do not treat a local skip as a CI pass. Adding `sha2` must stay on crates.io and allowed licenses.
- Fail closed for `bwrap` and `tar` / checksum. Relaunch and desktop-cache helpers may skip when the binary is absent, matching current spawn-error handling, but must not PATH-search.

## Risks

- `sandbox_command` signature: every test that builds the command must pass a `Path`. Miss one compile. Keep the path first or last consistently.
- Existence check vs argument tests: do not require `/usr/bin/bwrap` on disk inside `sandbox_command`. Resolve only at runtime spawn sites.
- Merged `/bin` → `/usr/bin`: FHS search plus FHS trust (after canonicalize) must accept both `/bin/sh` and `/usr/bin/sh`. Exec `/usr/bin/sh` (or `/bin/sh`), not the `dash`/`busybox` target.
- NixOS/Guix: a profile symlink into `/nix/store` or `/gnu/store` is accepted; `Command` program is the profile path. A search hit whose canonical target is outside trust roots is skipped. `/run/wrappers/bin` is both a search root (first) and a trust root.
- `omarchy` CLI may live only outside the allowlist; version detection must still use the existing version files, not PATH.
- In-process SHA-256 must match GNU `sha256sum` hex (lowercase) so published `.sha256` files keep working. Reuse `first_hash_token` for the remote file; do not parse subprocess output.
- Do not expand into #506 signed manifests, #334 archive-entry checks, or inner-jail helper names.

## Non-goals

- Exploits, planted-PATH demonstrations, or QA steps that install substitute helpers.
- Changing jail-internal `Command::new("ffprobe"|…)` in `sandbox_helper`.
- In-process release-archive extraction (`tar` crate) or signed-update protocol (#148 / #506 / #334).
- `xdg-terminal-exec` / `KNOWN_TERMINALS` in `src/ui/terminal.rs` (user-facing terminal launch; not listed on #1045).
- `build.rs` `git`, test-only `Command::new("sh"|"ffmpeg"|current_exe)`, `.desktop` Exec launches.
- Relabeling, reassigning away from `wmfeht`, or closing #1045 from this plan.

## Pipeline next step

1. Round 2 code on draft [#1055](https://github.com/lgse/strata/pull/1055) (`fix/1045-trusted-helper-paths`): two-list Nix/Guix lookup; exec the search hit.
2. Code-review → exploratory QA against `test-cases.md`. Leave draft. Do not squash. Do not send to Origin. Keep `working-docs/1045/`.
