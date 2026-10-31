---
name: qa-drive
description: >
  Drive the real Strata window by hand in QA mode to verify your own UI or
  behaviour change: start a private session with ./scripts/qa-drive.sh, send
  `strata --qa` JSON commands one at a time, read each reply's widget state,
  focus, selection, menus, dialogs, preferences and files, check accessible
  names, and take and view screenshots. Use after changing UI, input handling,
  focus, menus, dialogs, preferences or file operations; to reproduce a bug;
  or to find targets and names before writing a QA scenario.
---

# Drive Strata in QA mode

`./scripts/qa-drive.sh` keeps one `strata --qa` session open inside the
pinned container: a private Xvfb (`:90`), a fresh HOME and XDG directories,
a fixture tree, and either a private session bus or a bus address that
cannot connect. You send one JSON command at a time; each `send` prints the
reply and returns in milliseconds. Input is real XTEST from inside the
process, and state is read from the widget tree by accessible name.

This is a hands-on check, not coverage. To make a check permanent, write a
scenario with the **qa-scenarios** skill; its `reference.md` lists every
command, target, and predicate.

## Rules

- Drive only through `./scripts/qa-drive.sh`. It runs inside the pinned
  container, never on the desktop display or the user's session bus. Never
  launch `strata --qa` on the host. A host run needs the owner's explicit
  consent for that task, is never validation evidence, and goes in the
  handoff.
- The scripts use Docker or Podman, whichever they select (Podman when it is
  installed); `STRATA_CONTAINER_ENGINE` overrides it.
- One session per checkout. It shares `./scripts/qa.sh`'s build cache, so
  `stop` it before running `qa.sh` or any other container script, and never
  run two container scripts at once.
- A session is not test evidence. Report what you saw; a scenario run through
  `./scripts/qa.sh` is the evidence.

## Start

```bash
./scripts/qa-drive.sh start                                   # canonical fixture, seeded prefs
./scripts/qa-drive.sh start --tree '{"notes":{"plan.txt":"plan\n"},"readme.md":"# Readme\n"}'
./scripts/qa-drive.sh start --tree @target/my-tree.json --pref browser_mode=list --pref show_hidden=true
./scripts/qa-drive.sh start --bus --env RUST_LOG=debug        # private session bus, extra variable
```

`start` builds the `qa` binary (the same build `qa.sh` makes), seeds HOME
from `tests/fixtures/harness.json`, writes the fixture, starts Xvfb, launches
the app, and prints the fixture path and the launch reply. It takes a few
seconds when nothing changed, longer after source edits, and many minutes
on a cold cache: give it a 10-minute timeout, or run it in the background
and wait for it.

- `--tree` is a manifest tree name, inline JSON, or `@file`: an object whose
  string values are file contents and whose object values are folders. All
  times are pinned to the manifest's `mtime`. The default `canonical` tree is
  `archive/`, `documents/{notes.txt,report.md,spreadsheet.csv}`,
  `pictures/{diagram.txt,photo.txt}`, `readme.md`, `todo.txt`, `.hidden.txt`.
- `--pref KEY=VALUE` overrides a seeded preference; the value is JSON, or
  else a string (`--pref text_size=large`, `--pref list_file_clicks=1`).
- The fixture directory is `/tmp/strata-qa-fixture-XXXXXX`. Its name is the
  root pane's name and the first `directory`. Get it with
  `./scripts/qa-drive.sh exec pwd`.
- `start` rebuilds the binary; `restart` does not. After changing source,
  `stop` and `start` again.

## Worked session

This session verifies that a file's context menu shows its items and
shortcuts, and that Escape closes it and returns focus to the row. The
outputs are real; `…` marks trimmed text.

