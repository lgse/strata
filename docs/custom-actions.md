# Custom actions

Custom actions add your own scripts to the file and folder context menus.

They are ordinary programs running with your permissions. There is no sandbox and
no install step beyond enabling an action. Strata validates the definition and the
files around it, prepares a private working directory, and reports progress and
results; it does not restrict what a script may do.

## Where actions live

```text
$XDG_CONFIG_HOME/strata/actions/
  checksums/
    action.toml
    run.sh
```

One directory per action, named after its `id`, containing `action.toml` and
optionally one script beside it. Settings → Actions creates, edits, duplicates,
imports, and exports these files, and hand-editing them is supported: opening the
editor shows whatever is on disk.

## Creating an action

Choose **Settings → Actions → New action…**. The editor has three tabs:

- **General**: name, directory id, description, bundled icon, and enabled state.
  Leave the id blank to derive it from the name.
- **Script**: choose Python, Bash, or Command. Scripts have a line-numbered editor;
  commands take an installed program and one argument per line. Switching runtimes
  keeps each script draft until you close the dialog.
- **Behavior**: execution mode, per-item failure policy, working folder, menu
  placement, file/folder filters, selection limit, and confirmation.

**Create action** saves all three tabs together. Invalid fields bring you back to
that tab without discarding your edits. Clicking outside the dialog leaves it open;
**Cancel**, the close button, or **Escape** discards the draft.
Existing actions use the same editor with **Save changes**; their id stays fixed.

## Manifest

```toml
schema_version = 1
id = "checksums"
name = "Generate checksums"
description = "Writes a sha256 file next to each selection."
icon = "file-code"
enabled = true
menu = "submenu"

[when]
kinds = ["file"]
extensions = ["zip", "tar"]
min_items = 1

[run]
runtime = "bash"
entrypoint = "run.sh"
mode = "whole-selection"
on_error = "continue"
working_directory = "parent"
confirm = false
```

| Field | Meaning |
| --- | --- |
| `schema_version` | Must be `1`. A newer version is refused with an explanation. |
| `id` | Directory name: lowercase letters, digits, `.`, `-`, `_`. Cannot change later. |
| `name` | Menu label, 1–64 characters. |
| `description` | Optional tooltip. |
| `icon` | Bundled Lucide icon slug, for example `play`, `terminal`, `image`. |
| `enabled` | `false` keeps the action in Settings but out of menus. |
| `menu` | `submenu` (default) groups the action under **Actions**; `top` puts it in the menu body. |
| `[when].kinds` | `file`, `folder`, or both. Empty means either. |
| `[when].extensions` | Lowercase extensions without dots. Empty means any. |
| `[when].mime_types` | Guessed content types such as `image/*` or `image/png`. |
| `[when].min_items` / `max_items` | Selection size bounds. |
| `[run].runtime` | `python`, `bash`, or `command`. |
| `[run].entrypoint` | Script file name beside `action.toml` (Python and Bash only). |
| `[run].program` / `[run].args` | Program and arguments (command actions only). |
| `[run].mode` | `whole-selection` (default) invokes once with every path; `per-item` invokes once per path. |
| `[run].on_error` | `continue` (default) or `stop`, used by `per-item` runs. |
| `[run].working_directory` | `parent` (default), `home`, or `action`. |
| `[run].confirm` | Ask before each invocation. |

Validation is strict: unknown keys, unsupported versions, invalid ids, entrypoints
that are not a plain file name, and argument tokens that do not match the execution
mode are all refused with a message rather than being ignored.

## Matching

An action appears when **every** currently selected entry satisfies `[when]`, and
the selection size is within `min_items`/`max_items`. A mixed selection therefore
never runs an action on a subset the author did not intend to accept.

Opening the folder background menu offers the same rules with the folder itself as
the single input. Actions are only offered for native paths: Trash, GVfs, and other
non-native locations are excluded in this release.

## How an invocation runs

Nothing is ever passed through a shell. Programs and interpreters receive argv
entries directly, and every path is absolute, so a selected file name can never be
read as a command or as an option such as `-rf`.

| Environment variable | Contents |
| --- | --- |
| `STRATA_ACTION_VERSION` | Protocol version, currently `1`. |
| `STRATA_ACTION_ID`, `STRATA_ACTION_NAME` | Definition identity. |
| `STRATA_ACTION_MODE` | `per-item` or `whole-selection`. |
| `STRATA_ACTION_SOURCE` | `selection` or `background`. |
| `STRATA_ACTION_COUNT`, `STRATA_ACTION_POSITION` | Input count, and 1-based position for per-item runs. |
| `STRATA_ACTION_PATHS` | File containing every input path, NUL-delimited and byte-exact. |
| `STRATA_ACTION_PARENT` | File containing the invoking folder. |
| `STRATA_ACTION_PROGRESS` | File to append progress events to (see below). |
| `STRATA_ACTION_CONTEXT`, `STRATA_ACTION_RUN_DIR` | JSON context and private scratch directory, mode 0700. |
| `STRATA_ACTION_DIR` | The action's own directory. |

