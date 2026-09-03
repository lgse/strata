#!/usr/bin/env bash
# Runs a disposable OpenSSH server for exercising Strata's sftp:// support.
#
# Everything it creates — host keys, the client key pair, the authorized_keys
# file and the served directory — lives under a single directory that is removed
# when the server stops, so no state leaks into the developer's ~/.ssh.
set -euo pipefail

port="${STRATA_SFTP_PORT:-2222}"
password="${STRATA_SFTP_PASSWORD:-}"
root="${STRATA_SFTP_ROOT:-}"
keep=""

usage() {
  cat >&2 <<'USAGE'
usage: scripts/sftp-fixture.sh [--port PORT] [--root DIR] [--keep]

Starts sshd on 127.0.0.1 and prints the sftp:// URI to open in Strata.
Press Ctrl-C to stop it and delete everything it created.

  --port PORT  listen port (default 2222, or $STRATA_SFTP_PORT)
  --root DIR   directory to serve (default: a generated sample tree)
  --keep       leave the fixture directory in place on exit

Password authentication is disabled unless $STRATA_SFTP_PASSWORD is set, since
sshd can only verify a password against a real system account.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --port) port="$2"; shift 2 ;;
    --root) root="$2"; shift 2 ;;
    --keep) keep="yes"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage; exit 2 ;;
  esac
done

sshd="$(command -v sshd || true)"
for candidate in /usr/sbin/sshd /usr/lib/ssh/sshd /usr/libexec/sshd; do
  [[ -n "$sshd" ]] && break
  [[ -x "$candidate" ]] && sshd="$candidate"
done
if [[ -z "$sshd" ]]; then
  echo "sshd not found. Install your distribution's OpenSSH server package." >&2
  exit 1
fi
command -v ssh-keygen >/dev/null || { echo "ssh-keygen not found." >&2; exit 1; }

fixture="$(mktemp -d "${TMPDIR:-/tmp}/strata-sftp-XXXXXX")"
chmod 700 "$fixture"

cleanup() {
  if [[ -f "$fixture/sshd.pid" ]]; then
    kill "$(cat "$fixture/sshd.pid")" 2>/dev/null || true
  fi
  if [[ -z "$keep" ]]; then
    rm -rf -- "$fixture"
  else
    echo "Fixture kept at $fixture" >&2
  fi
}
trap cleanup EXIT INT TERM

if [[ -z "$root" ]]; then
  root="$fixture/served"
  mkdir -p "$root/documents" "$root/images" "$root/nested/deeper"
  printf 'hello from the disposable sftp fixture\n' > "$root/readme.txt"
  printf '{"fixture": true}\n' > "$root/documents/sample.json"
  printf 'plain text\n' > "$root/nested/deeper/leaf.txt"
  head -c 4096 /dev/urandom > "$root/images/blob.bin"
fi
root="$(cd "$root" && pwd)"

ssh-keygen -q -t ed25519 -N '' -f "$fixture/host_ed25519" -C 'strata-sftp-fixture-host'
ssh-keygen -q -t ed25519 -N '' -f "$fixture/client_ed25519" -C 'strata-sftp-fixture-client'
# An encrypted copy of the same key, for exercising the passphrase prompt.
cp "$fixture/client_ed25519" "$fixture/client_ed25519_encrypted"
cp "$fixture/client_ed25519.pub" "$fixture/client_ed25519_encrypted.pub"
ssh-keygen -q -p -P '' -N "${STRATA_SFTP_PASSPHRASE:-strata}" \
  -f "$fixture/client_ed25519_encrypted" >/dev/null

cp "$fixture/client_ed25519.pub" "$fixture/authorized_keys"
chmod 600 "$fixture/authorized_keys" "$fixture/host_ed25519" "$fixture/client_ed25519" \
  "$fixture/client_ed25519_encrypted"

sftp_server=""
for candidate in /usr/lib/ssh/sftp-server /usr/lib/openssh/sftp-server \
  /usr/libexec/sftp-server /usr/libexec/openssh/sftp-server; do
  [[ -x "$candidate" ]] && { sftp_server="$candidate"; break; }
done
if [[ -z "$sftp_server" ]]; then
  echo "sftp-server binary not found; install the OpenSSH server package." >&2
  exit 1
fi

password_auth="no"
[[ -n "$password" ]] && password_auth="yes"

cat > "$fixture/sshd_config" <<CONFIG
ListenAddress 127.0.0.1
Port $port
HostKey $fixture/host_ed25519
PidFile $fixture/sshd.pid
AuthorizedKeysFile $fixture/authorized_keys
PasswordAuthentication $password_auth
KbdInteractiveAuthentication no
PubkeyAuthentication yes
UsePAM no
StrictModes no
PrintMotd no
X11Forwarding no
Subsystem sftp $sftp_server
LogLevel VERBOSE
CONFIG

"$sshd" -f "$fixture/sshd_config" -E "$fixture/sshd.log"

user="$(id -un)"
cat <<SUMMARY
Disposable SFTP fixture is listening on 127.0.0.1:$port

  Open in Strata:   sftp://$user@127.0.0.1:$port$root
  Client key:       $fixture/client_ed25519
  Encrypted key:    $fixture/client_ed25519_encrypted (passphrase: ${STRATA_SFTP_PASSPHRASE:-strata})
  Server log:       $fixture/sshd.log

The fixture host key is new, so the first connection must answer Strata's
host-key question. To exercise the changed-host-key path, connect once, stop
the fixture, start it again (it generates a fresh host key), and reconnect.

Press Ctrl-C to stop the server and delete the fixture.
SUMMARY

while [[ -f "$fixture/sshd.pid" ]] && kill -0 "$(cat "$fixture/sshd.pid")" 2>/dev/null; do
  sleep 1
done
