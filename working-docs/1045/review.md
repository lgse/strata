# Review: lgse/strata#1055 (issue #1045 trusted helper paths)

- **PR:** https://github.com/lgse/strata/pull/1055
- **Title:** `fix(sandbox): resolve host helpers without PATH`
- **Head reviewed:** `cb720b74b9a7bf43b12757bad51ae9de3494d1ff` (`wmfeht:fix/1045-trusted-helper-paths`)
- **Base:** `944d67ce8bb3b9870c86dcb84123c0bc8b2b6715` (`lgse/strata` `main`)
- **Issue:** [#1045](https://github.com/lgse/strata/issues/1045)
- **Draft:** yes (left draft)
- **Agent:** `bc-200bb883-d299-52b9-97d3-ab49c855ce9f`
- **Verdict:** `approve-with-comments`
- **PR comment posted:** no

Round 1. Reviewed the product diff against `working-docs/1045/plan.md`, `test-cases.md`, `code-notes.md`, and `status.md`. Did not implement, undraft, squash, merge, or send to Origin. `working-docs/1045/` left in place.

## Intent

Host-side helpers that Strata execs by bare name must start from a trusted absolute path under `/usr/bin`, `/usr/sbin`, `/bin`, `/sbin`. Lookup must not read inherited `PATH`. Missing `bwrap` / `tar` fail closed. Relaunch and desktop/portal helpers skip or warn as before. Update archives are hashed in-process with SHA-256 instead of `sha256sum`. Jail-internal helper names stay unchanged.

## Verdict

**approve-with-comments** — resolver, call-site wiring, in-process checksum, and the planned test set match the plan. No correctness, safety, or test-quality blockers. Nits do not require another code round.

## Blockers

None.

## Test quality (scanned)

Added/changed tests:

- `src/trusted_command/tests.rs` (new): allowlist-only lookup, bad names/misses, out-of-allowlist symlink rejected, live `sh`/`tar` smoke plus missing-helper `Err`
- `src/sandbox/tests.rs`: inject `Path::new("/usr/bin/bwrap")` on existing argv fixtures; one new representative `get_program` + isolation-flag test
- `src/services/update_install/tests.rs`: empty and `abc` SHA-256 vectors, uppercase published token, mismatch string
- `src/ui/settings/tests.rs`: `restart_waiter` program is absolute and under the allowlist

These are the six planned cases. Not a combinatorial matrix. Not tautological as whole tests: fixture lookup would fail if PATH-extra dirs were searched or if an out-of-allowlist symlink were accepted; checksum vectors would fail on the wrong digest or hex case; `get_program` on the injected bubblewrap path is the planned wiring check (would fail if `sandbox_command` ignored the `Path` and used a relative `"bwrap"`). `command(name)` vs `resolve(name)` equality inside the live smoke restates the one-line wrapper; that single assertion is not a separate test and is not a blocker.

No `tests/` integration crate. No new E2E scenario. Jail-internal `ffprobe` / `dcraw` / `ffmpegthumbnailer` names untouched. Do not add more tests this round.

## Nits / suggestions

1. **`Cargo.toml` crate order.** `sha2 = "0.11.0"` was inserted above `sevenz-rust2`. Optional swap for the surrounding alpha order.
2. **`hex_lower`.** Correct, and the empty/`abc` vectors pin lowercase GNU `sha256sum` hex. `format!("{:x}", hasher.finalize())` would drop the helper if a later pass wants less surface.
3. **`restart_waiter_uses_absolute_sh` skip-and-pass.** If `command("sh")` is `Err`, the test returns without asserting. Allowed by case 6 when the helper is absent. Code notes recorded 1 passed on this Ubuntu host (`/bin/sh` → `dash`). Exploratory QA still needs a live relaunch when `sh` exists.

## Notes / residual risk

- `trusted_command::resolve` never reads `PATH`. Allowlist order matches `PACMAN` (`/usr/bin` first). Basename-only names; `.` / `..` / separators rejected. Canonical final path must stay under a canonical allowlisted directory; merged-usr `/bin` → `/usr/bin` is accepted. `Command::new` always gets that absolute path.
- Runtime `parse` / media `render` resolve `"bwrap"` and wrap misses as `Unable to start the preview sandbox: …`. `sandbox_command` itself does not require the file on disk. Jail argv still `--unshare-all`, `--clearenv`, `--setenv PATH /usr/bin`; in-sandbox `/usr/bin/prlimit` and `/usr/bin/ffmpegthumbnailer` unchanged. No unsandboxed preview fallback.
- `sha256sum` subprocess is gone. `first_hash_token` still lowercases the published `.sha256` body. `sha2` 0.11.0 was already in `Cargo.lock` (crates.io, Apache-2.0 / MIT).
- `omarchy` outside the allowlist falls through to `/usr/share/omarchy/version` and `~/.local/share/omarchy/version` (intended). `hyprctl` / desktop-cache / portal helpers skip or error without a relative name.
- Test-only `Command::new("sh")` in sandbox fixtures, `sandbox_helper` jail names, and `xdg-terminal-exec` are out of scope and were not changed.
- Case 1 did not mutate `PATH` (`set_var` is unsafe; crate denies `unsafe_code`). Third fixture dir omitted from the injected allowlist is enough.
- GitHub Metadata policy fails on Cursor Agent commit identity. Expected for this draft; do not squash this round.
- `status.md` at this tip listed `1f2cf0d812d693c13982915efaa37bf23eedd0e4`; reviewed code HEAD is `cb720b74` (status-note commit on top of `1f2cf0d8`).
- PR `mergeable_state` was `behind` `lgse/strata` `main` at review time. Not a product defect.

## CI

Did not re-run full `quality.sh` / `e2e.sh`. Isolation argv and jail `PATH=/usr/bin` were not rewritten. Targeted re-run on this VM failed to compile: host `pkg-config` has no `gstreamer-1.0.pc` (rebuild of `gstreamer-sys`). Not treated as a product failure.

GitHub on `cb720b74` (draft):

| Check | Result |
| --- | --- |
| Format, lint, and test | skipped (draft) |
| Quality build and lint | skipped (draft) |
| Rust / E2E shards | skipped (draft) |
| Metadata policy | **failure** (agent attribution on commits) |
| Dependency and spelling / Release scripts / [code]smith | skipped |

Code notes: targeted filters **82 passed**, 0 failed under private Xvfb, plus host `fmt` and clippy. Not re-verified here.

## Head SHA reviewed

`cb720b74b9a7bf43b12757bad51ae9de3494d1ff`
