# Code notes: #1045 round 2

Agent: `bc-9abf8ffd-b5b0-5611-83f1-958a6fe11409`

## What changed

Host helpers still never read inherited `PATH`. Lookup now uses two lists:

- **Search roots** (basename may appear): `/run/wrappers/bin` first, then `/usr/bin`, `/usr/sbin`, `/bin`, `/sbin`, `/run/current-system/sw/bin` (NixOS), `/run/current-system/profile/bin` (Guix). Never `$HOME` or user nix/guix profiles.
- **Trust roots** (canonicalize may land): FHS dirs, `/run/wrappers/bin` (NixOS wrappers are regular files, not store symlinks), `/nix/store`, `/gnu/store`.

`resolve` returns the search hit (e.g. `/run/current-system/sw/bin/bwrap` or `/usr/bin/sh`), not the canonical store/`dash` target, so argv0 stays intact. A search hit whose canonical path is outside trust roots is skipped.

Jail-internal `Command::new("ffprobe"|…)` is unchanged. In-process SHA-256 is unchanged.

## Files

- `src/trusted_command.rs` + `src/trusted_command/tests.rs` — two-list resolver; store-profile fixture; live `sh`/`tar` keep their basename
- `src/ui/settings/tests.rs` — `restart_waiter` program is a search-root path named `sh`
- `docs/preview-sandbox.md` — search hit vs trust-root canonical target
- `working-docs/1045/plan.md`, `test-cases.md` — round 2 rule

## Tests run

Private Xvfb, never `DISPLAY=:1`:

```text
cargo fmt --all --check
CXX=g++ LIBRARY_PATH=/usr/lib/gcc/x86_64-linux-gnu/13 \
  cargo clippy --all-targets --all-features -- -D warnings
xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  cargo test --all-targets --all-features -- <filter> --test-threads=1
```

| Filter | Result |
| --- | --- |
| `trusted_command::tests` | 5 passed |
| `sandbox::tests` | 24 passed |
| `services::update_install::tests` | 26 passed |
| `portal_setup::tests` | 23 passed |
| `restart_waiter` | 1 passed |
| `omarchy::tests` | 4 passed |
| `sandbox_helper::tests` (adjacent) | 10 passed |

Collection was nonzero (83 selected across the planned filters; 10 adjacent). `quality.sh` / `e2e.sh` not run: spawn program path only; isolation argv unchanged. `cargo-deny` / `typos` not on this image.

## Gaps / notes

- `/run/wrappers/bin` is a trust root as well as the first search root. Without that, NixOS `security.wrappers` binaries would fail the canonical-target check because they are not store symlinks.
- Case 1 still does not mutate `PATH` (`env::set_var` is unsafe; crate denies `unsafe_code`). Extra fixture dir omitted from search roots is enough.
- Live Ubuntu `sh` is now `/usr/bin/sh` (search hit), not canonical `dash`.
- Installed `libgstreamer1.0-dev` and `libgstreamer-plugins-base1.0-dev` on this VM so clippy/tests could compile `gstreamer-*-sys`. Host linker still needs `CXX=g++` and `LIBRARY_PATH` for `unrar_sys`.
- Did not undraft, squash, or send to Origin. `working-docs/1045/` left in place.
- `ManagePullRequest` could not update the lgse/strata PR description (workspace origin is `wmfeht/strata`). Branch push still moved the existing draft head.
