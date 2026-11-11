# P2 Clipboard with custom MIME types

| | |
| --- | --- |
| Phase / lane | 2 · Transfer |
| Plan | §4 P2, `operations/clipboard/*`, `.../file-manager-formats` |
| Depends on | P0, W3 |
| Unblocks | X3, M4 |
| Owns | `crates/strata-platform/src/clipboard/` |
| Feature | `clipboard` |
| Branch | `feat/<issue>-p2-clipboard` |

## Goal

Copy, cut and paste with any MIME types, including image writes, keeping Rune's async read
shape.

## Scope

- Write an offer with several MIME types and lazy writers (P0): `text/uri-list`,
  `x-special/gnome-copied-files`, `application/x-kde-cutselection`, `text/plain` and
  `image/png`.
- Read: list the offered types, then read one, asynchronously.
- Ownership changes: tell the app when another client takes the clipboard. This drives
  Strata's "Files on clipboard" badge.
- Use `wl_data_control` where S2 found it, or `wl_data_device` with its focus limits
  documented, and X11 `CLIPBOARD` and `PRIMARY` selections.
- Module docs that link the Rune issue and say what Rune lacks.

## Tests

Under W3, with the peer: a round-trip of each format in both directions, a large image
(INCR on X11), ownership loss, on Wayland and X11.

## Done when

The peer tests pass in CI. Copying files from Nautilus and Dolphin and pasting them into the
example, and the other way round, is recorded in the PR.
