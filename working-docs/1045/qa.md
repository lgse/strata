# QA: #1045 trusted helper paths (round 2)

- **verdict:** `pass-with-nits`
- **staging_pr:** https://github.com/lgse/strata/pull/1055 (draft; left draft)
- **head_sha:** product `ca0e3fe96a6f8a5c878b0080271aa54be11495f8`; branch tip at QA start `a02106c2a6a6b4441542d197fda9823cdb984d78` (round 2 code-notes + review-record docs on top of that SHA)
- **issue:** [#1045](https://github.com/lgse/strata/issues/1045)
- **build:** `CXX=g++ LIBRARY_PATH=/usr/lib/gcc/x86_64-linux-gnu/13` plus `cargo test --all-targets --all-features` on private Xvfb (`env -u DISPLAY xvfb-run -a`, inner display `:99`, `GDK_BACKEND=x11`, `GTK_A11Y=none`, `NO_AT_BRIDGE=1`, `STRATA_REQUIRE_GTK_TESTS=1`). Never `DISPLAY=:1`.
- **agent_id:** `bc-fa81f022-b550-515b-9fa8-a9679b18ecd8`
- **n:** 1045 (`working-docs/1045/`)

Issue-scoped exploratory QA against round 2 `test-cases.md` (Nix/Guix two-list lookup; exec the search hit). Did not implement, undraft, squash, merge, or send to Origin. Did not run the breadth QA suite or full `e2e.sh` / `quality.sh`. Did not plant substitute helpers or rewrite `PATH` as an attack. Empty inherited `PATH` for the test process used a vacant directory with no named helpers.

## Product non-nits

None. Every numbered case passed.

## Findings — nits / process

1. **`restart_waiter_uses_absolute_sh` still skip-and-pass if `command("sh")` is `Err`.** Allowed by case 6. This host has `/usr/bin/sh` → `dash`. The test **did not skip**: it passed with the test-process `PATH` set to `/tmp/qa-1045-empty-path` (empty; no named substitutes), so lookup used search roots, not `PATH`. Executed program basename stayed `sh`, not `dash`.
2. **Draft CI skipped** on this SHA (expected). Metadata policy failure is agent attribution, not a product fail.
3. **`docs/preview-sandbox.md` canonical-target sentence** still names FHS / `/nix/store` / `/gnu/store` and omits `/run/wrappers/bin`. Code trusts wrappers. Review leftover; not a product break.
4. **Review leftovers** (`Cargo.toml` crate order, optional `hex_lower`) are not product breaks. Not re-opened.

## Coverage

Private Xvfb only (`:99`). Targeted filters (nonzero collection). Host helpers present: `bwrap`, `tar`, `sh` under `/usr/bin` (merged-usr `/bin` is the same files). `/run/wrappers/bin`, `/run/current-system/sw/bin`, `/run/current-system/profile/bin`, `/nix/store`, and `/gnu/store` are absent on this Ubuntu VM. `omarchy` / `hyprctl` absent from search roots. User nix/guix profiles absent.

| Case | Result |
| --- | --- |
| 1 Resolver uses only search roots | **pass** — `resolve_uses_only_allowlisted_directories`. Extra fixture `path/` dir with `bwrap` is omitted from the search list. `SEARCH_ROOTS` is wrappers-first, then FHS, then NixOS `sw/bin` and Guix `profile/bin`. No `$HOME`, `/usr/local/bin`, `~/.nix-profile`, or `~/.guix-profile`. Re-ran the five `trusted_command` tests with empty inherited `PATH`: still **5 passed**. |
| 2 Rejects bad names and misses | **pass** — `resolve_rejects_bad_names_and_misses` (`""`, `.`, `..`, `usr/bin/bwrap`, missing `bwrap`, empty search list). `command("strata-missing-trusted-helper")` is `Err`. |
| 3 Canonical target under a trust root; exec the search hit | **pass** — 3a `resolve_requires_final_path_under_a_trusted_directory`: in-trust `tar` accepted as the search path; symlink whose canonical target is outside trust roots rejected. 3b `resolve_execs_search_path_when_canonical_target_is_in_store`: profile `sw/bin/bwrap` → store fixture; `Ok` is `sw/bin/bwrap`, not the store target; basename stays `bwrap`. Live FHS: `/usr/bin/sh` is the search hit (basename `sh`); canonical `/usr/bin/dash` still sits under FHS trust. |
| 4 Preview sandbox program is absolute bubblewrap | **pass** — `sandbox_command_starts_absolute_bubblewrap`: `get_program()` is `/usr/bin/bwrap`; argv still `--unshare-all`, `--clearenv`, `--setenv PATH /usr/bin`. Runtime `parse` / media `render` wrap `resolve("bwrap")` as `Unable to start the preview sandbox: …`. Isolation tests in `sandbox::tests` still **24 passed**. |
| 5 Update checksum is in-process SHA-256 | **pass** — `archive_checksum_hashes_in_process` plus `first_hash_token_*`. Empty digest `e3b0c442…`; `abc` digest `ba7816bf…`. Uppercase published token matches. Mismatch string is `Downloaded update failed checksum verification`. No `sha256sum` in `src/`. Independent Python `hashlib` and GNU `sha256sum` produced the same hex. |
| 6 Host tar / sh / portal helpers are absolute | **pass** — live `sh` / `tar` resolve under `SEARCH_ROOTS` with basename preserved; canonical under `TRUST_ROOTS`. `restart_waiter` program is a search-root path named `sh` (not skipped). Portal/omarchy tests **23 + 4 passed**. `try_install` uses `trusted_command::command("tar")?` (fail closed). Desktop-cache / `xdg-mime` / `gdbus` / `systemctl` / `hyprctl` go through `trusted_command`; no live `systemctl` spawn. `detected_major` still falls through to version files when `omarchy` is missing. |

Adjacent (not numbered):

- Jail-internal `Command::new("ffprobe"|"dcraw"|"simple_dcraw"|"ffmpegthumbnailer"|"prlimit")` unchanged vs PR base `944d67c` (empty diff under `src/sandbox_helper*`). `sandbox_helper::tests` **10 passed**.
- No production `Command::new("bwrap"|"tar"|"sha256sum"|"sh"|…)`. `sandbox_command` takes `&Path` and `Command::new(bwrap)`.
- `trusted_command.rs` never reads `PATH` or `$HOME`. Remaining `PATH` readers are `src/ui/terminal.rs` (`xdg-terminal-exec`, out of scope) and `src/services/install_source.rs` (install marker, out of scope).
- First search hit whose canonical path sits under a trust root wins. Executed path is `found_path(candidate)` (search hit), not `canonicalize()`.

## Gaps

- **Live NixOS wrappers / NixOS sw / Guix profile / store roots:** `/run/wrappers/bin`, `/run/current-system/sw/bin`, `/run/current-system/profile/bin`, `/nix/store`, and `/gnu/store` do not exist on this host. Wrappers-first and Guix `profile/bin` + `/gnu/store` are covered by the constant lists plus the nix-store profile fixture (case 3b). No second gnu-store clone (intentional). Did not create those directories or plant helpers.
- **Live missing `bwrap` / `tar`:** both exist on this Ubuntu host (`/usr/bin/bwrap`, `/usr/bin/tar`). Did not hide or replace them. Fail-closed miss is the unit `Err` plus `?` / `Unable to start the preview sandbox: …` wrapping. Case 4 allows this.
- **Live preview spawn** of a real thumbnail via `parse` was not driven in the GUI. `sandbox_command` wiring and isolation argv were asserted; cancel/oversize `parse` tests fail before resolve.
- **Canonical `./scripts/e2e.sh`** not rerun (issue-scoped; no new E2E scenario).
- Host linker still needs `CXX=g++` (`unrar_sys` `<new>`) and `LIBRARY_PATH` for `-lstdc++`. Installed `libgstreamer1.0-dev` so `gstreamer-1.0.pc` existed. Host setup, not a product defect.

## Commands / results

```bash
env -u DISPLAY xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
  GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  cargo test --all-targets --all-features -- <filter> --test-threads=1
```

| Filter | Result |
| --- | --- |
| `trusted_command::tests` | **5 passed** |
| `sandbox::tests` | **24 passed** |
| `services::update_install::tests` | **26 passed** |
| `portal_setup::tests` | **23 passed** |
| `restart_waiter` | **1 passed** |
| `omarchy::tests` | **4 passed** |
| `sandbox_helper::tests` (adjacent) | **10 passed** |

Planned filters **83 passed**, 0 failed. Adjacent jail helper tests **10 passed**. Empty-`PATH` re-run of `trusted_command::tests` and `restart_waiter`: **5 + 1 passed**.

No GTK criticals in these logs. Harmless `libEGL` DRI3 noise is possible on Xvfb; these filters did not print it.

## Evidence

Store paths (each verified present):

| File | What it shows |
| --- | --- |
| `round2-targeted-tests.log` | Six planned filters, 83 passed; Xvfb `:99` |
| `round2-empty-path-tests.log` | Same resolver + restart_waiter with empty inherited `PATH` |
| `round2-sandbox-helper.log` | Jail-internal helper tests, 10 passed |
| `round2-hash-vectors.txt` | Python and GNU `sha256sum` match the in-process vectors |
| `round2-live-fhs.txt` | Read-only live FHS first-hits (`/usr/bin/{bwrap,tar,sh}`); wrappers/Nix/Guix dirs absent |

Directory: `/cursor/stores/bc-715d6aa6-e6df-4e3a-8d69-f0567526c53b/media/pipeline-1045/`

No screenshots or video: process spawn and hashing only; no user-visible UI change.

## Stop condition

Review `approve-with-comments` + QA `pass-with-nits` with **no** product non-nits → no further code round for #1045 Nix/Guix two-list lookup.

Product: `working-docs/1045/qa.md`, `working-docs/1045/status.md`.