```console
$ ./scripts/qa-drive.sh start --tree '{"notes":{"plan.txt":"plan\n","todo.txt":"todo\n"},"readme.md":"# Readme\n"}'
building strata --qa (log: target/qa-drive/build.log)
fixture: /tmp/strata-qa-fixture-TGVfgz
{"ok":true,"app":{"dialog":null,"directory":"strata-qa-fixture-TGVfgz","field":null,"focused":"notes","focused_pane":"strata-qa-fixture-TGVfgz","matches":["notes","readme.md"],"menu":null,"panes":[{"focused":"notes","name":"strata-qa-fixture-TGVfgz","selected":["notes"]}],"preview":null,"view_mode":"Columns"},"changed":[…],"criticals":[],"result":{"ready":true},"failures":[],"error":null}

$ ./scripts/qa-drive.sh send '{"select":{"entry":"notes"}}'
{"ok":true,"app":{…,"directory":"notes",…,"focused_pane":"notes","matches":["plan.txt","todo.txt"],…,"panes":[{"focused":null,"name":"strata-qa-fixture-TGVfgz","selected":["notes"]},{"focused":null,"name":"notes","selected":[]}],…},"changed":["directory","focused","focused_pane","matches","panes"],"criticals":[],"result":null,"failures":[],"error":null}

$ ./scripts/qa-drive.sh send '{"wait":{"entries":["plan.txt","todo.txt"],"dir":"notes"},"message":"notes never listed its files"}'
{"ok":true,…,"changed":[],"criticals":[],"result":null,"failures":[],"error":null}

$ ./scripts/qa-drive.sh send '{"select":{"entry":"plan.txt","dir":"notes"}}'
{"ok":true,"app":{…,"focused":"plan.txt",…,{"focused":"plan.txt","name":"notes","selected":["plan.txt"]}],…},"changed":["focused","panes"],…}

$ ./scripts/qa-drive.sh send '{"click":{"entry":"plan.txt","dir":"notes"},"button":3}' \
    '{"wait":{"menu_items":{"contains":["Rename","Copy"]}},"message":"the context menu never opened"}'
{"ok":true,"app":{…,"focused":null,"focused_pane":null,…,"menu":["New Folder with Selection","Open","Open With…","Quick preview","Print","Cut","Copy","Duplicate","Rename","Move to…","Copy to…","Compress…","Customize…","Copy path","Copy name","Properties","Move to Trash","Permanently delete"],…},"changed":["focused","focused_pane","menu","panes"],…}
{"ok":true,…,"changed":[],"criticals":[],"result":null,"failures":[],"error":null}

$ ./scripts/qa-drive.sh send '{"expect":[{"description":{"target":{"menu_item":"Copy"},"text":"Ctrl+C"}}],"message":"Copy shows its shortcut"}'
{"ok":true,…,"failures":[],"error":null}

$ ./scripts/qa-drive.sh screenshot menu.png
target/qa-drive/menu.png

$ ./scripts/qa-drive.sh send '{"key":"Escape"}' '{"wait":{"menu_items":null},"message":"the menu stayed open"}'
{"ok":true,"app":{…,"focused":"plan.txt","focused_pane":"notes",…,"menu":null,…},"changed":["focused","focused_pane","menu","panes"],…}
{"ok":true,…,"changed":[],…}

$ ./scripts/qa-drive.sh send '{"expect":[{"focused":"plan.txt"},{"state":{"target":{"entry":"plan.txt","dir":"notes"},"focused":true,"selected":true}}],"message":"focus returned to the row"}'
{"ok":true,…,"failures":[],"error":null}

$ ./scripts/qa-drive.sh screenshot after.png
target/qa-drive/after.png

$ ./scripts/qa-drive.sh stop
stopped strata-qa-drive-cb491b96
```

Then view `target/qa-drive/menu.png` with your image-capable file reader
(in Claude Code, `Read` with the absolute path). It showed the menu headed
`plan.txt` and its path, with the accelerators right-aligned. The reply
already said what the picture confirms: the menu opened, focus left the
pane while it was open, and Escape gave focus back to `plan.txt` in `notes`.

## Subcommands

