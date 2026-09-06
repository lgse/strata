# SPDX-License-Identifier: GPL-3.0-or-later
"""Synthetic X11 input through the XTEST extension.

AT-SPI's own `GenerateMouseEvent` never replies on a headless server, so input
goes straight to XTEST — the same mechanism `at-spi2-registryd` would have
used. AT-SPI still locates and inspects controls; this module resolves native
surface origins and delivers input at coordinates derived from accessible bounds.
"""

from __future__ import annotations

import ctypes
import ctypes.util
from dataclasses import dataclass, field

KeySym = ctypes.c_ulong
Display = ctypes.c_void_p

CURRENT_SCREEN = -1
NO_DELAY = 0
IS_VIEWABLE = 2
BAD_WINDOW = 3


class WindowAttributes(ctypes.Structure):
    _fields_ = [
        ("x", ctypes.c_int), ("y", ctypes.c_int),
        ("width", ctypes.c_int), ("height", ctypes.c_int),
        ("border_width", ctypes.c_int), ("depth", ctypes.c_int),
        ("visual", ctypes.c_void_p), ("root", ctypes.c_ulong),
        ("window_class", ctypes.c_int), ("bit_gravity", ctypes.c_int),
        ("win_gravity", ctypes.c_int), ("backing_store", ctypes.c_int),
        ("backing_planes", ctypes.c_ulong), ("backing_pixel", ctypes.c_ulong),
        ("save_under", ctypes.c_int), ("colormap", ctypes.c_ulong),
        ("map_installed", ctypes.c_int), ("map_state", ctypes.c_int),
        ("all_event_masks", ctypes.c_long), ("your_event_mask", ctypes.c_long),
        ("do_not_propagate_mask", ctypes.c_long), ("override_redirect", ctypes.c_int),
        ("screen", ctypes.c_void_p),
    ]


class XErrorEvent(ctypes.Structure):
    _fields_ = [
        ("type", ctypes.c_int), ("display", ctypes.c_void_p),
        ("resourceid", ctypes.c_ulong), ("serial", ctypes.c_ulong),
        ("error_code", ctypes.c_ubyte), ("request_code", ctypes.c_ubyte),
        ("minor_code", ctypes.c_ubyte),
    ]


class XTestError(RuntimeError):
    """The X server or its XTEST extension is unusable."""


def _load(name: str) -> ctypes.CDLL:
    path = ctypes.util.find_library(name)
    if path is None:
        raise XTestError(f"lib{name} is not installed")
    return ctypes.CDLL(path)


