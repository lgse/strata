#!/usr/bin/env bash
# Runs the end-to-end GUI suite exactly as CI does.
#
#   ./scripts/e2e.sh                       # every scenario
#   ./scripts/e2e.sh -k drag               # one selection
#   STRATA_E2E_UPDATE_BASELINES=1 ./scripts/e2e.sh -k baseline
set -euo pipefail

# Never let dependency checks or a failed harness startup reach the desktop.
unset DISPLAY WAYLAND_DISPLAY

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
suite="$repository/tests/e2e"
venv="${STRATA_E2E_VENV:-$repository/target/e2e-venv}"

missing=()
for tool in Xvfb dbus-daemon dbus-send import; do
  command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done
if ! python3 -c 'import gi; gi.require_version("Atspi", "2.0"); from gi.repository import Atspi' 2>/dev/null; then
  missing+=("python3 AT-SPI bindings (python3-gi and gir1.2-atspi-2.0)")
fi
if ((${#missing[@]})); then
  echo "missing dependencies: ${missing[*]}" >&2
  echo "see docs/e2e-testing.md for the package list" >&2
  exit 1
fi

# PyGObject comes from the system; only pytest and Pillow are installed here.
if [[ ! -x "$venv/bin/python" ]]; then
  echo "Creating the end-to-end virtual environment in $venv"
  python3 -m venv --system-site-packages "$venv"
  "$venv/bin/pip" install --quiet --requirement "$suite/requirements.txt"
fi

if [[ -z "${STRATA_BINARY:-}" ]]; then
  cargo build --manifest-path "$repository/Cargo.toml" --bin strata
fi

cd "$repository"
exec "$venv/bin/python" -m pytest -c "$suite/pytest.ini" --rootdir "$repository" "$@"
