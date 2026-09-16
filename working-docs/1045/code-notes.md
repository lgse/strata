# Code notes: #1045 round 1

Agent: `bc-91ed4417-8d89-5141-91b4-addf8f99e9e0`

## What changed

Host helpers are resolved by basename under `/usr/bin`, `/usr/sbin`, `/bin`, `/sbin` only. Lookup never reads `PATH`. The chosen path is canonicalized and must still sit under an allowlisted directory (merged-usr `/bin` → `/usr/bin` is accepted). Invalid names and misses return `Err`.

`sha256sum` is gone. Update archives are hashed in-process with `sha2` 0.11.0 (already in `Cargo.lock`; promoted to a direct dependency). Published `.sha256` first-token compare is unchanged.

Jail-internal `Command::new("ffprobe"|…)` in `sandbox_helper` is unchanged.

## Files

- `src/trusted_command.rs` + `src/trusted_command/tests.rs` — resolver and `command(name)`
- `src/sandbox.rs`, `src/sandbox/media.rs`, `src/sandbox/tests.rs` — `sandbox_command` takes a bubblewrap `&Path`; runtime `parse` / media `render` resolve `"bwrap"` and fail with `Unable to start the preview sandbox: …`
- `src/services/update_install.rs` — in-process SHA-256; `tar` / desktop-cache helpers via `trusted_command`
- `src/ui/settings.rs` — `restart_waiter` resolves `sh`; missing helper skips spawn (no relative `"sh"`)
- `src/portal_setup.rs`, `src/portal_setup/omarchy.rs` — `xdg-mime`, `gdbus`, `systemctl`, `omarchy`, `hyprctl`. Failed `omarchy` resolve falls through to version files
- `docs/preview-sandbox.md` — host starts bubblewrap from a trusted absolute path
- `Cargo.toml` / `Cargo.lock` — direct `sha2 = "0.11.0"`

## Tests run

Private Xvfb, never `DISPLAY=:1`:

```text
cargo fmt --all --check
CXX=g++ cargo clippy --all-targets --all-features -- -D warnings
xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  cargo test --all-targets --all-features <filter>
```

| Filter | Result |
| --- | --- |
| `trusted_command::tests` | 4 passed |
| `sandbox::tests` | 24 passed |
| `services::update_install::tests` | 26 passed |
| `portal_setup::tests` | 23 passed |
| `restart_waiter` | 1 passed |
| `omarchy::tests` (extra, omarchy call site) | 4 passed |

Collection was nonzero (82 selected across those filters). `quality.sh` / `e2e.sh` not run: spawn program path only; `--unshare-all` / `--clearenv` / jail `PATH=/usr/bin` argv unchanged. `cargo-deny` / `typos` not on this image.

## Gaps / notes

- Case 1 did not mutate `PATH` (`env::set_var` is unsafe in Rust 2024; this crate denies `unsafe_code`). A third fixture dir with `bwrap` is omitted from the allowlist instead.
- Canonical `sh` on this Ubuntu host is `dash`. Tests assert the path is absolute and under the allowlist, not that the final file name is still `sh`.
- `sandbox_command` has 8 arguments; `#[expect(clippy::too_many_arguments)]` with reason, matching existing GTK helpers.
- Did not undraft, squash, or send to Origin. `working-docs/1045/` left in place.