@dataclass
class XTestConnection:
    """A private X connection for input injection and native surface geometry."""

    display_name: str
    _x11: ctypes.CDLL = field(init=False)
    _xtst: ctypes.CDLL = field(init=False)
    _display: int = field(init=False)
    _root: int = field(init=False, default=0)
    _keymap: dict[int, tuple[int, bool]] = field(init=False, default_factory=dict)

    def __post_init__(self) -> None:
        self._x11 = _load("X11")
        self._xtst = _load("Xtst")
        self._declare()
        display = self._x11.XOpenDisplay(self.display_name.encode())
        if not display:
            raise XTestError(f"cannot open X display {self.display_name}")
        self._display = display
        self._require_xtest()
        self._root = self._x11.XDefaultRootWindow(self._display)
        # Without this, synthetic events are discarded while the application
        # holds a pointer grab, which is exactly the state a drag runs in.
        self._xtst.XTestGrabControl(self._display, 1)
        self._keymap = self._read_keymap()

    def _declare(self) -> None:
        self._x11.XOpenDisplay.restype = Display
        self._x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        self._x11.XCloseDisplay.argtypes = [Display]
        self._x11.XFlush.argtypes = [Display]
        self._x11.XSync.argtypes = [Display, ctypes.c_int]
        self._x11.XDisplayKeycodes.argtypes = [
            Display,
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
        ]
        self._x11.XGetKeyboardMapping.restype = ctypes.POINTER(KeySym)
        self._x11.XGetKeyboardMapping.argtypes = [
            Display,
            ctypes.c_ubyte,
            ctypes.c_int,
            ctypes.POINTER(ctypes.c_int),
        ]
        self._x11.XFree.argtypes = [ctypes.c_void_p]
        self._x11.XDefaultRootWindow.restype = ctypes.c_ulong
        self._x11.XDefaultRootWindow.argtypes = [Display]
        self._x11.XSetErrorHandler.argtypes = [ctypes.c_void_p]
        self._x11.XSetErrorHandler.restype = ctypes.c_void_p
        self._x11.XQueryTree.argtypes = [
            Display, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)),
            ctypes.POINTER(ctypes.c_uint),
        ]
        self._x11.XGetWindowAttributes.argtypes = [
            Display, ctypes.c_ulong, ctypes.POINTER(WindowAttributes),
        ]
        self._x11.XQueryPointer.argtypes = [
            Display,
            ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_ulong),
        ] + [ctypes.POINTER(ctypes.c_int)] * 4 + [ctypes.POINTER(ctypes.c_uint)]
        self._xtst.XTestQueryExtension.argtypes = [Display] + [
            ctypes.POINTER(ctypes.c_int)
        ] * 4
        self._xtst.XTestGrabControl.argtypes = [Display, ctypes.c_int]
        self._xtst.XTestFakeMotionEvent.argtypes = [
            Display,
            ctypes.c_int,
            ctypes.c_int,
            ctypes.c_int,
            ctypes.c_ulong,
        ]
        self._xtst.XTestFakeButtonEvent.argtypes = [
            Display,
            ctypes.c_uint,
            ctypes.c_int,
            ctypes.c_ulong,
        ]
        self._xtst.XTestFakeKeyEvent.argtypes = [
            Display,
            ctypes.c_uint,
            ctypes.c_int,
            ctypes.c_ulong,
        ]

    def _require_xtest(self) -> None:
        values = [ctypes.c_int() for _ in range(4)]
        available = self._xtst.XTestQueryExtension(
            self._display, *(ctypes.byref(value) for value in values)
        )
        if not available:
            raise XTestError(f"the X server on {self.display_name} has no XTEST")

    def _read_keymap(self) -> dict[int, tuple[int, bool]]:
        """Map each keysym to the keycode and shift state that produces it."""

        first = ctypes.c_int()
        last = ctypes.c_int()
        self._x11.XDisplayKeycodes(
            self._display, ctypes.byref(first), ctypes.byref(last)
        )
        per_code = ctypes.c_int()
        count = last.value - first.value + 1
        mapping = self._x11.XGetKeyboardMapping(
            self._display, first.value, count, ctypes.byref(per_code)
        )
        if not mapping:
            raise XTestError("the X server returned no keyboard mapping")
        keymap: dict[int, tuple[int, bool]] = {}
        try:
            width = per_code.value
            for index in range(count):
                keycode = first.value + index
                for level in range(min(width, 2)):
                    keysym = mapping[index * width + level]
                    if keysym and keysym not in keymap:
                        keymap[keysym] = (keycode, level == 1)
        finally:
            self._x11.XFree(mapping)
        return keymap

    def keycode_for(self, keysym: int) -> tuple[int, bool]:
        try:
            return self._keymap[keysym]
        except KeyError:
            raise XTestError(
                f"keysym 0x{keysym:04x} is not on the keyboard layout"
            ) from None

    def key(self, keysym: int, pressed: bool) -> None:
        keycode, _ = self.keycode_for(keysym)
        self._xtst.XTestFakeKeyEvent(self._display, keycode, int(pressed), NO_DELAY)
        self.flush()

    def key_needs_shift(self, keysym: int) -> bool:
        return self.keycode_for(keysym)[1]

    def motion(self, x: int, y: int) -> None:
        self._xtst.XTestFakeMotionEvent(
            self._display, CURRENT_SCREEN, int(x), int(y), NO_DELAY
        )
        self.flush()

    def pointer_position(self) -> tuple[int, int]:
        """Where the X server currently believes the pointer is."""

        root = ctypes.c_ulong()
        child = ctypes.c_ulong()
        root_x = ctypes.c_int()
        root_y = ctypes.c_int()
        window_x = ctypes.c_int()
        window_y = ctypes.c_int()
        mask = ctypes.c_uint()
        self._x11.XQueryPointer(
            self._display,
            self._root,
            ctypes.byref(root),
            ctypes.byref(child),
            ctypes.byref(root_x),
            ctypes.byref(root_y),
            ctypes.byref(window_x),
            ctypes.byref(window_y),
            ctypes.byref(mask),
        )
        return root_x.value, root_y.value

    def surface_origin(self, width: int, height: int) -> tuple[int, int] | None:
        """Resolve a native popup's origin from its accessible surface dimensions."""

        if not self._display:
            raise XTestError("the X connection is closed")
        root, parent = ctypes.c_ulong(), ctypes.c_ulong()
        children = ctypes.POINTER(ctypes.c_ulong)()
        count = ctypes.c_uint()
        errors = []

        @ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.POINTER(XErrorEvent))
        def collect_error(_display, event):
            errors.append(event.contents.error_code)
            return 0

        self.flush()
        previous = self._x11.XSetErrorHandler(collect_error)
        origins = set()
        try:
            if not self._x11.XQueryTree(
                self._display, self._root, ctypes.byref(root), ctypes.byref(parent),
                ctypes.byref(children), ctypes.byref(count),
            ):
                raise XTestError("cannot enumerate native X surfaces")
            for index in range(count.value):
                attributes = WindowAttributes()
                if not self._x11.XGetWindowAttributes(
                    self._display, children[index], ctypes.byref(attributes)
                ):
                    continue
                if (
                    attributes.map_state == IS_VIEWABLE
                    and (attributes.width, attributes.height) == (width, height)
                ):
                    origins.add((
                        attributes.x + attributes.border_width,
                        attributes.y + attributes.border_width,
                    ))
        finally:
            if children:
                self._x11.XFree(children)
            self.flush()
            self._x11.XSetErrorHandler(previous)
        # A tooltip can disappear between enumeration and attribute lookup.
        if any(code != BAD_WINDOW for code in errors):
            raise XTestError(f"cannot inspect native X surfaces: {errors}")
        if len(origins) > 1:
            raise XTestError(f"ambiguous native surfaces for {width}x{height}: {origins}")
        return next(iter(origins), None)

    def button(self, number: int, pressed: bool) -> None:
        self._xtst.XTestFakeButtonEvent(
            self._display, number, int(pressed), NO_DELAY
        )
        self.flush()

    def flush(self) -> None:
        self._x11.XSync(self._display, 0)

    def close(self) -> None:
        display = getattr(self, "_display", None)
        if display:
            self._xtst.XTestGrabControl(display, 0)
            self._x11.XCloseDisplay(display)
            self._display = 0
