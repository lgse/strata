# Custom actions

Custom actions add your own scripts to the file and folder context menus. In
[10xer mode](10xer-mode.md), **;** then **1**–**9** / **0** runs the first
ten matching actions on the focused item or filled selection.

They are ordinary programs running with your permissions. There is no sandbox and
no install step beyond enabling an action. Strata validates the definition and the
files around it, prepares private invocation files, and reports progress and
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
  keeps each script draft until you close the dialog. For Python and Bash, a runtime
  indicator checks the declared shebang (or default interpreter) against the same executable lookup
  used when loading actions. Availability is checked automatically when the
  dialog opens; reopen it after installing a runtime.
  Bash/Command do not require Python. This does not execute anything or validate
  script syntax, dependencies, or behavior; missing runtimes do not prevent saving
  a draft.
- **Behavior**: execution mode, per-item failure policy, working folder, menu
  placement, file/folder filters, selection limit, and confirmation.

**Create action** saves all three tabs together. Invalid fields bring you back to
that tab without discarding your edits. Clicking outside the dialog leaves it open;
**Cancel**, the close button, or **Escape** discards the draft.
Existing actions use the same editor with **Save changes**; their id stays fixed.
Creating or duplicating an action never replaces an existing action directory.
Manifest-only MIME filters and minimum selection counts survive editor saves.
Command arguments preserve literal whitespace and empty lines; unchanged arguments
containing embedded newlines are also retained.

### Script examples

Open **Library** on the Script tab to browse bundled Python and Bash recipes in a
dropdown attached to the editor. Search names, descriptions, or requirements,
and narrow the list with **All**, **Files**, or **Media**. Each template shows its
language, purpose, run mode/input scope, and required tools:

| Example | Extra requirements | Result |
| --- | --- | --- |
| Log selected paths | None | Log paths and report progress without changing files |
| Convert images to WebP | ImageMagick (`magick` or `convert`) | First frame, quality 85, in `strata-webp-*/converted.webp` |
| Resize images to 1024px | ImageMagick (`magick` or `convert`) | First frame, aspect-preserving PNG, never enlarged, in `strata-resize-*/resized.png` |
| Convert videos to MP4 | FFmpeg with `libx264` and AAC | H.264/AAC copy in `strata-mp4-*/converted.mp4` |
| Extract MP3 audio | FFmpeg with `libmp3lame` | First audio stream in `strata-mp3-*/audio.mp3`; fails if there is no audio |
| SHA-256 checksums | None | Adjacent `<original-name>.sha256`, compatible with `sha256sum --check` from the original folder |
| Batch rename | Python 3 + Linux `renameat2` support | Rename regular files in place with a numbered pattern; never overwrite another name |
| Strip EXIF metadata | Bash + ExifTool + coreutils | Remove writable metadata in place after copying the original into a fresh adjacent `strata-original-*` folder |
| Lowercase file names | Python 3 + Linux `renameat2` support | Lowercase names and replace spaces with dashes, using the same collision-safe engine as Batch rename |
| Count lines | Bash + `wc` (coreutils) | Log each file's newline count and the total, without changing files |

Python recipes need Python 3 but no pip packages; Bash recipes do not need
Python. Strata does not install tools.
Conversion outputs go into fresh, private folders **beside each original**;
conversions never overwrite originals. EXIF stripping instead edits the working
photo after making a private backup, as described below. Existing checksum files
and symlinks are refused, not
replaced. A failed or cancelled conversion may leave a partial output folder.
Review scripts and use trusted input files: media tools and actions are not
sandboxed.

Applying a recipe selects its runtime and sets suitable file filters and run mode.
Conversions, checksums, and EXIF stripping select **Per item**; the starter,
rename/lowercase, and line-count recipes select **Whole selection**, even if the
previous draft used the opposite mode. It keeps a name or description you
entered, the id, and other settings. Choose a template to load it into the
editor for review. If you have edited the target runtime's draft, the dropdown first
asks you to **Replace script** or **Keep draft**, without opening another dialog.
Code replacement can be undone in the editor; drafts for the other runtimes are retained.
Escape or clicking outside closes only the dropdown, leaving the editor open.
Searching, browsing, or applying a template never saves or executes it. Review
and save the action, then launch it from its file/folder context menu or, in
10xer mode, **;** then a digit.

The documented Python starter and every Python recipe include the same maintained
[`context()` reference](../data/actions/context-api.txt) as a module docstring.
Examples are starting points: edit the output format, quality, resize limit,
FFmpeg flags, or filters before saving. Shared rename-recipe code is embedded
into the inserted script, so saved recipes do not depend on files in Strata's
source checkout.

**EXIF stripping:** only regular, non-symlink photos are accepted. Each original
is copied completely into a new private folder before ExifTool replaces the
working photo; repeated runs keep separate backups. Failed or cancelled copies
may leave partial backup folders, and cancellation does not restore earlier
changes. Removing writable metadata can remove orientation/color profiles and
is not a guarantee that every format is anonymized. Review results before
removing backups.

**Count lines:** `wc -l` counts newline characters; an unterminated final line is
not counted. Whole-selection mode gives a total across the supplied selection.

### Batch rename

