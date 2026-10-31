#!/usr/bin/env bash
# Drives one `strata --qa` session by hand inside the pinned image, one JSON
# command at a time, with a private Xvfb, bus, HOME, and fixture.
#   ./scripts/qa-drive.sh start [--tree JSON|@FILE] [--pref KEY=VALUE]... [--env NAME=VALUE]... [--bus]
#   ./scripts/qa-drive.sh send JSON...           one reply line per command, in order
#   ./scripts/qa-drive.sh screenshot NAME.png    the Xvfb root, saved as target/qa-drive/NAME.png
#   ./scripts/qa-drive.sh restart [--pref KEY=VALUE]... [--env NAME=VALUE]...
#   ./scripts/qa-drive.sh exec SHELL-COMMAND     run in the fixture ($FIXTURE), with the session's HOME, XDG, DISPLAY, bus
#   ./scripts/qa-drive.sh log                    the application's log
#   ./scripts/qa-drive.sh stop
# A --pref value is JSON, or else a string. Every exchange is appended to
# target/qa-drive/transcript.jsonl as {"command":...,"reply":...}. One session
# per checkout; it shares ./scripts/qa.sh's build cache, so never run the two at once.
# See "Driving a session by hand" in docs/testing.md.
set -euo pipefail

unset DISPLAY WAYLAND_DISPLAY NOTIFY_SOCKET
repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
session=/tmp/qa-drive
shared=/workspace/target/qa-drive
display=:90

# Must match how tests/qa.rs launches each run; scripts/test_qa_drive.py checks them.
xvfb_arguments=(-screen 0 1440x900x24 -dpi 96 -nolisten tcp -noreset -ac)
qa_environment=(GTK_A11Y=none NO_AT_BRIDGE=1)
xdg_directories=(HOME=home XDG_CONFIG_HOME=config XDG_DATA_HOME=data XDG_CACHE_HOME=cache XDG_STATE_HOME=state)
fixture_prefix=strata-qa-fixture-
no_session_bus=no-session-bus
launch_timeout=90
command_timeout=90
wait_margin=30
exit_timeout=10
terminate_grace=5
session_bus_config='<!DOCTYPE busconfig PUBLIC
 "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={socket}</listen>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
'

usage() {
  sed -n '2,14s/^# \{0,1\}//p' "${BASH_SOURCE[0]}"
}

inside() {
  [[ "${STRATA_QA_CONTAINER:-}" == 1 ]] || {
    echo "qa-drive.sh $1 runs only inside its container" >&2
    exit 2
  }
}

prepare() {
  python3 - "$@" <<'PYTHON'
import json, os, re, sys

def toml(value):
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return json.dumps(value)
    if isinstance(value, list):
        return "[" + ", ".join(toml(item) for item in value) + "]"
    if isinstance(value, str):
        return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'
    raise SystemExit(f"settings.toml cannot hold {value!r}")

def changed_prefs():
    for line in filter(None, os.environ.get("QA_DRIVE_PREFS", "").split("\n")):
        key, _, text = line.partition("=")
        try:
            value = json.loads(text)
        except ValueError:
            value = text
        toml(value)
        yield key, value

def populate(directory, tree):
    for name, value in tree.items():
        path = os.path.join(directory, name)
        if isinstance(value, dict):
            os.makedirs(path, exist_ok=True)
            os.chmod(path, 0o755)
            populate(path, value)
        elif isinstance(value, str):
            with open(path, "w") as file:
                file.write(value)
            os.chmod(path, 0o644)
        else:
            raise SystemExit(f"fixture {name} must be a file's text or a directory")

def pin(path, mtime):
    for root, directories, files in os.walk(path, topdown=False):
        for name in files + directories:
            os.utime(os.path.join(root, name), (mtime, mtime), follow_symlinks=False)
    os.utime(path, (mtime, mtime))

if sys.argv[1] == "prefs":
    path = sys.argv[2]
    lines = open(path).read().splitlines()
    for key, value in changed_prefs():
        entry = f"{key} = {toml(value)}"
        tables = [index for index, line in enumerate(lines) if line.startswith("[")]
        top = tables[0] if tables else len(lines)
        found = [index for index in range(top) if re.match(rf"\s*{re.escape(key)}\s*=", lines[index])]
        if found:
            lines[found[0]] = entry
        else:
            lines.insert(top, entry)
    open(path, "w").write("\n".join(lines) + "\n")
    raise SystemExit

session, fixture = sys.argv[2], sys.argv[3]
manifest = json.load(open("/workspace/tests/fixtures/harness.json"))
home = f"{session}/home"
os.makedirs(f"{home}/config/gtk-4.0", exist_ok=True)
with open(f"{home}/config/gtk-4.0/settings.ini", "w") as file:
    file.write(manifest["gtk_settings"])
os.makedirs(f"{home}/config/strata", exist_ok=True)
with open(f"{home}/config/strata/{manifest['portal_offer_marker']}", "w") as file:
    file.write("1\n")
preferences = dict(manifest["preferences"])
preferences.update(changed_prefs())
with open(f"{home}/config/strata/settings.toml", "w") as file:
    file.writelines(f"{key} = {toml(value)}\n" for key, value in preferences.items())

tree = os.environ.get("QA_DRIVE_TREE") or "canonical"
tree = manifest["trees"][tree] if tree in manifest["trees"] else json.loads(tree)
if not isinstance(tree, dict):
    raise SystemExit("--tree must be a manifest tree's name or a JSON object")
populate(fixture, tree)
pin(fixture, manifest["mtime"])

with open(f"{session}/app.env", "w") as file:
    for key, value in manifest["environment"].items():
        file.write(f"{key}={value}\n")
PYTHON
}

