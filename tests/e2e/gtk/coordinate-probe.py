# SPDX-License-Identifier: MIT
"""Reject GTK builds that lose ancestor padding from accessible coordinates."""

import hashlib
import math
from pathlib import Path

import gi

gi.require_version("Gdk", "4.0")
gi.require_version("Gtk", "4.0")
from gi.repository import Gdk, GLib, Gtk  # noqa: E402

EXPECTED_VERSION = (4, 22, 5)
EXPECTED_LIBRARY = Path("/usr/lib/libgtk-4.so.1.2200.5")
CHECKSUM_FILE = Path("/usr/share/strata-e2e/gtk.sha256")
PADDING = 22


def loaded_gtk() -> Path:
    for line in Path("/proc/self/maps").read_text().splitlines():
        fields = line.split()
        if fields and "/libgtk-4.so." in fields[-1]:
            return Path(fields[-1]).resolve()
    raise RuntimeError("libgtk-4 is absent from the probe's process maps")


def inspect(window: Gtk.Window, box: Gtk.Box, entry: Gtk.Entry, loop: GLib.MainLoop) -> bool:
    entry_valid, x, y, width, height = Gtk.Accessible.get_bounds(entry)
    box_valid, box_x, box_y, _, _ = Gtk.Accessible.get_bounds(box)
    rendered_valid, rendered = entry.compute_bounds(window)
    accessible_origin = (x + box_x, y + box_y)
    rendered_origin = (math.floor(rendered.get_x()), math.floor(rendered.get_y()))
    library = loaded_gtk()
    checksum = hashlib.sha256(library.read_bytes()).hexdigest()
    recorded_checksum, recorded_path = CHECKSUM_FILE.read_text().split()

    assert library == EXPECTED_LIBRARY.resolve()
    assert Path(recorded_path) == EXPECTED_LIBRARY
    assert checksum == recorded_checksum
    print(
        f"GTK {Gtk.get_major_version()}.{Gtk.get_minor_version()}.{Gtk.get_micro_version()} "
        f"from {library} sha256={checksum}"
    )
    print(
        f"entry={entry_valid} {x},{y} {width}x{height}; "
        f"box={box_valid} {box_x},{box_y}; rendered={rendered_valid} {rendered_origin}"
    )

    assert entry_valid and box_valid and rendered_valid
    assert accessible_origin == rendered_origin == (PADDING, PADDING)
    window.destroy()
    loop.quit()
    return GLib.SOURCE_REMOVE


def main() -> None:
    version = (Gtk.get_major_version(), Gtk.get_minor_version(), Gtk.get_micro_version())
    assert version == EXPECTED_VERSION, f"expected GTK {EXPECTED_VERSION}, found {version}"

    css = Gtk.CssProvider()
    css.load_from_string(f".padded {{ padding: {PADDING}px; }}")
    Gtk.StyleContext.add_provider_for_display(
        Gdk.Display.get_default(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
    )
    window = Gtk.Window(default_width=500, default_height=300)
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
    box.add_css_class("padded")
    entry = Gtk.Entry()
    box.append(entry)
    window.set_child(box)
    loop = GLib.MainLoop()
    window.present()
    GLib.timeout_add(100, inspect, window, box, entry, loop)
    loop.run()


if __name__ == "__main__":
    main()
