# SPDX-License-Identifier: GPL-3.0-or-later
"""Synthetic X11 input through the XTEST extension.

AT-SPI's own `GenerateMouseEvent` never replies on a headless server, so input
goes straight to XTEST — the same mechanism `at-spi2-registryd` would have
used. AT-SPI still does all of the finding and inspecting; this module only
delivers the events, and always at coordinates derived from accessible bounds.
"""

from __future__ import annotations

import ctypes
import ctypes.util
from dataclasses import dataclass, field

KeySym = ctypes.c_ulong
Display = ctypes.c_void_p

CURRENT_SCREEN = -1
NO_DELAY = 0


class XTestError(RuntimeError):
    """The X server or its XTEST extension is unusable."""


def _load(name: str) -> ctypes.CDLL:
    path = ctypes.util.find_library(name)
    if path is None:
        raise XTestError(f"lib{name} is not installed")
    return ctypes.CDLL(path)


@dataclass
class XTestConnection:
    """A private X connection used only for input injection."""

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
