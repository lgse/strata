# Test cases: #1045 trusted helper paths

Narrow set. Defense-in-depth: assert lookup and hashing behavior. Do not plant substitute helpers, rewrite PATH as an attack, or add an E2E hijack scenario. Code review treats excess/tautological tests as blockers.

## 1. Resolver uses only allowlisted directories

- **Purpose:** Host helpers resolve by basename under the injected directory list, never inherited `PATH`.
- **Setup:** `resolve_in` with two fixture dirs. Put a regular file `bwrap` only in the first. Optionally set `PATH` to a third fixture dir that also contains a `bwrap` file so the test proves `PATH` is unused (the third dir is not in the allowlist).
- **Steps:** `resolve_in("bwrap", &[first, second])`.
- **Expected:** `Ok` path is `first/bwrap` (absolute). Not the `PATH` dir. Not a relative name.
- **Automation:** `src/trusted_command/tests.rs`.

## 2. Resolver rejects bad names and misses

- **Purpose:** Fail closed; no `Command` with a relative program name.
- **Setup:** Same fixture helper. Empty allowlist or empty dirs.
- **Steps:** Resolve `""`, `.`, `..`, `usr/bin/bwrap`, `bwrap` when absent.
- **Expected:** All `Err`. No panic.
- **Automation:** Same module. One test function is enough.

## 3. Final path stays under a trusted directory

- **Purpose:** After following links, the chosen path must still be under an allowlisted directory (merged-usr: `/bin` → `/usr/bin` is acceptable when both are allowlisted).
- **Setup:** Fixture allowlist. A regular file inside it. A link whose final target is outside the allowlist must not be selected.
- **Expected:** In-allowlist file accepted; out-of-allowlist final path rejected.
- **Automation:** Same module. Skip the link case if the platform cannot create it; then note in `code-notes.md` and keep cases 1–2.

## 4. Preview sandbox command program is an absolute bubblewrap path

- **Purpose:** `sandbox_command` no longer uses the relative name `bwrap`.
- **Setup:** Existing `sandbox_command` argument fixtures in `src/sandbox/tests.rs`. Pass `Path::new("/usr/bin/bwrap")` (file need not exist).
- **Steps:** Build one representative command (image thumbnail or PDF is enough; do not duplicate every sandbox argv test).
- **Expected:** `command.get_program()` is `/usr/bin/bwrap`. Argv still includes `--unshare-all`, `--clearenv`, `--setenv PATH /usr/bin`. Isolation flags unchanged.
- **Automation:** Extend `src/sandbox/tests.rs`. Missing `bwrap` on disk at a runtime `resolve("bwrap")` seam should surface as a start error string, not a relative `Command`. If `parse` is too heavy for that miss case, unit-test the resolve error only (case 2) and keep this case as `get_program`.

## 5. Update checksum is in-process SHA-256

- **Purpose:** Drop the `sha256sum` subprocess; published `.sha256` first-token compare still works.
- **Setup:** Temp file with known bytes. Empty file digest `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` (SHA-256 of empty input). One nonempty fixture with a precomputed lowercase hex.
- **Steps:** Hash via the new helper (extract if `verify_checksum` still needs HTTP). Compare equal / unequal.
- **Expected:** Match succeeds; mismatch returns the existing verification failure string. `first_hash_token` still lowercases the published token. No `Command` program `sha256sum`.
- **Automation:** `src/services/update_install/tests.rs`. Do not hit the network.

## 6. Host tar / sh / portal helpers are absolute

- **Purpose:** Remaining listed execs use `trusted_command` (or the resolved `Path`) rather than bare names.
- **Setup:** If call sites go through `trusted_command::command("tar")` etc., assert `get_program()` is absolute and a basename of `tar` / `sh` / `xdg-mime` (one example each, or inspect a small extracted builder). Prefer checking the resolved path on a fixture via `resolve_in` plus a smoke that production call sites compile against `resolve`/`command` — do not spawn `systemctl` in unit tests.
- **Expected:** Program is an absolute path under the allowlist when the helper exists on the test host; `Err` when absent. No relative `"tar"` / `"sh"`.
- **Automation:** `trusted_command` tests plus, if a relaunch command builder is extracted from `settings.rs` `restart`, one `get_program()` assertion in `src/ui/settings/tests.rs`. Skip live `systemctl`/`hyprctl` spawns.

## Out of scope

- Jail-internal `ffprobe` / `dcraw` / `ffmpegthumbnailer` names.
- New `tests/e2e` scenario.
- Signed-manifest (#506) or archive-entry policy (#334).
- Terminal launcher PATH (`xdg-terminal-exec`).
- Visual/CSS.
