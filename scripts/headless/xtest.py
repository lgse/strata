# SPDX-License-Identifier: MIT
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
from pathlib import Path
from typing import Callable, NamedTuple, TextIO

KeySym = ctypes.c_ulong
Display = ctypes.c_void_p

CURRENT_SCREEN = -1
NO_DELAY = 0
IS_VIEWABLE = 2
BAD_WINDOW = 3
SUCCESS = 0
XKB_USE_CORE_KBD = 0x0100
LOCK_MASK = 1 << 1
ALL_MODIFIERS = 0xFF


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


class XkbStateRec(ctypes.Structure):
    _fields_ = [
        ("group", ctypes.c_ubyte), ("locked_group", ctypes.c_ubyte),
        ("base_group", ctypes.c_ushort), ("latched_group", ctypes.c_ushort),
        ("mods", ctypes.c_ubyte), ("base_mods", ctypes.c_ubyte),
        ("latched_mods", ctypes.c_ubyte), ("locked_mods", ctypes.c_ubyte),
        ("compat_state", ctypes.c_ubyte), ("grab_mods", ctypes.c_ubyte),
        ("compat_grab_mods", ctypes.c_ubyte), ("lookup_mods", ctypes.c_ubyte),
        ("compat_lookup_mods", ctypes.c_ubyte), ("ptr_buttons", ctypes.c_ushort),
    ]


class KeyboardState(NamedTuple):
    group: int
    locked_group: int
    locked_modifiers: int


INPUT_LOG_VARIABLE = "STRATA_INPUT_LOG"
OriginProvider = Callable[[], tuple[int, int]]


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
    _input_log: TextIO | None = field(init=False, default=None)
    _log_origin: OriginProvider | None = field(init=False, default=None)
    _last_origin: tuple[int, int] = field(init=False, default=(0, 0))

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
        self._x11.XResizeWindow.argtypes = [Display, ctypes.c_ulong, ctypes.c_uint, ctypes.c_uint]
        self._x11.XGetWindowAttributes.argtypes = [
            Display, ctypes.c_ulong, ctypes.POINTER(WindowAttributes),
        ]
        self._x11.XQueryPointer.argtypes = [
            Display,
            ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_ulong),
        ] + [ctypes.POINTER(ctypes.c_int)] * 4 + [ctypes.POINTER(ctypes.c_uint)]
        self._x11.XkbLockGroup.argtypes = [Display, ctypes.c_uint, ctypes.c_uint]
        self._x11.XkbLockModifiers.argtypes = [
            Display, ctypes.c_uint, ctypes.c_uint, ctypes.c_uint,
        ]
        self._x11.XkbGetState.argtypes = [
            Display, ctypes.c_uint, ctypes.POINTER(XkbStateRec),
        ]
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
        self._log(lambda: f"key {keycode} {_state(pressed)}")

    def key_needs_shift(self, keysym: int) -> bool:
        return self.keycode_for(keysym)[1]

    def lock_group(self, group: int) -> None:
        """Select a layout of the keymap, as a layout switch shortcut does."""

        if not self._x11.XkbLockGroup(self._display, XKB_USE_CORE_KBD, group):
            raise XTestError(f"cannot lock keyboard group {group}")
        self.flush()

    def lock_modifiers(self, affect: int, values: int) -> None:
        """Of the `affect` modifiers, lock those set in `values` and unlock the rest."""

        if not self._x11.XkbLockModifiers(self._display, XKB_USE_CORE_KBD, affect, values):
            raise XTestError(f"cannot lock keyboard modifiers 0x{affect:02x}")
        self.flush()

    def keyboard_state(self) -> KeyboardState:
        state = XkbStateRec()
        if self._x11.XkbGetState(self._display, XKB_USE_CORE_KBD, ctypes.byref(state)) != SUCCESS:
            raise XTestError("cannot read the keyboard state")
        return KeyboardState(state.group, state.locked_group, state.locked_mods)

    def motion(self, x: int, y: int) -> None:
        self._xtst.XTestFakeMotionEvent(
            self._display, CURRENT_SCREEN, int(x), int(y), NO_DELAY
        )
        self.flush()
        self._log(lambda: "motion {} {}".format(*self._relative(int(x), int(y))))

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

        surface = self._surface(width, height)
        return surface[1:] if surface is not None else None

    def resize_surface(self, current_width: int, current_height: int, width: int, height: int) -> None:
        """Resize the native surface identified by accessible dimensions."""

        surface = self._surface(current_width, current_height)
        if surface is None:
            raise XTestError("no native surface matches the accessible window")
        self._x11.XResizeWindow(self._display, surface[0], width, height)
        self.flush()

    def _surface(self, width: int, height: int) -> tuple[int, int, int] | None:
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
                        children[index],
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
        self._log(
            lambda: "pointer {} {} button {} {}".format(
                *self._relative(*self.pointer_position()), number, _state(pressed)
            )
        )

    def start_input_log(self, path: Path, origin: OriginProvider) -> None:
        """Append every later synthetic event to `path`.

        Positions are relative to `origin()`, the main window's surface origin,
        read when each line is written, as the `--qa` injector reads its own.
        """

        self.stop_input_log()
        path.parent.mkdir(parents=True, exist_ok=True)
        self._input_log = path.open("a", encoding="utf-8")
        self._log_origin = origin
        self._last_origin = (0, 0)

    def stop_input_log(self) -> None:
        log, self._input_log, self._log_origin = self._input_log, None, None
        if log is not None:
            log.close()

    def _relative(self, x: int, y: int) -> tuple[int, int]:
        if self._log_origin is not None:
            try:
                self._last_origin = self._log_origin()
            except Exception:  # the window can be mid-teardown; keep the last origin
                pass
        return x - self._last_origin[0], y - self._last_origin[1]

    def _log(self, line: Callable[[], str]) -> None:
        if self._input_log is not None:
            self._input_log.write(line() + "\n")
            self._input_log.flush()

    def flush(self) -> None:
        self._x11.XSync(self._display, 0)

    def close(self) -> None:
        self.stop_input_log()
        display = getattr(self, "_display", None)
        if display:
            self._xtst.XTestGrabControl(display, 0)
            self._x11.XCloseDisplay(display)
            self._display = 0


def _state(pressed: bool) -> str:
    return "down" if pressed else "up"
