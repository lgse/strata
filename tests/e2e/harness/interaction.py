# SPDX-License-Identifier: GPL-3.0-or-later
"""Keyboard and pointer input.

Pointer coordinates always come from a node's accessible bounds; no scenario
passes a literal screen position.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Sequence

from .tree import Bounds, Node
from .xtest import XTestConnection, XTestError

# X11 keysyms for the keys the scenarios use. Printable ASCII maps to its own
# code point, so only the named keys need an entry.
KEYSYMS: dict[str, int] = {
    "BackSpace": 0xFF08,
    "Tab": 0xFF09,
    "Return": 0xFF0D,
    "Escape": 0xFF1B,
    "Delete": 0xFFFF,
    "Home": 0xFF50,
    "Left": 0xFF51,
    "Up": 0xFF52,
    "Right": 0xFF53,
    "Down": 0xFF54,
    "Page_Up": 0xFF55,
    "Page_Down": 0xFF56,
    "End": 0xFF57,
    "space": 0x0020,
    "F1": 0xFFBE,
    "F2": 0xFFBF,
    "F5": 0xFFC2,
    "Menu": 0xFF67,
}
MODIFIER_KEYSYMS: dict[str, int] = {
    "ctrl": 0xFFE3,  # Control_L
    "shift": 0xFFE1,  # Shift_L
    "alt": 0xFFE9,  # Alt_L
}

# Synthetic events are delivered asynchronously, so these are transport
# settling gaps, not waits on application state. Every assertion still
# synchronizes through `wait_until`.
EVENT_GAP = 0.02
POINTER_GAP = 0.05
# Comfortably inside GTK's default double-click time of 400ms.
DOUBLE_CLICK_GAP = 0.09
# Comfortably outside it, so two clicks read as two separate single clicks.
SEPARATE_CLICK_GAP = 0.6
DRAG_STEPS = 16
ARRIVAL_ATTEMPTS = 5


def keysym(key: str) -> int:
    if key in KEYSYMS:
        return KEYSYMS[key]
    if len(key) == 1:
        return ord(key)
    raise KeyError(f"no keysym mapping for {key!r}; add one to KEYSYMS")


@dataclass
class Keyboard:
    """Synthetic keyboard input."""

    connection: XTestConnection

    def press(self, chord: str) -> None:
        """Send one key, optionally with modifiers, as `ctrl+shift+n`."""

        parts = chord.split("+")
        # A trailing "+" means the plus key itself.
        key = parts[-1] if parts[-1] else "+"
        modifiers = [part.lower() for part in parts[:-1] if part]
        held = [MODIFIER_KEYSYMS[modifier] for modifier in modifiers]
        for modifier in held:
            self.connection.key(modifier, True)
            time.sleep(EVENT_GAP)
        try:
            self._tap(keysym(key))
        finally:
            for modifier in reversed(held):
                self.connection.key(modifier, False)
                time.sleep(EVENT_GAP)

    def press_repeatedly(self, chord: str, times: int) -> None:
        for _ in range(times):
            self.press(chord)

    def type_text(self, text: str) -> None:
        for character in text:
            self._tap(ord(character))

    def _tap(self, symbol: int) -> None:
        shift = MODIFIER_KEYSYMS["shift"]
        try:
            needs_shift = self.connection.key_needs_shift(symbol)
        except XTestError:
            raise
        if needs_shift:
            self.connection.key(shift, True)
            time.sleep(EVENT_GAP)
        self.connection.key(symbol, True)
        time.sleep(EVENT_GAP)
        self.connection.key(symbol, False)
        time.sleep(EVENT_GAP)
        if needs_shift:
            self.connection.key(shift, False)
            time.sleep(EVENT_GAP)


@dataclass
class Pointer:
    """Synthetic pointer input, aimed by accessible bounds."""

    connection: XTestConnection
    _last_release: float = field(default=float("-inf"), init=False)

    def move_to(self, x: int, y: int) -> None:
        # Approach in two steps so GTK sees enter and motion rather than a
        # single teleport; hover and drag handling both depend on it.
        self.connection.motion(x - 6, y - 6)
        time.sleep(EVENT_GAP)
        self.connection.motion(x, y)
        time.sleep(POINTER_GAP)
        self._confirm_arrival(x, y)

    def _confirm_arrival(self, x: int, y: int) -> None:
        """Make sure the server moved the pointer before a button is pressed.

        A press delivered while the pointer is still somewhere else lands on
        the wrong widget, or on none at all.
        """

        for _ in range(ARRIVAL_ATTEMPTS):
            if self.connection.pointer_position() == (x, y):
                return
            self.connection.motion(x, y)
            time.sleep(POINTER_GAP)
        raise AssertionError(
            f"the pointer did not reach ({x}, {y}); it is at "
            f"{self.connection.pointer_position()}"
        )

    def click(
        self,
        node: Node,
        *,
        button: int = 1,
        at: tuple[int, int] | None = None,
        modifiers: Sequence[str] = (),
    ) -> None:
        x, y = self._target(node, at)
        self.move_to(x, y)
        held = [MODIFIER_KEYSYMS[modifier.lower()] for modifier in modifiers]
        for modifier in held:
            self.connection.key(modifier, True)
            time.sleep(EVENT_GAP)
        try:
            self._tap(button)
        finally:
            for modifier in reversed(held):
                self.connection.key(modifier, False)
                time.sleep(EVENT_GAP)
        time.sleep(POINTER_GAP)

    def double_click(self, node: Node, *, at: tuple[int, int] | None = None) -> None:
        x, y = self._target(node, at)
        self.move_to(x, y)
        self._tap(1)
        time.sleep(DOUBLE_CLICK_GAP)
        self._tap(1)
        time.sleep(POINTER_GAP)

    def click_twice_slowly(self, node: Node, *, at: tuple[int, int] | None = None):
        """Two clicks far enough apart that GTK does not pair them."""

        self.click(node, at=at)
        time.sleep(SEPARATE_CLICK_GAP)
        self.click(node, at=at)

    def right_click(self, node: Node, *, at: tuple[int, int] | None = None) -> None:
        self.click(node, button=3, at=at)

    def drag(
        self,
        source: Node,
        target: Node,
        *,
        target_point: tuple[int, int] | None = None,
        steps: int = DRAG_STEPS,
    ) -> None:
        """Press on `source`, travel to `target`, and release."""

        end = target_point or target.screen_bounds().center
        self._drag(source.screen_bounds().center, end, steps=steps, release=True)

    def drag_to_point(
        self, source: Node, end: tuple[int, int], *, steps: int = DRAG_STEPS
    ) -> None:
        self._drag(source.screen_bounds().center, end, steps=steps, release=True)

    def abandon_drag(
        self,
        source: Node,
        outside: tuple[int, int],
        *,
        steps: int = DRAG_STEPS,
    ) -> None:
        """Start a drag over a valid target, then release outside the window."""

        start = source.screen_bounds().center
        self._drag(start, outside, steps=steps, release=False)
        self.connection.button(1, False)
        time.sleep(POINTER_GAP)

    def scroll(self, at: tuple[int, int], *, clicks: int = 3, down: bool = True):
        """Turn the wheel over a point. Buttons 4 and 5 are wheel up and down."""

        self.move_to(*at)
        for _ in range(clicks):
            self._tap(5 if down else 4)

    def park(self, bounds: Bounds) -> None:
        """Move the pointer away so hover styling does not vary between runs."""

        self.move_to(bounds.x + bounds.width - 4, bounds.y + bounds.height - 4)

    @staticmethod
    def _target(node: Node, at: tuple[int, int] | None) -> tuple[int, int]:
        return at if at is not None else node.screen_bounds().center

    @staticmethod
    def _away_from(node: Node) -> tuple[int, int]:
        bounds = node.toplevel().screen_bounds()
        return bounds.x + bounds.width // 2, bounds.y + bounds.height - 8

    def _tap(self, button: int) -> None:
        self.connection.button(button, True)
        time.sleep(EVENT_GAP)
        self.connection.button(button, False)
        self._last_release = time.monotonic()
        time.sleep(EVENT_GAP)

    def _drag(
        self,
        start: tuple[int, int],
        end: tuple[int, int],
        *,
        steps: int,
        release: bool,
    ) -> None:
        self.move_to(*start)
        # A drag immediately after selecting must not become a double-click
        # activation before its first motion event reaches GTK.
        remaining = SEPARATE_CLICK_GAP - (time.monotonic() - self._last_release)
        if remaining > 0:
            time.sleep(remaining)
        self.connection.button(1, True)
        time.sleep(POINTER_GAP)
        # GTK starts a drag only once the pointer passes the drag threshold,
        # and the drop site needs motion events to register the hover.
        for step in range(1, steps + 1):
            x = start[0] + (end[0] - start[0]) * step // steps
            y = start[1] + (end[1] - start[1]) * step // steps
            self.connection.motion(x, y)
            time.sleep(EVENT_GAP)
        self.connection.motion(*end)
        time.sleep(POINTER_GAP)
        if release:
            self.connection.button(1, False)
            time.sleep(POINTER_GAP)
