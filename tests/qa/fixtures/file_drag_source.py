# SPDX-License-Identifier: MIT
"""A separate GTK process offering real external drags to Strata.

Paths are offered as a file list; each `--offer MIME PATH` offers that file's
bytes under the MIME type as they are, the way a browser offers content.
"""

import argparse

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gdk, Gio, GLib, Gtk

parser = argparse.ArgumentParser()
parser.add_argument("--move-only", action="store_true")
parser.add_argument("--offer", nargs=2, action="append", default=[], metavar=("MIME", "PATH"))
parser.add_argument("paths", nargs="*")
arguments = parser.parse_args()
if not arguments.paths and not arguments.offer:
    parser.error("offer paths or content")


def offered_content():
    providers = []
    if arguments.paths:
        providers.append(Gdk.ContentProvider.new_for_value(
            Gdk.FileList.new_from_array([Gio.File.new_for_path(path) for path in arguments.paths])
        ))
    for mime, path in arguments.offer:
        with open(path, "rb") as offered:
            providers.append(Gdk.ContentProvider.new_for_bytes(mime, GLib.Bytes.new(offered.read())))
    return providers[0] if len(providers) == 1 else Gdk.ContentProvider.new_union(providers)


class FileDragSource(Gtk.Application):
    def do_activate(self):
        window = Gtk.ApplicationWindow(application=self, title="Foreign file selection")
        window.set_default_size(240, 80)
        button = Gtk.Button(label="Foreign file selection")
        content = offered_content()
        actions = Gdk.DragAction.MOVE if arguments.move_only else Gdk.DragAction.COPY
        drag = Gtk.DragSource(actions=actions)
        drag.connect("prepare", lambda *_: content)
        drag.connect("drag-end", lambda _, __, delete_data: button.set_label(
            "Foreign drag requested deletion" if delete_data else "Foreign drag finished"
        ))
        button.add_controller(drag)
        window.set_child(button)
        window.present()


FileDragSource(application_id="io.github.lgse.Strata.TestDragSource").run([])
