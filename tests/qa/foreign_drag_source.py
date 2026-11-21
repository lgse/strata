# SPDX-License-Identifier: MIT
"""Runs a foreign drag source and prints its button for the QA runner.

With no window manager every window maps at the screen's top-left corner, where
it would cover Strata's first tab, so the source's window first moves to the
screen's bottom-right corner, which Strata's default window leaves free. Stdout
then gets `bounds X Y WIDTH HEIGHT` whenever the button's screen box changes and
`label TEXT` after each label change.
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
xlib.XDefaultScreen.argtypes = [ctypes.c_void_p]
xlib.XDisplayWidth.argtypes = [ctypes.c_void_p, ctypes.c_int]
xlib.XDisplayHeight.argtypes = [ctypes.c_void_p, ctypes.c_int]
xlib.XMoveWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int]
xlib.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
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


def move_to_bottom_right(surface):
    screen = xlib.XDefaultScreen(display)
    xlib.XMoveWindow(
        display, GdkX11.X11Surface.get_xid(surface),
        xlib.XDisplayWidth(display, screen) - surface.get_width(),
        xlib.XDisplayHeight(display, screen) - surface.get_height(),
    )
    xlib.XSync(display, False)


set_label = Gtk.Button.set_label


def reporting_set_label(button, label):
    set_label(button, label)
    report(f"label {label}")


present = Gtk.Window.present


def laid_out_bounds(window):
    button = window.get_child()
    if button is None or window.get_surface() is None or not button.get_mapped():
        return None
    found, bounds = button.compute_bounds(window)
    return bounds if found and bounds.get_width() > 0 and bounds.get_height() > 0 else None


def screen_box(window, bounds):
    origin_x, origin_y = surface_origin(window.get_surface())
    offset_x, offset_y = window.get_surface_transform()
    return (
        origin_x + int(offset_x + bounds.get_x()),
        origin_y + int(offset_y + bounds.get_y()),
        int(bounds.get_width()),
        int(bounds.get_height()),
    )


def reporting_present(window):
    present(window)
    last = []
    moved = []

    def poll():
        bounds = laid_out_bounds(window)
        if bounds is None:
            return GLib.SOURCE_CONTINUE
        if not moved:
            move_to_bottom_right(window.get_surface())
            moved.append(True)
            return GLib.SOURCE_CONTINUE
        box = screen_box(window, bounds)
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
