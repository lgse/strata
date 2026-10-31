# SPDX-License-Identifier: MIT
"""A separate GTK process offering real external file-list drags to Strata."""

import argparse

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gdk, Gio, Gtk

parser = argparse.ArgumentParser()
parser.add_argument("--move-only", action="store_true")
parser.add_argument("paths", nargs="+")
arguments = parser.parse_args()


class FileDragSource(Gtk.Application):
    def do_activate(self):
        window = Gtk.ApplicationWindow(application=self, title="Foreign file selection")
        window.set_default_size(240, 80)
        button = Gtk.Button(label="Foreign file selection")
        content = Gdk.ContentProvider.new_for_value(
            Gdk.FileList.new_from_array([Gio.File.new_for_path(path) for path in arguments.paths])
        )
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
