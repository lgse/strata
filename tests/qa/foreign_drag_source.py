# SPDX-License-Identifier: MIT
"""Runs a foreign drag source and prints its button for the QA runner.

Stdout gets `bounds X Y WIDTH HEIGHT` whenever the button's screen box changes
and `label TEXT` after each label change.

On X11 the window's place comes from the X server. On the session host's sway
(`GDK_BACKEND=wayland`) no client can learn where its window is, so it comes
from sway's tree over `SWAYSOCK`; the window is undecorated there, so its
widgets start at the window's corner.
"""

import ctypes
import ctypes.util
import json
import os
import runpy
import subprocess
import sys

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("GdkX11", "4.0")
from gi.repository import GdkX11, GLib, Gtk  # noqa: E402

POLL_MS = 50
APP_ID = "io.github.lgse.Strata.TestDragSource"
WAYLAND = os.environ.get("GDK_BACKEND") == "wayland"

xlib = ctypes.CDLL(ctypes.util.find_library("X11"))
xlib.XOpenDisplay.restype = ctypes.c_void_p
xlib.XOpenDisplay.argtypes = [ctypes.c_char_p]
xlib.XDefaultRootWindow.restype = ctypes.c_ulong
xlib.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
xlib.XTranslateCoordinates.argtypes = [
    ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_int, ctypes.c_int,
    ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_ulong),
]
display = None if WAYLAND else xlib.XOpenDisplay(None)


def report(line):
    print(line, flush=True)


def sway_window(node):
    if node.get("app_id") == APP_ID:
        return node
    for child in node.get("nodes", []) + node.get("floating_nodes", []):
        found = sway_window(child)
        if found is not None:
            return found
    return None


def sway_origin():
    tree = subprocess.run(
        ["swaymsg", "--socket", os.environ["SWAYSOCK"], "--type", "get_tree", "--raw"],
        capture_output=True, check=False,
    )
    try:
        window = sway_window(json.loads(tree.stdout))
    except ValueError:
        return None
    if window is None:
        return None
    rect, inner = window["rect"], window["window_rect"]
    return rect["x"] + inner["x"], rect["y"] + inner["y"]


def surface_origin(surface):
    if WAYLAND:
        return sway_origin()
    x, y, child = ctypes.c_int(), ctypes.c_int(), ctypes.c_ulong()
    xlib.XTranslateCoordinates(
        display, GdkX11.X11Surface.get_xid(surface), xlib.XDefaultRootWindow(display),
        0, 0, ctypes.byref(x), ctypes.byref(y), ctypes.byref(child),
    )
    return x.value, y.value


set_label = Gtk.Button.set_label


def reporting_set_label(button, label):
    set_label(button, label)
    report(f"label {label}")


present = Gtk.Window.present


def button_box(window):
    """The button's screen box, or None while it is not shown or not placed."""
    button, surface = window.get_child(), window.get_surface()
    if button is None or surface is None or not button.get_mapped():
        return None
    found, bounds = button.compute_bounds(window)
    if not found or bounds.get_width() <= 0 or bounds.get_height() <= 0:
        return None
    origin = surface_origin(surface)
    if origin is None:
        return None
    offset_x, offset_y = window.get_surface_transform()
    return (
        origin[0] + int(offset_x + bounds.get_x()),
        origin[1] + int(offset_y + bounds.get_y()),
        int(bounds.get_width()),
        int(bounds.get_height()),
    )


def reporting_present(window):
    if WAYLAND:
        window.set_decorated(False)
    present(window)
    last = []

    def poll():
        box = button_box(window)
        if box is not None and box not in last:
            last[:] = [box]
            report("bounds " + " ".join(map(str, box)))
        return GLib.SOURCE_CONTINUE

    GLib.timeout_add(POLL_MS, poll)


Gtk.Button.set_label = reporting_set_label
Gtk.Window.present = reporting_present
source = sys.argv[1]
sys.argv = sys.argv[1:]
runpy.run_path(source, run_name="__main__")
