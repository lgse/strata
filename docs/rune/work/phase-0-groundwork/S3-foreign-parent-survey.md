# S3 Foreign parent survey (spike)

| | |
| --- | --- |
| Phase / lane | 0 · Desktop |
| Plan | P4, §8 ("Test P4 on Hyprland, GNOME, KDE and sway in phase 0, not phase 6") |
| Depends on | — |
| Unblocks | P4 |
| Owns | `docs/decisions/S3-foreign-parent.md` |
| Branch | `docs/<issue>-s3-foreign-parent-survey` |

## Goal

Find out early whether a Rune window can be made transient for a foreign window on each
compositor. With no GTK fallback, a failure here blocks the release.

## Questions

1. Which compositors support `zxdg_importer_v2` (and `xdg_foreign` v1)? Does
   `set_parent_of` take effect for a winit toplevel?
2. On X11, does setting `WM_TRANSIENT_FOR` on a winit window keep it above its parent and
   place it there?
3. Does a request from Firefox and from a GTK app carry a usable `wayland:` or `x11:`
   handle on each compositor?
4. Where the import fails, what fallback is acceptable: centring, or a `xdg_activation`
   token only? Record it per compositor.

## Deliverable

A table of Hyprland, GNOME, KDE and sway, with Wayland and X11 results for each, the
fallback for each failure, and a repro for the Rune issue if the parent has to be set
inside Rune.

## Done when

P4 can be built against known behaviour, or the record names a blocker that the owner must
decide on.