session_variables() {
  local pair
  for pair in "${xdg_directories[@]}"; do
    printf '%s\n' "${pair%%=*}=$session/home/${pair#*=}"
  done
  printf '%s\n' "DISPLAY=$display" "XDG_RUNTIME_DIR=$session/runtime" \
    "DBUS_SESSION_BUS_ADDRESS=$(cat "$session/bus")"
}

launch_app() {
  local -a environment variables
  mapfile -t environment < "$session/app.env"
  mapfile -t variables < <(session_variables)
  rm -f "$session/in" "$session/replies"
  mkfifo "$session/in"
  : > "$session/replies"
  echo 1 > "$session/sent"
  setsid env -i --chdir="$(cat "$session/fixture")" PATH="$PATH" "${environment[@]}" \
    "${qa_environment[@]}" "${variables[@]}" \
    STRATA_INPUT_LOG="$shared/input.log" MESA_SHADER_CACHE_DIR="$session/shaders" \
    "${CARGO_TARGET_DIR:?}/debug/strata" --qa "$(cat "$session/fixture")" \
    < "$session/in" > "$session/replies" 2> "$session/strata.log" &
  echo "$!" > "$session/app.pid"
  # Holds the command pipe open read-write, so a send never blocks on a dead reader.
  sleep infinity <> "$session/in" > /dev/null 2>&1 &
  echo "$!" > "$session/keeper.pid"
  await_reply 1 "$launch_timeout" launch
}

# A reply arriving after its send gave up keeps its number, so it is never
# taken for a later command's.
await_reply() {
  local number="$1" deadline=$((SECONDS + $2)) app
  app="$(cat "$session/app.pid")"
  while (($(wc -l < "$session/replies") < number)); do
    if ! kill -0 "$app" 2> /dev/null; then
      echo "strata exited; log tail:" >&2
      tail -n 40 "$session/strata.log" | sed 's/\x1b\[[0-9;]*m//g' >&2
      return 1
    fi
    if ((SECONDS >= deadline)); then
      echo "no reply to $3 within $2 s; its reply will be skipped if it comes" >&2
      echo "(read ./scripts/qa-drive.sh log, then restart, or stop and start)" >&2
      return 1
    fi
    sleep 0.05
  done
  sed -n "${number}p" "$session/replies"
}

stop_app() {
  [[ -f "$session/app.pid" ]] || return 0
  local app signal wait
  app="$(cat "$session/app.pid")"
  kill "$(cat "$session/keeper.pid")" 2> /dev/null || true
  for signal in NONE TERM KILL; do
    wait="$terminate_grace"
    if [[ "$signal" == NONE ]]; then
      wait="$exit_timeout"
    else
      kill -s "$signal" -- "-$app" 2> /dev/null || true
    fi
    for _ in $(seq $((wait * 20))); do
      kill -0 "$app" 2> /dev/null || break 2
      sleep 0.05
    done
  done
  rm -f "$session/app.pid"
}