The **Batch rename** recipe defaults to `PATTERN = "{index:03d}_{filename}"`,
producing names such as `001_photo.jpg`. Edit the pattern, or replace
`new_name(context)` with your own Python logic returning a single filename.
Its per-file context provides:

- `filename`: original name including its extension.
- `stem` and `suffix`: name without its last extension, and that extension with
  its leading dot (empty if absent).
- `index`: **1-based** position in Strata's supplied selection order, not click
  order; `total`: number of files in the job.
- `path`: original absolute `pathlib.Path`.
- `batch`: the full Strata `context()` object documented below, including logging
  and progress methods. These per-file fields belong to this recipe, not the
  general helper.

Patterns accept `filename`, `stem`, `suffix`, `index`, and `total`. For example,
`"photo_{index:04d}{suffix}"` produces `photo_0001.jpg`. The default whole-selection
mode checks the entire plan before renaming anything. Changing to per-item mode
keeps the job-wide index but can only preflight one file at a time.

Names cannot contain directory separators or be empty, `.` or `..`. Duplicate
inputs/destinations, existing destinations (including symlinks), folders, and
symlink inputs are refused. Unchanged names are skipped; swaps into other selected
names are not supported. Every move uses Linux's atomic no-replace operation,
including if another process creates the destination after preflight. Missing
support fails rather than falling back to an overwriting rename. Some filesystems
may refuse case-only changes. **Lowercase file names** uses the same engine and
safety rules, with an editable `new_name(context)` transformation instead of a
numbering pattern.

File contents are unchanged, but original **names** change. This is not an atomic
batch or an undo feature: cancellation, concurrent filesystem changes, or an I/O
error can leave earlier files renamed. Review the script and try it on copies first.

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
| `[run].confirm` | Ask once before queuing the entire job. |

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

Custom actions occupy a separate section between the opening/printing commands
and Cut/Copy. The context menu uses GTK's native nested menu model: hover over
**Actions** to open its submenu, then move onto an action to highlight it.
Hovering another menu item closes the submenu. Clicking or keyboard activation
also opens it; outside-click or Escape dismisses the menu. Long filenames and
paths use middle ellipsis, preserving their beginning and extension.

## How an invocation runs

Nothing is ever passed through a shell. Programs and interpreters receive argv
entries directly, and every path is absolute, so a selected file name can never be
read as a command or as an option such as `-rf`.

| Environment variable | Contents |
| --- | --- |
| `STRATA_ACTION_VERSION` | Protocol version, currently `1`. |
| `STRATA_ACTION_ID` | Definition identity; the display name is in context JSON. |
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
`ctx.paths_bytes()` when a tool needs the original bytes. `ctx.single` returns
the sole path for a one-item invocation, or `None`. `ctx.count` counts inputs in
this invocation, not the entire per-item job.

Other properties are `parent` (invoking folder, not necessarily the working
directory), `directory` (action directory), `run_directory` (private scratch,
removed after the invocation), `action_id`, `version`, `metadata` (raw JSON),
`mode`, `source`, and the optional 1-based per-item `position` and `total`.
`ctx.log(message)` writes to captured stdout. Bytes that are not Unicode
are escaped, so logging a native path still succeeds when the locale uses a
strict UTF-8 stdout. `ctx.progress(processed,
total=None, message=None)` reports invocation-local work; `ctx.output(path)`
reports an absolute output path, but does not create a file. Output reporting is
best-effort: names containing non-UTF-8 bytes cannot be represented by the JSON
progress channel and produce a warning rather than failing the action.

Import `find_tool(name)` to look up an executable (returns its path or `None`), or
`require_tool(name)` to raise `ContextError` when a dependency is missing.

Any other language can follow the same contract: read the paths file, write the
progress file, and exit with a status.

## Jobs and cancellation

Each invocation runs in the background. Queuing an action automatically opens
Jobs in the **launching window**, with that new job first and its queued/running
state, progress, elapsed time, and output. Confirmation actions open Jobs only
after you choose **Run**. This presentation does not change queue execution order
or open dashboards in other windows. The footer shows running/queued counts and
reopens finished history when clicked.

Minimizing, pressing Escape, or clicking away only hides the dashboard. Progress
and completion updates do not reopen it; launching another action does.
Cancellation is its own button and signals the whole process group, escalating
to SIGKILL if the process ignores SIGTERM. Closing the last application window
is blocked while jobs are queued, running, or cancelling; finish or cancel them
in Jobs first. Other windows may close without interrupting work.

An invocation ends when its direct process exits. Output capture and scratch
files are then closed without waiting for background descendants. Scripts should
wait for subprocesses that need the invocation's context or captured output.

Finished jobs stay in the dashboard for the current session, including successes,
failures, and cancellations. **Details** expands captured output and **Hide**
collapses it. The header summarizes running, queued, and finished jobs. Rows use
the action's icon and theme-accent status, with elapsed time and a check or X for
finished results. Failure output appears only in Details, not a separate banner.
The dashboard opens above the footer without covering it.
Controls retain tooltips and accessible names. **Minimize** keeps history;
**Dismiss** removes one finished job,
and **Clear finished** removes finished history without disturbing active work.

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
