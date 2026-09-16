# QA: #1045 trusted helper paths (round 1)

- **verdict:** `pass-with-nits`
- **staging_pr:** https://github.com/lgse/strata/pull/1055 (draft; left draft)
- **head_sha:** product `cb720b74b9a7bf43b12757bad51ae9de3494d1ff`; branch tip at QA start `0c3efe7d9f076248c353a60b14c87cf61a522104` (review-record docs only on top of that SHA)
- **issue:** [#1045](https://github.com/lgse/strata/issues/1045)
- **build:** `CXX=g++ LIBRARY_PATH=/usr/lib/gcc/x86_64-linux-gnu/13` plus `cargo test --all-targets --all-features` on private Xvfb (`xvfb-run -a`, `GDK_BACKEND=x11`, `GTK_A11Y=none`, `NO_AT_BRIDGE=1`, `STRATA_REQUIRE_GTK_TESTS=1`). Never `DISPLAY=:1`.
- **agent_id:** `bc-44c2a0ef-24f3-511e-bd0d-a5c88b0f159f`
- **n:** 1045 (`working-docs/1045/`)

Issue-scoped exploratory QA against `test-cases.md`. Did not implement, undraft, squash, merge, or send to Origin. Did not run the breadth QA suite or full `e2e.sh` / `quality.sh`. Did not plant substitute helpers or rewrite `PATH` as an attack.

## Product non-nits

None. Every numbered case passed.

## Findings — nits / process

1. **`restart_waiter_uses_absolute_sh` still skip-and-pass if `command("sh")` is `Err`.** Allowed by case 6. This host has `/usr/bin/sh` → `dash`. The test **did not skip**: it passed with inherited `PATH` set to an empty directory (`/tmp/qa-1045-empty-path`), so lookup used the allowlist, not `PATH`.
2. **Draft CI skipped** on this SHA (expected). Metadata policy failure is agent attribution, not a product fail.
3. **Review leftovers** (`Cargo.toml` crate order, optional `hex_lower`) are not product breaks. Not re-opened.

## Coverage

Private Xvfb only. Targeted filters (nonzero collection). Host helpers present: `bwrap`, `tar`, `sh` under `/usr/bin` (merged-usr `/bin` is the same files). `omarchy` / `hyprctl` absent.

| Case | Result |
| --- | --- |
| 1 Resolver uses only allowlisted directories | **pass** — `resolve_uses_only_allowlisted_directories`. Extra fixture `path/` dir with `bwrap` is omitted from the allowlist. Re-ran the four `trusted_command` tests with `PATH=/tmp/qa-1045-empty-path` (empty; no named substitutes): still **4 passed**. |
| 2 Rejects bad names and misses | **pass** — `resolve_rejects_bad_names_and_misses` (`""`, `.`, `..`, `usr/bin/bwrap`, missing `bwrap`, empty allowlist). `command("strata-missing-trusted-helper")` is `Err`. |
| 3 Final path stays under a trusted directory | **pass** — in-allowlist `tar` accepted; symlink to an out-of-allowlist `bwrap` rejected. Merged-usr `/bin/sh` → `/usr/bin/dash` accepted on the live `sh` smoke. |
| 4 Preview sandbox program is absolute bubblewrap | **pass** — `sandbox_command_starts_absolute_bubblewrap`: `get_program()` is `/usr/bin/bwrap`; argv still `--unshare-all`, `--clearenv`, `--setenv PATH /usr/bin`. Runtime `parse` / media `render` wrap `resolve("bwrap")` as `Unable to start the preview sandbox: …`. Isolation tests in `sandbox::tests` still **24 passed**. |
| 5 Update checksum is in-process SHA-256 | **pass** — `archive_checksum_hashes_in_process` plus `first_hash_token_*`. Empty digest `e3b0c442…`; `abc` digest `ba7816bf…`. Uppercase published token matches. Mismatch string is `Downloaded update failed checksum verification`. No `sha256sum` in `src/`. Independent Python `hashlib` and GNU `sha256sum` produced the same hex. |
| 6 Host tar / sh / portal helpers are absolute | **pass** — live `sh` / `tar` resolve under the allowlist; `restart_waiter` program is absolute (not skipped). Portal/omarchy tests **23 + 4 passed**. `try_install` uses `trusted_command::command("tar")?` (fail closed). Desktop-cache / `xdg-mime` / `gdbus` / `systemctl` / `hyprctl` go through `trusted_command`; no live `systemctl` spawn. |

Adjacent (not numbered):

- Jail-internal `Command::new("ffprobe"|"dcraw"|"simple_dcraw"|"ffmpegthumbnailer"|"prlimit")` unchanged vs `origin/main`. `sandbox_helper::tests` **10 passed**.
- No production `Command::new("bwrap"|"tar"|"sha256sum"|"sh"|…)`.
- `trusted_command.rs` never reads `PATH`. Remaining `PATH` readers are `src/ui/terminal.rs` (`xdg-terminal-exec`, out of scope) and `src/services/install_source.rs` (install marker, out of scope).
- `omarchy` is missing on this host; `detected_major` still has the version-file fallback. `omarchy::tests` passed.

## Gaps

- **Live missing `bwrap` / `tar`:** both exist on this Ubuntu host (`/usr/bin/bwrap`, `/usr/bin/tar`). Did not hide or replace them. Fail-closed miss is the unit `Err` plus `?` / `Unable to start the preview sandbox: …` wrapping. Case 4 allows this.
- **Live preview spawn** of a real thumbnail via `parse` was not driven in the GUI. `sandbox_command` wiring and isolation argv were asserted; cancel/oversize `parse` tests fail before resolve.
- **Canonical `./scripts/e2e.sh`** not rerun (issue-scoped; no new E2E scenario).
- First compile on this VM needed `CXX=g++` (`unrar_sys` `<new>`) and `LIBRARY_PATH` for `-lstdc++`. Host linker setup, not a product defect. `libgstreamer1.0-dev` was installed so `gstreamer-1.0.pc` existed.

## Commands / results

```bash
xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
  GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  cargo test --all-targets --all-features -- <filter> --test-threads=1
```

| Filter | Result |
| --- | --- |
| `trusted_command::tests` | **4 passed** |
| `sandbox::tests` | **24 passed** |
| `services::update_install::tests` | **26 passed** |
| `portal_setup::tests` | **23 passed** |
| `restart_waiter` | **1 passed** |
| `omarchy::tests` | **4 passed** |
| `sandbox_helper::tests` (adjacent) | **10 passed** |

Planned filters **82 passed**, 0 failed. Adjacent jail helper tests **10 passed**. Empty-`PATH` re-run of `trusted_command::tests` and `restart_waiter`: **4 + 1 passed**.

Harmless `libEGL` DRI3 noise is possible on Xvfb; these filters did not print GTK criticals.

## Evidence

Store paths (each verified present):

| File | What it shows |
| --- | --- |
| `targeted-tests.log` | Six planned filters, 82 passed |
| `empty-path-tests.log` | Same resolver + restart_waiter with empty inherited `PATH` |
| `sandbox_helper.log` | Jail-internal helper tests, 10 passed |
| `hash-vectors.txt` | Python and GNU `sha256sum` match the in-process vectors |

Directory: `/cursor/stores/bc-715d6aa6-e6df-4e3a-8d69-f0567526c53b/media/pipeline-1045/`

No screenshots or video: process spawn and hashing only; no user-visible UI change.

## Stop condition

Review `approve-with-comments` + QA `pass-with-nits` with **no** product non-nits → no further code round for #1045 trusted helper paths.

Product: `working-docs/1045/qa.md`, `working-docs/1045/status.md`.
