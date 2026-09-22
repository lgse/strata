# Patched GTK for the test image

The Arch-based quality/E2E image pins GTK 4.22.5 and replaces only its
`libgtk-4.so.1.2200.5` with a build from the matching upstream source archive.
`gtk-4.22-accessible-bounds.patch` corrects the coordinate-space mismatch
tracked in [#1154](https://github.com/lgse/strata/issues/1154): widget bounds are
computed in the parent's content coordinates, while AT-SPI composes them from
the parent's border origin.

Source: <https://download.gnome.org/sources/gtk/4.22/gtk-4.22.5.tar.xz>

SHA-256: `7fd725deb2cb3f8dc218ad862c5056ff8548f49d3b0e4081796e444c22d19686`

The source and patch are LGPL-2.1-or-later. GTK's license text is retained by
the signed Arch package in the image and in
[`packaging/media-runtime/licenses/GTK-COPYING`](../../../packaging/media-runtime/licenses/GTK-COPYING).
`coordinate-probe.py` is MIT-licensed and runs while the image is built. It
checks the exact GTK version, loaded library path/checksum, and accessible bounds
against rendered bounds on a private X display and D-Bus session.

This patch is restricted to test images. It is not part of Strata's release
artifacts and does not apply or retire the independent media-runtime patches.
