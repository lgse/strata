# Unsafe Code Policy

Strata is a safe-Rust codebase by default. Unsafe code is an exception for a narrow platform boundary, not a general implementation tool.

## Requirements

Unsafe code may be introduced only when all of the following are true:

1. A required capability is unavailable through a suitable maintained safe API.
2. Avoiding it would remove an intentional product capability or create a worse operational compromise.
3. The unsafe operation is isolated behind a small safe interface.
4. The unsafe block is reduced to the exact FFI or pointer operation that requires it.
5. Every block has a `SAFETY:` comment describing the concrete invariants that make it sound.
6. The containing item uses `#[expect(unsafe_code, reason = "...")]` with a specific reason.
7. Error paths, null pointers, ownership, lifetimes, threading, and retained-pointer behavior are considered explicitly.
8. The change receives focused review and appropriate tests or runtime validation.

Do not use `#[allow(unsafe_code)]`. `#[expect]` is intentional: the compiler reports the attribute if the unsafe operation is later removed, preventing stale exceptions.

## Automated enforcement

`Cargo.toml` configures the compiler and Clippy to:

- Deny unsafe code globally
- Deny unsafe operations hidden inside unsafe functions
- Deny unnecessary unsafe blocks
- Deny `#[allow(...)]` attributes
- Require reasons on lint overrides
- Require `SAFETY:` documentation on every unsafe block
- Require safety documentation for public unsafe functions
- Limit each unsafe block to one unsafe operation

CI runs Clippy with warnings treated as errors, so violations block merges.

## Current inventory

### Bundled font registration

Location: `src/assets.rs::register_application_fonts`

Reason: Fontconfig exposes application-private font registration through its C API, and the available safe wrapper does not expose that capability. Strata uses three small FFI calls during single-threaded startup and presents the rest of the application with a safe function.

The operations are individually scoped and document:

- How the Fontconfig configuration pointer is obtained and null-checked
- The lifetime and ownership of the C path string
- Fontconfig's path-copy behavior
- Why rebuilding the font set occurs before GTK/Pango creates the application font map

If a maintained safe API gains this capability, this exception should be removed.

### Verified Trash result

Location: `src/adapters/local_operations/environment.rs::System::trash`

