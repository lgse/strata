# Remote locations and saved connections

Strata browses remote servers through GIO/GVfs. Locations stay URI-native
(`smb://`, `sftp://`, `ftp://`, `ftps://`, `dav://`, `davs://`), and the GVfs
backend does the network work. This page describes the behavior Strata adds on
top of GVfs and how to exercise it.

## Network discovery

The **Network** place opens `network:///`, which GVfs fills from its discovery
backends (DNS-SD, SMB browsing, WS-Discovery). The column explains each state
instead of showing a blank view:

| State | Shown when |
| --- | --- |
| Empty | Discovery ran but nothing advertised itself. |
| Discovery unavailable | GVfs network support, or every discovery backend, is missing. |
| Discovery failed | Discovery was blocked or returned an error. |

Every state keeps **Ctrl+L** direct entry available and says so. The Network
place always opens, so these states appear in the column, not as error
dialogs.

## Connecting

All remote mounts use `adapters::remote_mount::MountSession`. It routes every
GVfs prompt to Strata's own dialogs:

- **Passwords and passphrases.** Strata asks for a password, or for an SSH key
  or volume passphrase. If the server rejects it, Strata asks again, but only
  when the backend reports the rejection, so a wrong password never loops.
  Credentials typed into a URI answer only the first prompt and are never
  stored with the location.
- **Trust questions.** For unknown SSH host keys, keys that differ from the
  key recorded for the address, and untrusted TLS certificates, Strata shows
  the backend's text and every choice the backend offers. The accepting choice
  is styled as dangerous and is never focused or chosen by default. Closing the
  dialog or pressing Escape declines.
- **Changed host keys.** OpenSSH refuses these outright. Strata then explains
  that the server's identity changed. It doesn't offer to override the refusal.
- **Plaintext transport.** Before the first new connection to an `ftp://` or
  `dav://` server in a session, Strata warns that credentials and files are
  unencrypted and requires **Connect Anyway**. This warning concerns transport
  only; it never affects certificate validation for `ftps://` or `davs://`.
  Locations that are already mounted don't ask again.

Cancelling a prompt or declining a trust question returns to the previous
location without adding history. Other failures show sanitized, actionable
messages. These cover a missing backend, host not found, connection refused,
timeout, permission, sign-in, host key, and certificate failures. Messages
never repeat hosts, users, paths, or secrets. `https://` entered in Ctrl+L is
rejected with a hint to use `davs://`; it is never reinterpreted as WebDAV.

If a mount disappears while you're browsing it, Strata cancels that column's
work, closes the deeper columns, and leaves the column showing
**This location is unavailable**. **Retry** reconnects, prompting if needed,
and reloads. Strata doesn't navigate away.

## Mounted network rows

Mounts for any remote protocol appear under **Devices** with network
presentation. **Disconnect** is offered only when the mount reports that it
can be unmounted. If other applications keep files open, Strata asks before
disconnecting, and it reports disconnects that are busy or fail. The sidebar
refreshes after every disconnect.

## Saved connections

Connections are named destinations in their own **Connections** section. They
are separate from **Pinned** folders (GTK bookmarks) and from transient
**Devices**. Add one with the **+** on the Connections heading, **Add
connection…** on the Network place's menu, or **Save Connection…** on a
mounted network row. After you connect directly to a server that isn't saved,
Strata shows a **Save Connection…** offer. It never saves on its own.

The form accepts a protocol, a server, an optional port, an optional username,
and either an SMB share and folder or a remote path. For WebDAV it also accepts
an `https://` endpoint, which it normalizes to `davs://`. The form has no
password field.

Selecting a connection reuses a matching GVfs mount when one exists, and
otherwise connects and prompts through GVfs. When a connection matches a
mount, the mount appears only as the connection row, which shows whether the
connection is online. Row menus (secondary click, **Menu**, or **Shift+F10**)
offer these actions:

- **Rename…** changes only Strata's label.
- **Edit…** validates and changes the destination.
- **Disconnect** appears only when the mount can be unmounted.
- **Remove…** deletes only Strata's record. If the connection is still open,
  Strata offers to disconnect as a separate step. It never touches files on
  the server or passwords saved in the desktop keyring.

### Storage

Connections are stored in `$XDG_CONFIG_HOME/strata/connections.json`:

```json
{
  "version": 1,
  "connections": [
    {
      "id": "8c1f…",
      "name": "Backups",
      "protocol": "sftp",
      "uri": "sftp://alice@nas.example:2222/srv/backups"
    }
  ]
}
```

- Writes are atomic and private (`0600`, in a `0700` directory). Strata refuses
  to replace a symlink or other non-regular file.
- Strata keeps fields and entries it doesn't understand when it rewrites the
  file. Files without a version are read as version 1.
- A file from a newer format version, or one that can't be parsed, is shown
  where possible but never rewritten.
- Destinations are canonical. Scheme and host are lowercase; default ports are
  removed; percent-encoding and dot segments are normalized; trailing
  separators are dropped; SMB shares and folders compare case-insensitively.
  Exact duplicates are rejected, but connections that differ by user, port, or
  path are allowed.
- Passwords, passphrases, keys, tokens, and URI authentication parameters are
  never written. Fields with those names are dropped even if another version
  added them.

## Disposable server fixtures

`scripts/remote-fixtures.py` starts disposable OpenSSH (SFTP), vsftpd
(FTP and explicit FTPS), and Apache (WebDAV over HTTP and HTTPS) containers.
They're published only on `127.0.0.1` and use self-signed certificates. The
client side is isolated as follows:

- It runs on a private D-Bus session that can activate only the GVfs daemon.
- It uses private XDG directories and has no keyring service.
- An `ssh` wrapper supplies a private `known_hosts` file and fixture-only keys,
  so your `~/.ssh` and desktop session are never used.

```bash
./scripts/remote-fixtures.py test            # run the ignored fixture tests, then clean up
./scripts/remote-fixtures.py test --keep     # leave servers running afterwards
./scripts/remote-fixtures.py up              # start servers and print endpoints
./scripts/remote-fixtures.py shell gio mount sftp://strata@127.0.0.1:PORT/
./scripts/remote-fixtures.py down            # stop servers and remove state
```

`test` runs `adapters::remote_mount::tests::fixtures` with `--ignored`. The
tests cover the following cases through the same mount driver the UI uses:

- SFTP: password sign-in with a wrong-password retry, key authentication, an
  encrypted-key passphrase, cancellation, unknown and changed host keys,
  refused and unknown hosts, browsing, and a disconnect.
- FTP and WebDAV: plaintext policy, sign-in with a retry, abandoning bad
  credentials, and browsing.
- FTPS and DAVS: untrusted-certificate decisions, then sign-in and browsing.

The fixtures need a container engine (Docker or Podman), GVfs with its SFTP,
FTP, and DAV backends, the OpenSSH client, and `dbus-run-session`. Building
the fixture images downloads Alpine packages.

Browsing an FTPS or DAVS server with a certificate the system already trusts
needs a certificate signed by a trusted CA. The self-signed fixtures don't
provide that, so check that case manually against such a server.