Paths travel in files rather than in the environment so unusual names — spaces,
newlines, and bytes that are not valid UTF-8 — survive exactly.

`whole-selection` runs the action once with every selected path. `per-item` runs it
once per path and tracks succeeded, failed, and remaining counts itself; with
`on_error = "stop"` the first failure ends the job.

### Command argument tokens

`args` entries are either literal text or exactly one of:

- `{path}` — the single input of a per-item run.
- `{paths}` — every input of a whole-selection run, as separate arguments.
- `{parent}` — the invoking folder.

A token written inside a larger string (`prefix-{path}`) is rejected, so nothing is
ever interpolated into a command line.

## Progress and results

Process exit status decides success: `0` means the invocation succeeded and any
other status means it failed. Reported percentages are presentational only.

A script may optionally report measurable progress by appending one JSON object per
line to `$STRATA_ACTION_PROGRESS`:

```json
{"event": "progress", "processed": 42, "total": 100, "message": "Converting images"}
{"event": "output", "path": "/home/you/Pictures/out.webp"}
```

- `processed` is required and bounded; `total` and `message` are optional.
- `output` records a created location: absolute paths only, which is why a relative
  path is ignored rather than guessed.
- Malformed or unknown lines are ignored, the file is read incrementally, and both
  the file and the number of events per invocation are bounded.

Scripts that never report progress still appear as **Running** with elapsed time
and an indeterminate indicator. stdout and stderr are captured as bounded output
for the Jobs dashboard; they are not the progress channel.

## Python helper

Python actions can use the bundled standard-library helper, which is importable
inside a run and needs no `pip install`:

```python
#!/usr/bin/env python3
from strata_actions import context

ctx = context()
for index, path in enumerate(ctx.paths, start=1):
    convert(path)
    ctx.progress(index, len(ctx.paths), "Converting images")
    ctx.output(f"{path}.webp")
```

`ctx.paths` are `str` values decoded with `surrogateescape`, so use
`ctx.paths_bytes()` when a tool needs the original bytes. `ctx.parent`,
`ctx.mode`, `ctx.source`, `ctx.position`, `ctx.total`, and `ctx.single` describe the
invocation; `ctx.log(...)` writes to captured output; `require_tool("ffmpeg")`
raises a clear error when a dependency is missing.

Any other language can follow the same contract: read the paths file, write the
progress file, and exit with a status.

## Jobs and cancellation

Each invocation runs in the background. The footer shows how many jobs are running
and queued, and opens a dashboard with progress, elapsed time, output, and the
finished history. Minimizing, pressing Escape, or clicking away only hides the
dashboard; cancellation is its own button and signals the whole process group,
escalating to SIGKILL if the process ignores SIGTERM.

Cancelling stops the work that has not happened yet. It does not roll back changes
already written to disk, and Strata does not retry a failed or cancelled action.

## Trust model

- Actions are trusted local programs. Out-of-process execution is not a filesystem
  or network boundary; a script can do anything you can do.
- Imported actions arrive **disabled**. Review the script before enabling it.
  Import copies only `action.toml` and its entrypoint.
- Strata never discovers or runs scripts found in browsed folders, and never
  installs runtimes, interpreters, or packages. A missing interpreter or program
  leaves the action visible but disabled, with the reason in its tooltip.
- Manifests and generated scripts are written owner-only (0600) inside an
  owner-only directory, and entrypoints are read as regular files: a symlinked
  manifest, entrypoint, action directory, or scratch directory is refused rather
  than followed.
- Captured output is bounded, and the dashboard renders it as text.

## Limitations

- Local native locations only; Trash, remote mounts, and the file chooser are out.
- `per-item` runs are sequential, and at most two jobs run at once.
- Whole-selection commands that cannot report progress show no percentage.
- There is no sandbox, no restart/resume, and no built-in parameter form; a script
  that needs options reads them from its own file or environment.
- Editing an action's script from Settings replaces the whole file; keep large
  scripts in version control if you edit them outside Strata.

## Testing an action

Enable it, then right-click a file that matches `[when]` and choose it. The Jobs
dashboard shows the invocation, its output, and its exit status. A definition that
cannot load appears under **Settings → Actions → Problems** with the reason, and
never silently disappears from the menus.
