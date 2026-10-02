# External file providers (protocol 1)

File providers add state-dependent context actions and file decorations without
loading provider code into Strata. Examples include cloud availability and
version-control status. Providers are opt-in trusted programs, **not sandboxes**;
they run with the user's permissions and receive visible native paths. Only
install a provider from a source you trust.

Static custom actions continue to use [custom-actions.md](custom-actions.md).
They do not change. A file provider is appropriate when eligibility and state
come from an external service and must update while a directory/menu stays open.
The browser does not need to know about cloud accounts, pins or service sockets.

## Registration and artwork

Install one directory per provider under `$XDG_CONFIG_HOME/strata/providers/`
(default `~/.config/strata/providers/`), then restart Strata. For example:

```json
{
  "version": 1,
  "id": "example",
  "command": ["/usr/bin/python3", "/home/alice/.config/strata/providers/example/provider.py"],
  "icons": {"available": "available.png", "working": "working.png"}
}
```

Save this as `example/provider.json`. The id must match the directory name.
Commands are literal argv with an absolute executable; no shell interpolation,
PATH lookup, placeholder expansion or working-directory executable lookup is
performed. The child runs with `/` as its working directory. Nothing in a browsed
folder registers a provider. Removing the registration and restarting Strata
uninstalls it. Editing a registration takes effect on restart.

Registration directories/manifests and icon files must be owned by the current
user or root, must not be group/world writable, and cannot be symlinks. Executable
symlinks are resolved for the executable permission check. Providers can reference
only declared icon ids in replies. PNG artwork lives beside the manifest (plain
filenames, no subdirectories), at most 64 KiB and 256×256 per image, up to 16 icons.
It is loaded once from trusted configuration, never from selected file content or
URLs. Branded artwork preserves its colors; use artwork legible on both light and
dark backgrounds. Strata's own interface continues to use themed Lucide icons.

## Transport

The child reads newline-delimited UTF-8 JSON requests on stdin and writes JSON
responses/events on stdout. Flush each line. Stdin/stdout are a private full-duplex
socket, not a terminal. Protocol data only belongs on stdout. Provider stderr is
discarded, so do not rely on it for user-facing failures or print secrets there.

Every request has `version: 1`, a unique numeric `id`, `method`, `paths` (absolute
UTF-8 strings), and `background` (boolean). A background menu has the current
folder as its single path. Non-native and non-UTF-8 paths are not sent. Newlines,
quotes, spaces and Unicode in filenames are JSON data, not delimiters or commands.

`query` asks for decorations for up to 200 paths:

```json
{"version":1,"id":1,"method":"query","paths":["/home/alice/cloud/report.txt"],"background":false}
{"version":1,"id":1,"decorations":[{"path":"/home/alice/cloud/report.txt","badge":"available","description":"Available offline"}]}
```

Omit a path or use `badge: null` to draw no decoration. Replies can only affect
requested paths. Descriptions become accessible descriptions; status badges do not use tooltips. Rows
are rebound to their actual current path; old responses cannot decorate a reused
row. Badges share the thumbnail/icon rendering used by Columns, List and Icons.

`menu` asks for actions for the **entire selection**, up to 200 items:

```json
{"version":1,"id":2,"method":"menu","paths":["/home/alice/cloud/report.txt"],"background":false}
{"version":1,"id":2,"actions":[{"id":"keep","label":"Keep offline","icon":"available"}]}
```

Return an empty `actions` array for ineligible or mixed selections. No placeholder
or disabled global menu item is inserted. Up to 16 actions, with ids containing
letters, digits and dashes, are allowed. Icons may be null. Labels are plain text.
Menus populate asynchronously, including after they have opened.

`activate` repeats the exact selection/background plus the selected `action` id:

```json
{"version":1,"id":3,"method":"activate","paths":["/home/alice/cloud/report.txt"],"background":false,"action":"keep"}
{"version":1,"id":3,"message":"The request was accepted. Downloads continue in the service."}
```

**Revalidate eligibility on activation.** A menu is not authorization to act on a
stale object or mount. Return a plain-text outcome (at most 16 KiB); Strata shows
it in a dialog. Long operations should be submitted as service jobs, not performed
inside the request. There is no automatic retry of actions, including after a lost
reply; providers must distinguish acceptance from completion and report partial
batch acceptance honestly.

To invalidate all cached answers, emit an event at any time:

```json
{"version":1,"event":"invalidate"}
```

Invalidate on service state changes, mount disappearance, disconnect/reconnect or
lost event history. Strata advances a generation, discards older in-flight query
and menu responses, and refreshes visible items and open menus. Events carry no
paths. A five-second refresh interval is a fallback, not an event replacement. Refreshes
retain the last answer while awaiting its replacement; identical answers do not
rebuild menus or clear badges. Changed menu rows are reconciled in place. Negative
answers and provider disconnects withdraw presentation; unanswered state expires
after at most fifteen seconds even if events keep invalidating it.

## Bounds and failure behavior

At most eight registrations are loaded (from at most 64 directory entries), with
one child and bounded request/update queues per provider. Each response line is
limited to 1 MiB. Requests time out after eight seconds, socket writes after
250 ms. Invalid frames, excessive output and timeouts disconnect the provider;
its process group is terminated and state is withdrawn, with a two-second restart
backoff. A permanently closed worker (for example after exhausting its output
queue) withdraws all state until Strata restarts. A full request queue reports an
unsent action rather than silently accepting it. Ordinary on-demand/unrecognized files stay unbadged.

The GTK thread does not perform provider socket or subprocess I/O. It applies
completed bounded batches on its main loop. Each provider cache holds at most
2,048 path answers and 32 menu selections; at most 1,024 live thumbnail slots are
tracked. Only mapped slots are queried. Multiple providers may contribute menu
actions; the first registered provider with a badge wins for a given path.

This API does not authorize downloading file content to calculate presentation.
Use cached metadata/service status and never inspect selected documents merely
to draw a menu or badge. Strata's existing thumbnail/preview behavior is separate.

## Tests

`./scripts/test-headless.py services::file_providers::tests` covers trusted
registration, protocol bounds, literal filenames and process framing.
`./scripts/e2e.sh tests/e2e/scenarios/test_file_providers.py` exercises live menu
refresh and withdrawal, mixed selections, and activation through the actual GUI.
The normal pinned quality and E2E checks remain required for this cross-cutting
change. Use private display/configuration environments; do not test by changing
the default file manager or loading fixtures into a user's installed application.
