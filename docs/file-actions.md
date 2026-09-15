# User file actions

Strata's context menus can run **out-of-process** commands you define. The
application never loads plugins into its address space; each action is an
argv launched with the selected native paths.

This is how “Sync to this device” (Resilio) and “Send via LocalSend” appear
without baking those tools into Strata.

## File

`$XDG_CONFIG_HOME/strata/actions.toml` (usually `~/.config/strata/actions.toml`).

A missing file is normal: the menus stay as they shipped. Invalid TOML is
logged and ignored so a typo cannot keep Strata from starting. Changes are
read when Strata launches; restart after editing.

## Schema

```toml
[[actions]]
id = "localsend"
label = "Send via LocalSend"
icon = "strata-external-link"
command = ["localsend", "--headless", "send"]
targets = "any"
requires = ["localsend"]
```

| Field | Required | Meaning |
| --- | --- | --- |
| `id` | yes | Stable identifier. Duplicates are skipped. |
| `label` | yes | Context-menu text. |
| `command` | yes | Program and arguments. No shell. Native paths are appended unless a token is used. |
| `icon` | no | A bundled Strata icon name such as `strata-external-link`, `strata-download`, `strata-folder`, `strata-globe`. Unknown names fall back to `strata-external-link`. |
| `targets` | no | `any` (default), `files`, or `folders`. |
| `requires` | no | Extra binaries that must exist on `PATH` before the item is shown. The first `command` word is always required. |

Path tokens, each as a whole argument:

- `{path}` — the single selected path. Hidden when more than one item is selected.
- `{paths}` — replaced by every selected path, in order.

Trash, remote/GVfs locations without a native path, and the file chooser never
show these actions.

## Examples

Resilio Sync selective download (same helper the Nautilus extension calls):

```toml
[[actions]]
id = "resilio-sync-to-device"
label = "Sync to this device"
icon = "strata-download"
command = ["omarchy-rslsync-add"]
targets = "any"
requires = ["omarchy-rslsync-add"]
```

LocalSend:

```toml
[[actions]]
id = "localsend"
label = "Send via LocalSend"
icon = "strata-external-link"
command = ["localsend", "--headless", "send"]
targets = "any"
requires = ["localsend"]
```

## Isolation

Commands run as a detached child with stdin, stdout, and stderr discarded.
Strata does not wait for them and does not pass a shell. A missing binary
shows an error dialog; a running tool is responsible for its own UI.

A future public extension protocol should stay out of process as well; see
[Architecture principles](architecture.md).