case "${1:-}" in
  _build)
    inside "$1"
    mkdir -p "$HOME" "$CARGO_HOME" "$shared"
    if ! cargo test --locked --features qa,invariant-checks --test qa --no-run > "$shared/build.log" 2>&1; then
      tail -n 40 "$shared/build.log" >&2
      exit 1
    fi
    exit 0
    ;;
  _launch)
    inside "$1"
    mkdir -p "$session/runtime" "$session/shaders" "$shared"
    chmod 700 "$session/runtime"
    for pair in "${xdg_directories[@]}"; do
      mkdir -p "$session/home/${pair#*=}"
    done
    # The tmpfs hides the image's /tmp, so its X socket directory is made again.
    mkdir -p -m 1777 /tmp/.X11-unix
    fixture="$(mktemp -d "/tmp/${fixture_prefix}XXXXXX")"
    echo "$fixture" > "$session/fixture"
    prepare seed "$session" "$fixture"
    printf '%s\n' "${QA_DRIVE_ENV:-}" | sed '/^$/d' >> "$session/app.env"
    setsid Xvfb "$display" "${xvfb_arguments[@]}" < /dev/null > "$session/xvfb.log" 2>&1 &
    for _ in $(seq 300); do
      [[ -S "/tmp/.X11-unix/X${display#:}" ]] && break
      sleep 0.05
    done
    [[ -S "/tmp/.X11-unix/X${display#:}" ]] || {
      echo "Xvfb did not start:" >&2
      cat "$session/xvfb.log" >&2
      exit 1
    }
    if [[ "${QA_DRIVE_BUS:-0}" == 1 ]]; then
      socket="$session/runtime/bus"
      printf '%s' "${session_bus_config//\{socket\}/$socket}" > "$session/session-bus.conf"
      setsid dbus-daemon --nofork --nopidfile --config-file="$session/session-bus.conf" \
        < /dev/null > "$session/dbus-daemon.log" 2>&1 &
      for _ in $(seq 600); do
        [[ -S "$socket" ]] && break
        sleep 0.05
      done
      [[ -S "$socket" ]] || {
        echo "the private session bus did not start:" >&2
        cat "$session/dbus-daemon.log" >&2
        exit 1
      }
      echo "unix:path=$socket" > "$session/bus"
    else
      # An address that cannot connect keeps GLib from autolaunching a bus.
      echo "unix:path=$session/runtime/$no_session_bus" > "$session/bus"
    fi
    echo "fixture: $fixture" >&2
    launch_app
    exit
    ;;
  _send)
    inside "$1"
    shift
    for command in "$@"; do
      checked="$(python3 - "$command" "$command_timeout" "$wait_margin" <<'PYTHON'
import json, sys
try:
    command = json.loads(sys.argv[1])
except ValueError as error:
    sys.exit(f"not JSON ({error}): {sys.argv[1]}")
default, margin = int(sys.argv[2]), int(sys.argv[3])
timeout = command.get("timeout_ms") if isinstance(command, dict) else None
print(json.dumps(command, ensure_ascii=False, separators=(",", ":")))
print(max(default, int(timeout) // 1000 + margin if isinstance(timeout, (int, float)) else default))
PYTHON
)" || exit 2
      line="$(sed -n 1p <<< "$checked")"
      number=$(($(cat "$session/sent") + 1))
      echo "$number" > "$session/sent"
      printf '%s\n' "$line" > "$session/in"
      reply="$(await_reply "$number" "$(sed -n 2p <<< "$checked")" "$line")"
      printf '{"command":%s,"reply":%s}\n' "$line" "$reply" >> "$shared/transcript.jsonl"
      printf '%s\n' "$reply"
    done
    exit
    ;;
  _screenshot)
    inside "$1"
    import -display "$display" -window root "$shared/$2"
    echo "target/qa-drive/$2"
    exit
    ;;
  _restart)
    inside "$1"
    stop_app
    prepare prefs "$session/home/config/strata/settings.toml"
    printf '%s\n' "${QA_DRIVE_ENV:-}" | sed '/^$/d' >> "$session/app.env"
    launch_app
    exit
    ;;
  _exec)
    inside "$1"
    cd "$(cat "$session/fixture")"
    mapfile -t variables < <(session_variables)
    env FIXTURE="$PWD" "${variables[@]}" bash -c "$2"
    exit
    ;;
  _stop)
    inside "$1"
    stop_app
    cp "$session/strata.log" "$shared/strata.log" 2> /dev/null || true
    exit 0
    ;;
