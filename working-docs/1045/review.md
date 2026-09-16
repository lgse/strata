# Review: lgse/strata#1055 (issue #1045 trusted helper paths)

- **PR:** https://github.com/lgse/strata/pull/1055
- **Title:** `fix(sandbox): resolve host helpers without PATH`
- **Product reviewed:** `ca0e3fe96a6f8a5c878b0080271aa54be11495f8`
- **Branch tip at review:** `f0e059e7b8beb77aa03b6f6ca69afef2c7970d5d` (round 2 code-notes only on top of product)
- **Head reviewed:** `ca0e3fe96a6f8a5c878b0080271aa54be11495f8` (`wmfeht:fix/1045-trusted-helper-paths`)
- **Base:** `944d67ce8bb3b9870c86dcb84123c0bc8b2b6715` (`lgse/strata` `main`)
- **Issue:** [#1045](https://github.com/lgse/strata/issues/1045)
- **Draft:** yes (left draft)
- **Agent:** `bc-b75980a6-797e-5127-96a9-d16b19c6a2a4`
- **Verdict:** `approve-with-comments`
- **PR comment posted:** no

Round 2 (Nix/Guix two-list lookup). Reviewed the product diff against `working-docs/1045/plan.md`, `test-cases.md`, `code-notes.md`, and `status.md`. Did not implement, undraft, squash, merge, or send to Origin. `working-docs/1045/` left in place. No GitHub review comment (no blockers).

## Intent

Host-side helpers that Strata execs by bare name must start from a trusted absolute **search hit**. Lookup may search only admin-managed roots (NixOS wrappers first, then FHS, then NixOS/Guix system profiles). It must never read inherited `PATH` or `$HOME`. After canonicalize, the target must sit under a separate trust-root list (FHS, wrappers, `/nix/store`, `/gnu/store`). Exec the search path, not the store/`dash` target. Missing `bwrap` / `tar` fail closed. Relaunch and desktop/portal helpers skip or warn as before. Update archives stay in-process SHA-256. Jail-internal helper names stay unchanged.

## Verdict

**approve-with-comments** — two-list resolver, exec-the-search-hit, call-site wiring, and the planned tests match William’s round 2 algorithm. No correctness, safety, or test-quality blockers. Nits do not require another code round.

## Blockers

None.

## Test quality (scanned)

Added/changed tests this round (vs round 1 `cb720b74`):

- `src/trusted_command/tests.rs`: split `resolve_in` into search vs trust; case 1–2 still one function each; case 3a still rejects an out-of-trust symlink; **new** `resolve_execs_search_path_when_canonical_target_is_in_store` for case 3b (profile symlink into a store fixture; `Ok` is `sw/bin/bwrap`, not the store target); live `sh`/`tar` smoke now asserts search-root path + basename `sh`/`tar` + canonical under `TRUST_ROOTS`
- `src/ui/settings/tests.rs`: `restart_waiter` program is a `SEARCH_ROOTS` path named `sh`
- Unchanged this round: sandbox `get_program` fixture, in-process checksum vectors

These are the six planned cases (3a and 3b as two functions in the same module; case 6 is live smoke plus the extracted waiter). Not a combinatorial matrix. No second gnu-store clone of the nix-store fixture. Not tautological as whole tests: the store-profile case would fail if `resolve` returned the canonical store path; the out-of-trust symlink case would fail if search and trust were collapsed; the live smoke would fail if `sh` became `dash`. `command(name)` vs `resolve(name)` equality inside the live smoke still restates the one-line wrapper; that single assertion is not a separate test and is not a blocker.

No `tests/` integration crate. No new E2E scenario. Jail-internal `ffprobe` / `dcraw` / `ffmpegthumbnailer` names untouched. Do not add more tests this round.

## Nits / suggestions

1. **`docs/preview-sandbox.md` trust sentence.** Search roots correctly include wrappers and profiles. The canonical-target sentence still says only FHS / `/nix/store` / `/gnu/store`. NixOS wrapper hits canonicalize under `/run/wrappers/bin`, which the code trusts. Optional one-word add; code is correct.
2. **Leftover from round 1 (not re-opened as blockers):** `sha2` still sits above `sevenz-rust2` in `Cargo.toml`; `hex_lower` is still a manual table; `restart_waiter_uses_absolute_sh` still skip-and-pass if `command("sh")` is `Err`.

## Notes / residual risk

- `SEARCH_ROOTS` order matches the required list: `/run/wrappers/bin` first, then `/usr/bin` `/usr/sbin` `/bin` `/sbin`, then `/run/current-system/sw/bin`, `/run/current-system/profile/bin`. `TRUST_ROOTS` are FHS, wrappers, `/nix/store`, `/gnu/store`. No `$HOME`, `/usr/local/bin`, `~/.nix-profile`, or `~/.guix-profile`. No `PATH` read.
- Basename-only names; empty / `.` / `..` / separators / NUL rejected. First search hit whose canonical path sits under a trust root wins. `Command::new` gets `found_path(candidate)` (the search-root path), not `canonicalize()`.
- `/run/wrappers/bin` is both first search root and a trust root so NixOS `security.wrappers` regular files are accepted. Profile symlinks are accepted only when the store target is trusted; the executed argv0 stays the profile path.
- Runtime `parse` / media `render` still wrap `resolve("bwrap")` as `Unable to start the preview sandbox: …`. `sandbox_command` still does not require the file on disk. Jail argv still `--unshare-all`, `--clearenv`, `--setenv PATH /usr/bin`. No unsandboxed preview fallback.
- `try_install` still fail-closes on `command("tar")?`. Desktop/portal/omarchy helpers still skip or warn without a relative name. `detected_major` still falls through to version files when `omarchy` is not on a search root.
- `sits_under` falls back to the absolute trust-root string when canonicalize of that root fails (so a missing `/nix/store` on FHS hosts does not disable other roots). Residual: a regular file sitting only under a profile dir (not a store symlink) is skipped; that matches the two-list rule.
- Test-only `Command::new("sh")` in sandbox fixtures, `sandbox_helper` jail names, and `xdg-terminal-exec` remain out of scope.
- GitHub Metadata policy fails on Cursor Agent commit identity. Expected for this draft; do not squash this round.
- PR `mergeable_state` was `behind` `lgse/strata` `main` at review time. Not a product defect.

## CI

Did not re-run full `quality.sh` / `e2e.sh`. Isolation argv and jail `PATH=/usr/bin` were not rewritten. Code notes on this SHA: targeted filters **83 passed** (5 `trusted_command` including the new store-profile case) plus 10 adjacent jail-helper tests, host `fmt` and clippy. Not re-verified here.

GitHub on `f0e059e7` (draft; product `ca0e3fe`):

| Check | Result |
| --- | --- |
| Format, lint, and test | skipped (draft) |
| Quality build and lint | skipped (draft) |
| Rust / E2E shards | skipped (draft) |
| Metadata policy | **failure** (agent attribution on commits) |
| Dependency and spelling / Release scripts / [code]smith | skipped |

## Head SHA reviewed

`ca0e3fe96a6f8a5c878b0080271aa54be11495f8`