Reason: GLib can return FALSE from `g_file_trash` without setting an error
(GLib #4060). gio-rs asserts in debug builds that FALSE always carries an
error, so its safe `trash()` would panic there, and in release builds it
cannot tell that failure apart. Strata calls `g_file_trash` directly and treats
FALSE without an error as "not moved".

The file and cancellable outlive the call, and the error out-pointer starts
null. When GLib sets it, ownership transfers to Strata, which takes it with
`from_glib_full`; otherwise nothing is freed. The call runs only off the main
thread. Callers re-check the item's identity afterwards, so a TRUE that moved
nothing is caught too.

### File handles

Location: `src/adapters/local_operations/native_fs.rs::file_handle`

Reason: on NFS and ext2/3/4 without birth times, a freed inode number goes straight to the next
file, and only the file handle's generation tells the two apart. rustix does not wrap
`name_to_handle_at`, so Strata calls it through `libc` with a `struct file_handle` sized for
`MAX_HANDLE_SZ` (128 bytes), stating that size in `handle_bytes`, and a NUL-terminated name; the
call keeps no pointer. It keeps a 64-bit digest of the handle type and bytes, not the handle.

### Checked descriptor close

Location: `src/adapters/local_operations/fs_ops.rs::System::close`

Reason: `OwnedFd` and rustix discard close errors, but NFS, SMB and FUSE can report
failed writes only at close. The safe wrapper consumes the owned descriptor and
calls `libc::close` exactly once. Linux releases the descriptor even when close
reports `EINTR`; retrying could close a descriptor another thread has since reused.
The copy and archive regressions inject delayed close errors and require publication
to fail without deleting originals.

### Background close of enumerators and streams

Location: `src/adapters/gio_close.rs::Closable::close_in_background`

Reason: GLib closes an enumerator or stream still open at its last unref in place, so one
the UI thread lets go of would close there, waiting on a dead NFS or SMB server, a stopped
FUSE daemon or a GVfs backend. Strata starts the close with `g_file_enumerator_close_async`
or `g_input_stream_close_async`, which run it on GLib's worker pool or, for GVfs, without
blocking. gio-rs requires a Rust callback, which glib-rs binds to the thread that started the
close; a close that outlives a test would then abort the next test that dispatches the default
main context. Strata passes no callback, which GIO annotates as allowed.

The object is alive for the call, and GLib holds its own reference until the close ends. The
cancellable, callback and user data are null. The calls run only on the UI thread, and only
when no other operation is pending on the object; `Closing` rechecks pending objects until
then. Written streams are never closed this way, as their close reports lost data. A GVfs stream
whose open succeeded just as it was cancelled never reaches Strata: GLib frees it, and it closes
in place (see [Off the interface thread](filesystem-compatibility.md#off-the-interface-thread)).
The regression test closes a probe enumerator and stream let go after and during a call, and
requires each close to run off the UI thread.

### File and folder monitors made off the UI thread

Location: `src/adapters/local_files/monitors.rs::BareMonitor`

Reason: GLib's inotify backend creates, cancels and delivers every file and directory monitor
under one global lock, and creating one walks the watched path, which blocks on a mount that
stopped answering. Strata therefore creates and cancels every monitor (open folders, the Trash,
its own settings files, custom themes and the Omarchy theme folders) on one serial worker thread,
which `gio::FileMonitor` (not `Send`) cannot cross without a wrapper.

`BareMonitor` is the only value that moves a monitor between that worker and the UI thread, and
it never carries a signal handler: the worker creates the monitor with no thread-default main
context pushed, so its events reach the default main context, and sends it bare. That delivery
is read from GLib's and GVfs's source; for a GVfs folder it was not measured (see [What a dead
server does](filesystem-compatibility.md#what-a-dead-server-does)). The UI thread
connects the `changed` handler, whose closure owns `Rc` state. On release the UI thread first
disconnects that handler, removes the coalescing timeout and drops the state, and only then drops
the bare monitor, whose `Drop` hands it to the worker for `g_file_monitor_cancel` and the final
unref. So no closure, and no `Rc` it captured, can be freed off the UI thread; GObject reference
counts and the cancel are thread-safe. A monitor created after its request was dropped is sent back
to the worker the same way and never gets a handler. If the worker cannot take it, the monitor is
leaked rather than cancelled on the UI thread.

### Browser sandbox fork boundary

Location: `src/sandbox/browser/process.rs::{fork,namespaces}`

Reason: reusing a codec-free supervisor's process image avoids per-file helper
exec and namespace-tool startup while keeping native decoders disposable and
isolated per file.
Neither libc fork nor descriptor-sensitive namespace unshare has a suitable safe
API for this use. The two calls are individually scoped behind private interfaces.

Only the separate bubblewrap supervisor and its trusted setup child call them,
never the GTK application or an initialized decoder. Both reject anything other
than one task in `/proc/self/task`; no caller starts threads, owns shared mappings
or external locks, or initializes codec/RNG state before this boundary. libc fork
runs atfork handlers and resets libc's threading state. Namespace unshare uses
only user/mount/PID/IPC flags, never `CLONE_FILES`.

Children replace stdin/stdout and close inherited control descriptors before
setup. Results use a per-job write-only pipe directly to the application; the
supervisor never buffers media bytes or metadata, including previous thumbnails,
that a subsequent fork could expose. A second fork enters the new PID namespace;
its PID 1 installs a private proc mount, scratch mounts and input protections
before decoding. Children exit
through `exit_group`, without inherited atexit handlers or supervisor-buffer
flushes. The supervisor reaps the setup child, and the parent application's
absolute deadline tears down the entire bubblewrap namespace on failure.

Coverage includes rejecting multithreaded callers, descriptor ownership and
hostile framing, read-only source mutation attempts, decoder operation under the
policy, and real process reuse/replacement across browser views. Runtime checks
use private headless displays/buses where needed. Changes to supervisor
initialization must preserve these pre-fork invariants, not merely keep the
task-count check.