esac

# shellcheck source=lib/qa-container.sh
source "$repository/scripts/lib/qa-container.sh"
qa_container_engine
name="strata-qa-drive-$(cd "$repository" && pwd -P | tr -d '\n' | sha256sum | cut -c1-8)"
exists() {
  command -v "$engine" > /dev/null 2>&1 && "$engine" container inspect "$name" > /dev/null 2>&1
}
running() {
  [[ "$("$engine" container inspect --format '{{.State.Running}}' "$name" 2> /dev/null)" == true ]]
}
need_session() {
  running || {
    echo "No QA drive session is running; start one with ./scripts/qa-drive.sh start" >&2
    exit 1
  }
}
run_inside() {
  "$engine" exec "$@"
}

collect_options() {
  prefs=""
  extra=""
  bus=0
  tree=""
  while (($#)); do
    case "$1" in
      --pref)
        prefs+="${2:?--pref needs KEY=VALUE}"$'\n'
        shift 2
        ;;
      --env)
        extra+="${2:?--env needs NAME=VALUE}"$'\n'
        shift 2
        ;;
      --bus)
        bus=1
        shift
        ;;
      --tree)
        tree="${2:?--tree needs JSON or @FILE}"
        if [[ "$tree" == @* ]]; then tree="$(cat "${tree#@}")"; fi
        shift 2
        ;;
      *)
        echo "unknown option $1" >&2
        usage >&2
        exit 2
        ;;
    esac
  done
}

case "${1:-}" in
  start)
    shift
    collect_options "$@"
    if exists; then
      echo "A QA drive session container ($name) exists; ./scripts/qa-drive.sh stop removes it." >&2
      exit 1
    fi
    qa_container_prepare
    qa_container_options --detach --init --name "$name"
    qa_container_mounts
    rm -rf "$repository/target/qa-drive"
    mkdir -p "$repository/target/qa-drive"
    # Set before the container exists, so a failed start always removes it.
    trap '"$engine" rm --force "$name" > /dev/null 2>&1 || true' EXIT
    "$engine" run "${options[@]}" "${mounts[@]}" "$image" sleep infinity > /dev/null
    echo "building strata --qa (log: target/qa-drive/build.log)" >&2
    run_inside "$name" bash /workspace/scripts/qa-drive.sh _build
    run_inside --env "QA_DRIVE_PREFS=$prefs" --env "QA_DRIVE_ENV=$extra" \
      --env "QA_DRIVE_BUS=$bus" --env "QA_DRIVE_TREE=$tree" \
      "$name" bash /workspace/scripts/qa-drive.sh _launch
    trap - EXIT
    ;;
  send)
    shift
    (($#)) || {
      echo "send needs at least one JSON command" >&2
      exit 2
    }
    need_session
    run_inside "$name" bash /workspace/scripts/qa-drive.sh _send "$@"
    ;;
  screenshot)
    file="${2:?screenshot needs a file name such as menu.png}"
    if [[ "$file" == */* || "$file" != *.png ]]; then
      echo "screenshot takes a plain file name ending in .png" >&2
      exit 2
    fi
    need_session
    run_inside "$name" bash /workspace/scripts/qa-drive.sh _screenshot "$file"
    ;;
  restart)
    shift
    collect_options "$@"
    if ((bus)) || [[ -n "$tree" ]]; then
      echo "restart keeps the session's bus and fixture; stop and start again to change them" >&2
      exit 2
    fi
    need_session
    run_inside --env "QA_DRIVE_PREFS=$prefs" --env "QA_DRIVE_ENV=$extra" \
      "$name" bash /workspace/scripts/qa-drive.sh _restart
    ;;
  exec)
    need_session
    run_inside "$name" bash /workspace/scripts/qa-drive.sh _exec "${2:?exec needs a shell command}"
    ;;
  log)
    need_session
    run_inside "$name" sed 's/\x1b\[[0-9;]*m//g' "$session/strata.log"
    ;;
  stop)
    if exists; then
      if running; then
        run_inside "$name" bash /workspace/scripts/qa-drive.sh _stop || true
      fi
      "$engine" rm --force "$name" > /dev/null
      echo "stopped $name"
    else
      echo "no QA drive session is running"
    fi
    ;;
  -h | --help | help)
    usage
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