| Command | Does |
| --- | --- |
| `send JSON...` | Sends each command in order and prints one reply line each. Exit 0 when a reply came back, whatever `ok` says; 1 when the app exited (with its log tail) or no reply came in 90 s (or the command's `timeout_ms` + 30 s), in which case that reply is skipped if it comes later; 2 for text that is not JSON. |
| `screenshot NAME.png` | Captures the whole Xvfb screen to `target/qa-drive/NAME.png`. |
| `restart [--pref K=V]... [--env N=V]...` | Closes the app's stdin, waits up to 10 s for it to quit, then signals its group; rewrites those keys in its saved `settings.toml`; relaunches on the same fixture and HOME and prints the launch reply. |
| `exec 'SHELL'` | Runs in the fixture directory with `$FIXTURE`, the session's `HOME`, `XDG_*_HOME`, `XDG_RUNTIME_DIR`, `DISPLAY`, and `DBUS_SESSION_BUS_ADDRESS`, so `gio` and similar tools see the session. |
| `log` | The application's log without colour codes. |
| `stop` | Quits the app and removes the container. Safe to repeat. |

Every exchange is appended to `target/qa-drive/transcript.jsonl` as
`{"command":…,"reply":…}`. `target/qa-drive/` also holds `input.log` (every
synthetic key and button event), `build.log`, the screenshots, and, after
`stop`, `strata.log`. `start` empties the directory.

## Read a reply

`{"ok","app","changed","criticals","result","failures","error"}`:

- `ok`: true when nothing failed and there was no error.
- `app`: the snapshot after the command: `view_mode`, `directory` (the
  current directory's name), `panes` (left to right, each with `name`, `selected`, and its
  `focused` entry), `focused` (the focused entry's name), `focused_pane`,
  `dialog` (its name), `menu` (the open menu's items), `preview` (the preview
  drawer's text), `field` (the focused text field's `name`, `text`, `caret`,
  `selection`, `focused`), and `matches` (the deepest pane's entries, or its
  search results).
- `changed`: the `app` keys that differ from the previous reply. Read it
  first to see what your input did.
- `criticals`: GLib warnings and criticals since the last reply. Any entry is a
  defect a scenario would fail on; read `log` and investigate.
- `result`: what a query, `holds`, or `note` returns; `null` for input.
- `failures`: one `{"predicate","actual","message"}` per mismatch, from
  `expect` or a timed-out `wait`:

  ```json
  {"ok":false,"failures":[{"predicate":{"focused":"todo.txt"},"actual":"archive","message":"deliberately wrong"}],"error":null}
  ```

- `error`: the command could not run: it did not parse, a target was not
  found (`no entry "missing.txt" on screen after 15 s`), or a `wait` timed out
  (`timed out after 1000 ms`). A timed-out `wait` also puts the widget tree in
  `result.tree`.

Pipe through `jq` to cut a reply down:
`./scripts/qa-drive.sh send '{"query":"state"}' | jq -c '.app.panes'`.

## Command cheat sheet

Input:

| Command | Meaning |
| --- | --- |
| `{"select":{"entry":"todo.txt","dir":"archive"}}` | Click until the entry is selected (no `dir`: any pane). |
| `{"click":TARGET,"button":3}` | Right-click; `"count":2` double-clicks; `"mods":["ctrl"]`; `"at":[4,"50%"]` aims from the top-left. |
| `{"move":TARGET}` | Hover. |
| `{"wheel":TARGET,"clicks":3,"direction":"down"}` | Scroll. |
| `{"drag":{"from":TARGET,"to":TARGET}}` | Drag and drop; `"mods":["ctrl"]`, `"release":false`. |
| `{"key":"ctrl+period"}`, `{"key":"Down","repeat":2}` | Tap a chord. Keys are X keysym names (`Return`, `Escape`, `F2`, `Delete`, `Tab`, `period`) or one character, at US key positions; modifiers are `ctrl`, `shift`, `alt`, `altgr`. |
| `{"type":"notes"}` | Type text, one key per character. |
| `{"key_down":"ctrl"}`, `{"key_up":"ctrl"}`, `{"button_down":1}`, `{"button_up":1}` | Hold and release. |
| `{"activate":TARGET}` | The widget's default action, without the pointer. |
| `{"resize":{"width":420,"height":300}}` | Resize the main window. |

Checks and queries:

| Command | Meaning |
| --- | --- |
| `{"wait":PREDICATE,"message":"..."}` | Poll until it holds; `timeout_ms` defaults to 15000. Use after every input. |
| `{"expect":[PREDICATE,...],"message":"..."}` | Check now and report every mismatch in `failures`. |
| `{"holds":PREDICATE}` | `true` or `false` in `result`, never a failure. |
| `{"query":"state"}`, `{"query":"prefs"}`, `{"query":"tree"}` | The snapshot, the saved `settings.toml` as JSON, or the widget tree as text. |

Targets: `{"entry":"a.txt","dir":"notes"}`, `{"pane":"notes"}`,
`{"menu_item":"Copy"}`, `{"dialog_button":"Cancel"}`, `{"dialog":true}`,
`{"sidebar":"Home"}`, `{"role":"button","name":"Settings"}` (add
`"within":TARGET`, `"nth":-1`, or `"states":["checked"]`),
`{"focused_widget":true}`, `{"field":true}`, `{"toplevel":"Copy to"}`.

Predicates: `{"entries":NAMES,"dir":...}`, `{"selected":NAMES,"dir":...}`,
`{"focused":"a.txt"}`, `{"focused_pane":"notes"}`, `{"directory":"notes"}`,
`{"menu_items":NAMES}` (`{}` any open menu, `null` none), `{"dialog":"Name"}`
(`true` any, `null` none), `{"view_mode":"List"}`,
`{"field":{"name":"Rename","text":"archive","selection":[0,7]}}`,
`{"pref":{"key":"show_hidden","value":true}}`,
`{"file":{"path":"/tmp/strata-qa-fixture-…/todo.txt","exists":false}}`,
`{"present":TARGET}`, `{"absent":TARGET}`, `{"state":{"target":TARGET,"focused":true}}`,
`{"description":{"target":TARGET,"text":"Ctrl+C"}}`,
`{"name":{"target":TARGET,"text":"..."}}`, `{"text":{"target":TARGET,"text":"..."}}`,
`{"focused_widget":"archive"}`, `{"focused_widget_is":TARGET}`,
`{"focus_within":TARGET}`, `{"editable_fields":0}`, and `{"any":[...]}`,
`{"all":[...]}`, `{"not":...}`. `NAMES` is an exact list in visual order or
`{"contains":[...],"excludes":[...],"count":4}`.

Runner-only steps (`fs`, `layout`, `restart`, `screenshot`, `log`, `proc`,
`dbus_send`, and the rest) are not process commands; the app answers them
with an `error`. Use the subcommands instead, or write a scenario.

## Check accessible names and focus

The app reads its own widget tree with AT-SPI off (`GTK_A11Y=none`); names
are what `src/ui/accessibility.rs` sets, as a screen reader would hear them.

1. Print the tree:
   `./scripts/qa-drive.sh send '{"query":"tree"}' | jq -r .result > target/qa-drive/tree.txt`.
   Its first line is the focus (`focus: GtkModelButton role="menu item" name="New Folder with Selection"`);
   every widget then shows its role, `name`, `description`, screen box, and
   `[selected]` or `[focused]`:

   ```text
   GtkModelButton role="menu item" name="Copy" description="Ctrl+C" (29,269 294x26)
   StrataSidebarToggle role="toggle button" name="Toggle sidebar (Ctrl+B)" (5,6 24x24)
   ```

2. Assert what matters: `{"name":…}`, `{"description":…}`, or `present` with
   the role and name. An icon-only button must have a name; a tooltip never
   counts as one.
3. Check focus after every interaction that moves it (menus, dialogs, rename,
   Escape, close): `{"focused":…}` for an entry,
   `{"focused_widget":"name"}` or `{"focused_widget_is":TARGET}` for any
   widget, `{"state":{"target":…,"focused":true}}` for one widget, and the
   tree's `focus:` line when nothing seems focused. Focus that lands on an
   unnamed widget (`"actual":""`) is worth reporting.

## Simulate input

- Find a shortcut's binding in the `ui::keys::bindings` catalog and send the
  chord: `{"key":"ctrl+period"}` toggled hidden files in a test session.
- Keys are US key positions under the session's US layout. Layout-dependent
  behaviour needs a scenario with a `layout` step.
- Prefer `select` over `click` to select an entry; it retries until the
  selection takes.
- Aim with targets, never screen coordinates. `"at"` offsets from a target's
  top-left corner (`[4,"50%"]`, `["end-6","50%"]`).
- Use `activate` when you need the action without pointer side effects; use
  `click` when the pointer path is what you changed.

## Preferences and settings

- Start with a preference: `start --pref browser_mode=icons`.
- Change one and relaunch: `restart --pref text_size=large`. The app quits
  first, so the new value is not overwritten by its own save.
- Read what the app saved: `{"query":"prefs"}`, or
  `exec 'cat "$XDG_CONFIG_HOME/strata/settings.toml"'`.
- Verify a live change through the UI the user would use, then wait on both
  the effect and the saved value:

  ```bash
  ./scripts/qa-drive.sh send '{"key":"ctrl+period"}' \
    '{"wait":{"entries":{"contains":[".hidden.txt"]}},"message":"Ctrl+. to show hidden files"}' \
    '{"wait":{"pref":{"key":"show_hidden","value":true}},"message":"show_hidden saved"}'
  ```

- Different GTK settings, keyboard layouts, or a fixed fixture path need a
  scenario header; `start` cannot set them.

## Files

- Inspect or change the fixture: `exec 'ls -A; cat todo.txt'`,
  `exec 'echo new > fresh.txt'`, then
  `send '{"wait":{"entries":{"contains":["fresh.txt"]}},"message":"fresh.txt listed"}'`.
- Check a result on disk from inside the app with the `file` predicate, or
  with `exec` after a `wait` on the UI state that signals the operation ended.
- The session's trash is `$XDG_DATA_HOME/Trash`.

## Pitfalls

- **Wait on state, not time.** A reply comes once the main loop drains, but
  directory loads, file monitors, dialogs, and saves land later. After every
  input, `wait` on what the user would see change. Never `sleep` to make a
  check pass.
- **`expect` does not wait.** Use it right after a `wait`, or give it
  `"timeout_ms"`.
- **Xvfb is slow and software-rendered.** It is a debug build on `llvmpipe`.
  A target lookup waits up to 15 s, so a wrong name costs 15 s before its
  error. The window is 1200x760 on a 1440x900 screen; the black margin in
  screenshots is normal.
- **Rebuilds.** `restart` reruns the binary already built. After an edit,
  `stop` then `start`.
- **A stuck session.** If `send` reports no reply, read `log`, then `restart`,
  or `stop` and `start`. If `start` was interrupted, `stop` clears the
  leftover container.
- **Never run container scripts concurrently.** `stop` before `./scripts/qa.sh`
  or `./scripts/quality.sh`.
- **Quote JSON for the shell.** Wrap each command in single quotes; splice a
  variable with `'"$name"'`. A JSON line is one command; do not pretty-print it.
- **Snapshot names are names.** `directory`, `panes[].name`, and
  `focused_pane` are directory names, not paths.

## Turn a session into a scenario

When the check guards behaviour worth keeping, write it as a scenario with the
**qa-scenarios** skill. The transcript holds the steps:

```bash
jq -c '.command | select(has("query") | not)' target/qa-drive/transcript.jsonl
```

Each process command is already a valid step. Then:

- Put `--tree` into the header's `tree` and `--pref` into its `prefs`.
- Replace the fixture name with `$root` and its path with `$fixture`.
- Turn `exec` file changes into `fs` steps, `restart` into `{"restart":true}`,
  and `screenshot` into `{"screenshot":"name.png"}`.
- Give every `wait` and `expect` a `message`, and drop exploratory queries.
- Look for an existing scenario that owns the behaviour before adding a file,
  and follow that skill to run it and prove it can fail.

## Clean up

```bash
./scripts/qa-drive.sh stop
docker ps -a --filter name=strata-qa-drive      # nothing listed (or podman ps -a)
```

`stop` removes the container and everything in its `/tmp`: Xvfb, the bus,
HOME, and the fixture. `target/qa-drive/` stays until the next `start`; it is
ignored by Git. Never commit screenshots or transcripts; attach evidence to
the PR as the repository instructions say.
