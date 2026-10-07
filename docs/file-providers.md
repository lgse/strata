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
  "name": "Example Cloud",
  "command": ["/usr/bin/python3", "/home/alice/.config/strata/providers/example/provider.py"],
  "icons": {"available": "available.png", "working": "working.png"}
}
```

Save this as `example/provider.json`. The id must match the directory name.
The optional `name` supplies the human-readable menu group heading; without it,
Strata uses the id. Names must contain non-whitespace plain text, at most 64 UTF-8
bytes, without control characters. Existing registrations need no changes.
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
requested paths. An optional `priority` is 0 (normal, the default), 1 (warning), or
2 (error). The highest priority wins; equal priorities use the provider id in
lexical order, independent of discovery timing. Accessible descriptions include
all contributing providers, prefixed with their ids. Status badges do not use tooltips. Rows
are rebound to their actual current path; old responses cannot decorate a reused
row. Badges share the thumbnail/icon rendering used by Columns, List and Icons.

`menu` asks for actions for the **entire selection**, up to 200 items:

```json
{"version":1,"id":2,"method":"menu","paths":["/home/alice/cloud/report.txt"],"background":false}
{"version":1,"id":2,"actions":[{"id":"keep","label":"Keep offline","icon":"available"}]}
```

Return an empty `actions` array for ineligible selections. No placeholder or
disabled global menu item is inserted. Strata always keeps each provider's root
entries together in an inline section with a small, muted, non-interactive
heading. This is not an extra submenu: providers choose flat actions, recursive
submenus, or a mixture within their section:

```json
{"version":1,"id":2,"actions":[{"id":"share","label":"Copy share link","context":"opaque-selection-token"},{"id":"offline","label":"Offline availability","children":[{"id":"keep","label":"Keep offline","icon":"available","context":"opaque-selection-token"}]}]}
```

Only leaves activate; a branch's nonempty `children` array is navigation only.
Branches cannot have `context`. Across the entire tree, allow at most 64 nodes,
four levels (roots are level one), and unique ids of 1–64 ASCII letters, digits
or dashes. Labels are nonempty plain text, at most 128 UTF-8 bytes, without control
characters. Icons are optional declared icon ids. Leaf `context` is an optional
nonempty opaque string of at most 2,048 UTF-8 bytes. Action and submenu labels
remain as supplied, without repeated provider suffixes. The section heading
identifies their source using the registered name or id, including when different
providers supply identical labels. Empty sections and their headings disappear.
Menus populate asynchronously; retained provider sections and branches reuse
their models on refresh. Removed leaves are disabled before removal, including
when their containing section or branch is withdrawn or the menu closes.

Each menu and activation describes the whole selection, never a silently filtered
subset. An adapter may support cross-account selections; otherwise return no
actions. For parent/child selections, define whether a recursive operation covers
descendants and report counts against the original selection. Deduplication or
batching inside the adapter must preserve those semantics. Above 200 items Strata
offers no provider actions; it does not truncate a menu or activation selection.
Selections must also fit within a request frame: the JSON-escaped path array has
a budget of 1 MiB minus 16 KiB reserved for the envelope and context. Larger
selections receive no actions; decoration queries split into fitting batches.
Oversized host requests are rejected before admission and do not kill an adapter.

`activate` repeats the exact selection/background, selected leaf `action` id and
its `context`, if provided:

```json
{"version":1,"id":3,"method":"activate","paths":["/home/alice/cloud/report.txt"],"background":false,"action":"keep","context":"opaque-selection-token"}
{"version":1,"id":3,"message":"Downloads continue in the service.","outcome":{"status":"accepted","accepted":1,"total":1,"job":"job-17"}}
```

**Revalidate identity and eligibility on activation.** A menu is not authorization
to act on a replaced file, renamed path or new mount incarnation. Bind context to
the exact selection, account/collection/item identities and mount incarnation;
reject stale tokens. Enforce identity at the service's operation boundary, not
just during a prior path lookup. Strata echoes the token unchanged and replaces
it when the same leaf receives new context. A context can also be an adapter's
idempotency token; the host neither interprets it nor manufactures replay safety.
Adapters without tokens must still guarantee safe identity revalidation.

Every successful activation requires a nonempty plain-text `message` (at most
16 KiB). Messages and decoration descriptions may contain newlines, carriage
returns and tabs, but no other control characters, including NUL or terminal
escape sequences. Optional `outcome.status` is `accepted`, `rejected`, `partial`
or `unknown`.
`accepted` and `total` must appear together, with `total` equal to the original
selection size and `0 <= accepted <= total`. Accepted means all submitted;
rejected means none; partial means some but not all. Unknown may report a confirmed
accepted lower bound while other results remain uncertain. Optional `job` is a
nonempty service reference of at most 128 bytes without control characters.
The dialog shows source, status, supplied counts and job reference, and message.
Unknown counts are explicitly shown as a lower bound. Message-only legacy replies remain
supported, but do not imply confirmed acceptance or completion.

Long operations should be submitted as service jobs. There is no automatic retry
of actions, including after a lost reply. An unsent action is distinguished from
an action whose outcome is unknown; a disconnect after submission must not be
represented as proof that nothing happened. Partial acceptance is not a transaction
rollback. Check service state before a manual retry.

To mark cached answers dirty, emit an event at any time. An optional `paths` array
scopes it to up to 200 absolute native UTF-8 roots; omission means all paths:

```json
{"version":1,"event":"invalidate","paths":["/home/alice/cloud"],"revision":17}
```

Use a provider-session monotonic unsigned `revision` for reliable snapshot
ordering. A query/menu reply's revision identifies the coherent snapshot used for
that selection. It must cover every preceding relevant invalidation; a reply older
than such an event is rejected. Unrelated roots do not reject it. Invalidation
roots overlap selections in either ancestor/descendant direction, at component
boundaries; paths use lexical spelling, so adapters should consistently normalize
their native roots without resolving selected content or symlinks.

Strata preserves wire order across events and replies. A change emitted *after*
a valid snapshot marks it dirty for another refresh, while retaining presentation.
This guarantees progress for coherent snapshots under sustained activity, without
treating a reply older than a preceding relevant event as current. Once any revision
is used, all subsequent invalidations and successful query/menu replies in that
process session need revisions; invalidation revisions cannot decrease. A process
restart begins a new session and may restart its counter at zero.

Legacy unversioned events are refresh hints. Their query/menu replies must still
be coherent snapshots at emission time; Strata accepts them while retaining the
need to refresh if a hint crossed the request. Without revisions the host cannot
detect an adapter emitting an obsolete snapshot. New adapters should use revisions
and scoped invalidations rather than global hints for unrelated account activity.

Invalidate on mount disappearance, disconnect/reconnect or lost event history.
An empty method-specific answer withdraws presentation immediately. An event alone
does not erase a valid answer: it requests its replacement. Provider disconnects
withdraw everything. A five-second refresh interval is a fallback; an answer not
replaced expires after fifteen seconds. Identical answers do not rebuild menus
or clear badges, and open submenu models survive retained-branch refreshes.

## Compatibility

Unknown additive fields in manifests, requests and responses should be ignored;
known fields must retain their types and bounds. A response has exactly one of
`id` or `event`. Successful queries require `decorations`, menus require `actions`,
and activations require `message`. Legacy adapters may additionally include empty
`actions`/`decorations` arrays for another method. Missing required fields,
duplicate tree ids, unknown icons and malformed outcomes are protocol errors.
Providers must not send nonempty fields belonging to another method.

For an unsupported method or request, reply with the request id, a bounded slug
`error` such as `unsupported-method`, and an optional human-readable `message`;
omit `actions`, `decorations` and `outcome`. A method error withdraws that requested
presentation without killing the provider. Version 1 has a fixed baseline and no
handshake: hosts only send the three methods above. Do not require a startup reply
or a capabilities request. A future optional negotiation must preserve that baseline.

## Bounds and failure behavior

At most eight registrations are loaded (from at most 64 directory entries), with
one child per provider. Strata sends **one request at a time** on each connection;
serial adapters are supported, and events may arrive at any time. It queues up to
16 speculative query/menu requests and eight activations separately. User actions
have dispatch priority after the current request; pending refreshes for the same
visible path/menu are coalesced by the host. A full queue rejects admission, never
kills a healthy process. An action rejected at admission was not sent and is not
retried. Replies have 32 reserved queue slots; reading events continues while new
dispatch is paused for host backpressure. Events coalesce only between replies,
with independent per-root revisions. More than 200 roots or 64 KiB of root strings
in one event batch conservatively becomes a global invalidation.

Each response frame, excluding its newline, is limited to 1 MiB; adjacent valid
frames do not share that bound. Native paths are at most 16,384 UTF-8 bytes.
Requests time out eight seconds after dispatch, not admission; socket writes after
250 ms. Invalid frames, excessive output and timeouts disconnect the provider;
its process group is terminated and state is withdrawn, with a two-second restart
backoff. Actions in flight are reported as uncertain, unsent queued actions as
unsent; neither is replayed. Disconnect updates retire only requests from the
failed session; requests admitted afterward keep their outcome tracking across
restart. Ordinary on-demand/unrecognized files stay unbadged.

The GTK thread does not perform provider socket or subprocess I/O. It applies
completed bounded batches on its main loop. Each provider cache holds at most
2,048 path answers and 32 menu selections; at most 1,024 live thumbnail slots are
tracked. Only mapped slots are queried. Unmapped slots lose their decoration and
re-register on mapping; slots refused at capacity retry admission while mapped.

Strata logs bounded static failure categories and validated provider ids for rejected
registrations, spawn/transport failures, timeouts and malformed output. Repeated
transport diagnostics are rate-limited to once per ten seconds. Host saturation
is a separate busy/unsent condition, not an adapter protocol fault. Raw stdout,
stderr, command arguments, selected paths, context tokens and messages are never
included in these diagnostics.

This API does not authorize downloading file content to calculate presentation.
Use cached metadata/service status and never inspect selected documents merely
to draw a menu or badge. This is a provider obligation, not an enforced security
boundary: a trusted child has the user's permissions. Strata's existing
thumbnail/preview behavior is separate. Remote-only locations and non-UTF-8
filenames are unsupported; no lossy conversion is performed.

## Tests

`./scripts/test-headless.py file_providers` covers trusted registration, protocol
bounds, literal filenames, process framing, queue saturation, snapshot ordering,
recursive action retirement, tracking admission and window remapping.
`./scripts/e2e.sh tests/e2e/scenarios/test_file_providers.py` exercises live menu
refresh and withdrawal, sustained events, overlapping providers, mixed selections
and flat/nested leaf activation through the actual GUI.
The normal pinned quality and E2E checks remain required for this cross-cutting
change. Use private display/configuration environments; do not test by changing
the default file manager or loading fixtures into a user's installed application.
