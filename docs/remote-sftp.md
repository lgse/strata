# SFTP locations

Strata reaches `sftp://` locations through GIO/GVfs, the same URI-native path it
uses for every other remote scheme. This page covers what the client supports
and how to exercise it against a disposable server.

## Addresses

| Address | Meaning |
| --- | --- |
| `sftp://host/path` | Connect as the local user on port 22 |
| `sftp://user@host/path` | Connect as `user` |
| `sftp://user@host:2222/path` | Connect as `user` on port 2222 |

Passwords are never accepted in the address bar. A password typed there is
stripped from the URI, used once for the connection, and never saved.

## Authentication

The SFTP backend decides which credentials it needs and asks Strata for them:

- **Password** — the backend requests a username and password. Strata shows a
  sign-in dialog with only the fields the backend asked for, so SMB's domain and
  anonymous options do not appear on an SFTP prompt.
- **SSH key** — the agent or an unencrypted key answers without any prompt.
- **Encrypted key** — the backend asks for the key's passphrase through the same
  request, and Strata labels the field "Passphrase" rather than "Password".

Wrong credentials reopen the prompt with a message naming only the fields the
prompt is showing. Cancelling returns to the previous location without recording
history.

## Host keys

An unrecognized or changed host key reaches Strata as a `GMountOperation`
question, and is always answered by the user:

- The dialog shows the backend's own text, including the key fingerprint, and one
  button per choice the backend offered.
- The declining choice is focused, so pressing Enter never trusts a key.
- Escape, the close button, and a click on the backdrop do not silently accept:
  the backdrop is inert and the other two decline explicitly.
- A changed host key is styled as a warning, since it can mean interception.

Declining ends the attempt where a cancelled sign-in ends: back at the previous
location, with no retry prompt.

## Failures

Missing backends, name-resolution failures, refused connections, timeouts,
unreachable hosts and rejected host keys each map to their own actionable
message. Backend text that reaches a dialog has any URI user-info stripped first.

Logs never carry a host, path, username, or secret at the default level. `INFO`
records only the backend name and outcome; the location is logged at `DEBUG`,
already redacted of user-info, query and fragment.

## Disposable test server

`scripts/sftp-fixture.sh` runs an OpenSSH server on `127.0.0.1` whose host key,
client keys, `authorized_keys`, and served files all live in one temporary
directory that is deleted when the server stops. Nothing is written to `~/.ssh`.

```console
$ ./scripts/sftp-fixture.sh --port 2222
Disposable SFTP fixture is listening on 127.0.0.1:2222

  Open in Strata:   sftp://you@127.0.0.1:2222/tmp/strata-sftp-XXXXXX/served
  Client key:       /tmp/strata-sftp-XXXXXX/client_ed25519
  Encrypted key:    /tmp/strata-sftp-XXXXXX/client_ed25519_encrypted (passphrase: strata)
```

Options: `--port`, `--root DIR` to serve an existing directory, and `--keep` to
leave the fixture in place. `STRATA_SFTP_PASSPHRASE` changes the encrypted key's
passphrase. Password authentication stays off unless `STRATA_SFTP_PASSWORD` is
set, because `sshd` can only check a password against a real system account.

### Manual matrix

Run these against the fixture. Each row is a user-visible behaviour that has no
automated coverage, because the crate is a binary with no library target for
integration tests and mounting through GVfs needs a live session bus.

| Case | How | Expected |
| --- | --- | --- |
| Unknown host key | Connect for the first time | Host-key dialog, declining choice focused |
| Declined host key | Answer Cancel | Returns to the previous location, no sign-in prompt |
| Accepted host key | Answer the accepting choice | Connection continues |
| Key authentication | `ssh-add` the client key, reconnect | Browses with no prompt |
| Encrypted key | `ssh-add` the encrypted key, reconnect | Passphrase prompt, field labelled "Passphrase" |
| Wrong passphrase | Answer with the wrong one | Prompt reopens with the retry message |
| Cancelled sign-in | Escape the prompt | Previous location, no history entry |
| Non-default port | Use `sftp://user@127.0.0.1:PORT/path` | Browses normally |
| Changed host key | Restart the fixture, reconnect | Warning-styled dialog, or a `known_hosts` message |
| Connection refused | Stop the fixture, reconnect | "The host refused the connection…" |
| Host not found | Use a name that does not resolve | "That host couldn't be found…" |
| Navigation | Breadcrumbs, history, Miller descent, hover peek | Behave as on local paths |
| Export | Copy, drag out, and open a remote file | Handled by the default application |
