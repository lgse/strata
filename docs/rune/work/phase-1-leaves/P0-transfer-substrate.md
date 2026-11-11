# P0 Transfer substrate

| | |
| --- | --- |
| Phase / lane | 1 · Transfer |
| Plan | P1, P2, P4 (protocols bound on winit's display, the way Rune's `platform/layer_shell.rs` does) |
| Depends on | S2 (approach), W3 (harness) |
| Unblocks | P1, P2, P4 |
| Owns | `crates/strata-platform/src/transfer/` (internal, enabled by `dnd`, `clipboard` and `foreign-parent`) |
| Branch | `feat/<issue>-p0-transfer-substrate` |

## Goal

Build once the plumbing that P1, P2 and P4 would each otherwise build: getting at winit's
Wayland display and X11 connection, and moving typed data between processes.

## Scope

- **Display access:** take winit's Wayland display and seat, or its X11 connection, from a
  Rune window. Bind globals on a queue of our own, and dispatch it from Rune's loop (or a
  thread), using the approach S2 chose.
- **Offers in:** a `wl_data_offer` or X11 selection that lists its MIME types and reads one
  type asynchronously, over a pipe or with INCR. Results arrive on Rune's loop through
  `AppHandle::update` or the setup `Waker`.
- **Sources out:** a `wl_data_source` or X11 selection owner with several MIME types and a
  lazy writer per type, so a large `text/uri-list` or `image/png` is produced only when asked
  for.
- **MIME helpers:** `text/uri-list` encoding and decoding (RFC 2483 escaping),
  `x-special/gnome-copied-files` and `application/x-kde-cutselection`. These are shared by P1
  and P2 and carry no Strata names.
- Public API: none yet. P1, P2 and P4 expose their own APIs on top.

## Tests

- Unit tests for the MIME encoders, with non-UTF-8 paths and `%` escapes.
- Under W3: offer three MIME types from the window and read each one back with the peer, on
  Wayland and on X11. Then the other way round.

## Done when

Both directions work under headless sway and Xwayland in CI, which runs Ubuntu 26.04
(libwayland 1.24): offers in and sources out on the data device, on data control (bound on
winit's display and on a connection of its own) and on X11, and drops into a Wayland window.
On a host with libwayland older than 1.23 (Ubuntu 24.04, unsupported since the owner's ruling
on S3's decision 1), winit's display is out of reach: the tests that need it check only that
it is reported Unsupported, and data control falls back to a connection of its own. Each feature compiles alone (`mise run compile-features`).
