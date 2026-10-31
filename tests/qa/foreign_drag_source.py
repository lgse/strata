# SPDX-License-Identifier: MIT
"""Runs a foreign drag source and prints its button for the QA runner.

Stdout gets `bounds X Y WIDTH HEIGHT` whenever the button's screen box changes
and `label TEXT` after each label change.
"""

import ctypes
import ctypes.util
import runpy
import sys

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("GdkX11", "4.0")
from gi.repository import GdkX11, GLib, Gtk  # noqa: E402

POLL_MS = 50

xlib = ctypes.CDLL(ctypes.util.find_library("X11"))
xlib.XOpenDisplay.restype = ctypes.c_void_p
xlib.XOpenDisplay.argtypes = [ctypes.c_char_p]
xlib.XDefaultRootWindow.restype = ctypes.c_ulong
xlib.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
xlib.XTranslateCoordinates.argtypes = [
    ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_int, ctypes.c_int,
    ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_ulong),
]
display = xlib.XOpenDisplay(None)


def report(line):
    print(line, flush=True)


def surface_origin(surface):
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


def reporting_present(window):
    present(window)
    last = []

    def poll():
        button, surface = window.get_child(), window.get_surface()
        if button is not None and surface is not None and button.get_mapped():
            found, bounds = button.compute_bounds(window)
            if found and bounds.get_width() > 0 and bounds.get_height() > 0:
                origin_x, origin_y = surface_origin(surface)
                offset_x, offset_y = window.get_surface_transform()
                box = (
                    origin_x + int(offset_x + bounds.get_x()),
                    origin_y + int(offset_y + bounds.get_y()),
                    int(bounds.get_width()),
                    int(bounds.get_height()),
                )
                if box not in last:
                    last[:] = [box]
                    report("bounds " + " ".join(map(str, box)))
        return GLib.SOURCE_CONTINUE

    GLib.timeout_add(POLL_MS, poll)


Gtk.Button.set_label = reporting_set_label
Gtk.Window.present = reporting_present
source = sys.argv[1]
sys.argv = sys.argv[1:]
runpy.run_path(source, run_name="__main__")
