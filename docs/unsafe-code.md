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
