# Cursor Cloud Agent: running tests

This is the machine-specific runbook for the Cursor Cloud Agent
environment used with `lgse/strata`. It records commands that were
verified on this VM. Product rules in `AGENTS.md` still apply; this
file only explains how to satisfy them here.

The Cloud Agent desktop is an XFCE session on TigerVNC `DISPLAY=:1`.
Never run GTK or E2E tests against that display. Do not use
`./scripts/e2e-native.sh` as a substitute for `./scripts/e2e.sh`.

## Machine profile

Observed on this snapshot (Ubuntu 24.04.4 LTS, user `ubuntu`, uid/gid
`1000`, workspace `/workspace`):

| Item | Value |
| --- | --- |
| CPUs / RAM | 4 logical CPUs, 15 GiB RAM, no swap |
| Rust | 1.98.1 at `/usr/local/cargo` (`CARGO_HOME=/usr/local/cargo`, `RUSTUP_HOME=/usr/local/rustup`) |
| Components | `rustfmt`, `clippy` installed; `cargo-deny` and `typos` are not |
| GTK | GTK 4.14.5, GtkSourceView 5.12.0, Poppler 24.02.0 |
| Headless X | `xvfb-run`, `Xvfb`, `at-spi2-core`, `python3-gi`, `gir1.2-atspi-2.0`, `dbus-daemon`, ImageMagick `import`, Cantarell fonts |
| E2E venv | `/opt/e2e-venv` (pytest 9.1.1); the start script links it to `/workspace/target/e2e-venv` |
| Docker | 29.1.3 (`docker.io`), no Podman |
| Docker daemon | `fuse-overlayfs`, `iptables: false`, `ip6tables: false`, `bridge: none` |

The git checkout baked into the snapshot can be an hour or more behind
`origin/main`. Fetch before branching or comparing against latest
`main`:

```bash
git fetch origin main
git checkout -B <branch> origin/main
```

## Confirm the start script ran

The personal environment `start` script (`/tmp/cursor/start-user/start-user.sh`)
must have exited 0. It:

1. Creates `/workspace/target` and links `/workspace/target/e2e-venv` → `/opt/e2e-venv`.
2. Starts `dockerd` if `docker info` fails, using `/etc/docker/daemon.json`.
3. Runs `sudo chmod 666 /var/run/docker.sock`.

If Docker is down later in the session, restart it the same way. Do not
enable the default bridge or iptables; this nested VM does not provide
them.

```bash
sudo docker info >/dev/null
ls -l /workspace/target/e2e-venv
test -x /opt/e2e-venv/bin/python
```

`./scripts/check.sh` is useful for format and Clippy, but it unsets
`DISPLAY` and therefore skips display-dependent Rust tests. Use the
commands below for a full suite.

## Full local CI (format, lint, Rust tests)

Run these from `/workspace`. The test command uses a private Xvfb
display and disables accessibility bridging, as required by `AGENTS.md`.

```bash
cargo fmt --all --check

cargo clippy --all-targets --all-features -- -D warnings

xvfb-run -a env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
  GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  cargo test --all-targets --all-features
```

`./scripts/test-headless.py` is an equivalent wrapper for the Rust
suite. Prefer the `xvfb-run` form above so the invocation matches
`AGENTS.md`.

`cargo-deny` and `typos` are not installed on this image.
`./scripts/check.sh` will skip them. Do not treat a local skip as a CI
pass; GitHub Actions still runs those jobs.

## End-to-end container suite

`./scripts/e2e.sh` is the pre-push gate. It builds
`tests/e2e/Dockerfile` and runs the suite inside that image.

This VM's Docker daemon has **no default `bridge` network**. A stock
`docker build` cannot resolve apt hosts and fails with `Temporary
failure resolving 'archive.ubuntu.com'`. `docker pull` from the host
works. `docker build --network=host` and `docker run --network=host`
work.

Put a host-network Docker wrapper first on `PATH`, then call the
canonical script. Do not commit the wrapper.

```bash
mkdir -p /tmp/docker-hostnet
cat > /tmp/docker-hostnet/docker <<'EOF'
#!/bin/bash
set -euo pipefail
real=/usr/bin/docker
case "${1:-}" in
  build) shift; exec "$real" build --network=host "$@" ;;
  run)   shift; exec "$real" run --network=host "$@" ;;
  *)     exec "$real" "$@" ;;
esac
EOF
chmod +x /tmp/docker-hostnet/docker

export PATH="/tmp/docker-hostnet:$PATH"
export STRATA_E2E_WORKERS="${STRATA_E2E_WORKERS:-auto}"

./scripts/e2e.sh
```

The auto worker budget on this 4-CPU / 15 GiB machine is 2, the same
as the CI runner. Override only when debugging:

```bash
STRATA_E2E_WORKERS=1 PATH="/tmp/docker-hostnet:$PATH" ./scripts/e2e.sh -n 0
```

First image build installs GTK, Xvfb, and Rust 1.98.1 from the pinned
Ubuntu snapshot and takes several minutes. Later runs reuse
`target/e2e-container`. Failure artifacts land in `target/e2e-artifacts`.

Native debugging (`./scripts/e2e-native.sh`) can use the preinstalled
`/opt/e2e-venv` and host GTK 4.14.5. It does not replace
`./scripts/e2e.sh`.

## What not to do

- Do not run `cargo test` or E2E with `DISPLAY=:1` (the Cloud Agent
  desktop). That maps real windows onto the VNC session.
- Do not set `STRATA_CONTAINER_ENGINE=podman`; Podman is not installed.
- Do not fall back to Broadway or the host Wayland/X11 display if Xvfb
  fails. Fix Xvfb instead.
- Do not change `/etc/docker/daemon.json` to enable `bridge` or
  `iptables` unless you are prepared to recover `dockerd`. The host
  network wrapper is the supported workaround.
- Do not push until format, Clippy, the isolated Rust suite, and
  `./scripts/e2e.sh` have all passed.

## Verified on this snapshot

Ran on 2026-09-07 against `origin/main` at `ab0cdc8` (Strata 0.12.0)
inside Cloud Agent run `bc-e9e926b7-3910-4572-ac73-35846d9c7804`.

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | passed |
| `cargo clippy --all-targets --all-features -- -D warnings` | passed (`dev` compile 39.61s after a warm `target/`) |
| Isolated `cargo test --all-targets --all-features` | **991 passed**, 0 failed, **19 ignored** in 46.97s (test compile 24.62s) |
| `PATH=/tmp/docker-hostnet:$PATH ./scripts/e2e.sh` | **329 passed** in 367.06s (0:06:07) |

The ignored Rust tests are expected on this VM: they require GVfs Trash,
`dbus-run-session`, `xdotool`, or “run this test alone” mapped-window
fixtures. `STRATA_REQUIRE_GTK_TESTS=1` still executed the GTK cases that
can share the private Xvfb display.

E2E container banner from this run:

```text
GTK: 4.14.5
rustc 1.98.1 (48a229cea 2026-09-01)
E2E resources: 4 CPUs, 13.8 GiB available; 2 workers
```

Cold E2E image build (apt snapshot packages, rustup 1.98.1, Python venv)
is the long pole; the in-container `cargo build --locked --bin strata`
then took 54.55s. Later runs reuse `target/e2e-container`.

Harmless noise seen here:

- `libEGL warning: DRI3 error: Could not get DRI3 device` during GTK
  Rust tests on Xvfb. Software rendering still works.
- Docker prints `DEPRECATED: The legacy builder is deprecated` because
  Buildx is not installed. The host-network wrapper still applies to
  `docker build` / `docker run`.
