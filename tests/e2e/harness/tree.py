# SPDX-License-Identifier: GPL-3.0-or-later
"""Accessible-tree navigation.

Controls are located by accessible role, name, and state. Nothing here matches
on screen pixels, and no scenario may hard-code an absolute coordinate: pointer
targets are always derived from a located node's accessible bounds.
"""

from __future__ import annotations

import re
import time
from dataclasses import dataclass
from typing import Callable, Iterable, Iterator

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi  # noqa: E402  (must follow require_version)

DEFAULT_TIMEOUT = 15.0
POLL_INTERVAL = 0.05
# Deep enough for the browser's nested panes, shallow enough that a cycle in a
# misbehaving tree cannot hang the run.
MAX_DEPTH = 60

TOPLEVEL_ROLES = frozenset({"frame", "window", "dialog", "alert", "file chooser"})
SurfaceOriginProvider = Callable[[int, int], tuple[int, int] | None]
_surface_origin: SurfaceOriginProvider | None = None


def set_surface_origin_provider(provider: SurfaceOriginProvider | None) -> None:
    global _surface_origin
    _surface_origin = provider


class TreeTimeout(AssertionError):
    """A condition on the accessible tree did not hold in time."""


@dataclass(frozen=True)
class Bounds:
    x: int
    y: int
    width: int
    height: int

    @property
    def center(self) -> tuple[int, int]:
        return self.x + self.width // 2, self.y + self.height // 2

    def inset_center(self, fraction: float) -> tuple[int, int]:
        """A point `fraction` of the way across the box, vertically centred."""

        return (
            self.x + max(1, int(self.width * fraction)),
            self.y + self.height // 2,
        )


class Node:
    """A thin, comparable wrapper over an `Atspi.Accessible`."""

    __slots__ = ("_accessible",)

    def __init__(self, accessible: Atspi.Accessible) -> None:
        self._accessible = accessible

    @property
    def accessible(self) -> Atspi.Accessible:
        return self._accessible

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Node) and self._accessible == other._accessible

    def __hash__(self) -> int:
        return hash(id(self._accessible))

    def __repr__(self) -> str:
        return f"<{self.role} {self.name!r}>"

    @property
    def role(self) -> str:
        try:
            name = self._accessible.get_role_name()
            return "button" if name == "push button" else name
        except Exception:
            return "<gone>"

    @property
    def name(self) -> str:
        try:
            return self._accessible.get_name() or ""
        except Exception:
            return ""

    @property
    def description(self) -> str:
        try:
            return self._accessible.get_description() or ""
        except Exception:
            return ""

    @property
    def states(self) -> frozenset[str]:
        try:
            state_set = self._accessible.get_state_set()
            return frozenset(state.value_nick for state in state_set.get_states())
        except Exception:
            return frozenset()

    def has_state(self, state: str) -> bool:
        return state in self.states

    @property
    def alive(self) -> bool:
        try:
            self._accessible.get_role_name()
            return True
        except Exception:
            return False

    @property
    def text(self) -> str:
        """The editable or displayed text, when the node implements Text."""

        try:
            interface = Atspi.Accessible.get_text_iface(self._accessible)
        except Exception:
            interface = None
        if interface is None:
            return ""
        try:
            length = Atspi.Text.get_character_count(interface)
            return Atspi.Text.get_text(interface, 0, length) or ""
        except Exception:
            return ""

    @property
    def children(self) -> list["Node"]:
        try:
            count = self._accessible.get_child_count()
        except Exception:
            return []
        found = []
        for index in range(count):
            try:
                child = self._accessible.get_child_at_index(index)
            except Exception:
                continue
            if child is not None:
                found.append(Node(child))
        return found

    @property
    def parent(self) -> "Node | None":
        try:
            parent = self._accessible.get_parent()
        except Exception:
            return None
        return Node(parent) if parent is not None else None

    def ancestors(self) -> Iterator["Node"]:
        current = self.parent
        depth = 0
        while current is not None and depth < MAX_DEPTH:
            yield current
            current = current.parent
            depth += 1

    def toplevel(self) -> "Node":
        for ancestor in self.ancestors():
            if ancestor.role in TOPLEVEL_ROLES:
                return ancestor
        return self

    def window_bounds(self) -> Bounds:
        return self._bounds(Atspi.CoordType.WINDOW)

    def screen_bounds(self) -> Bounds:
        """Bounds in screen coordinates.

        Without a window manager the X server reports no surface origin, so
        AT-SPI's own screen coordinates collapse to the origin. Composing the
        toplevel's screen position with the node's window-relative box is
        correct either way.
        """

        toplevel = self.toplevel()
        origin = toplevel._bounds(Atspi.CoordType.SCREEN)
        if toplevel == self:
            return origin
        local = self.window_bounds()
        if _surface_origin is not None:
            for ancestor in (self, *self.ancestors()):
                if ancestor == toplevel:
                    break
                bounds = ancestor.window_bounds()
                # GTK 4.14 locates popups relative to their anchor widget, not
                # their native surface. Newer GTK already reports the real origin.
                if (
                    bounds.width > 0 and bounds.height > 0
                    and (bounds.width, bounds.height) != (origin.width, origin.height)
                ):
                    position = _surface_origin(bounds.width, bounds.height)
                    if position is not None:
                        return Bounds(
                            position[0] + local.x - bounds.x,
                            position[1] + local.y - bounds.y,
                            local.width, local.height,
                        )
        return Bounds(origin.x + local.x, origin.y + local.y, local.width, local.height)

    def _bounds(self, coordinate_type: Atspi.CoordType) -> Bounds:
        try:
            extents = self._accessible.get_extents(coordinate_type)
        except Exception:
            # Application and desktop roots do not implement Component.
            return Bounds(0, 0, 0, 0)
        return Bounds(extents.x, extents.y, extents.width, extents.height)

    def activate(self) -> bool:
        """Invoke the node's default accessible action.

        Coordinate-free, so it is unaffected by a scroller still settling.
        Pointer behavior itself is always exercised with real clicks; this is
        for reaching a control that a scenario only needs to set up.
        """

        try:
            action = Atspi.Accessible.get_action_iface(self._accessible)
            if action is None:
                return False
            return bool(Atspi.Action.do_action(action, 0))
        except Exception:
            return False

    def scroll_into_view(self) -> bool:
        """Ask the containing scroller to bring this node on screen."""

        try:
            component = Atspi.Accessible.get_component_iface(self._accessible)
            if component is None:
                return False
            return bool(
                Atspi.Component.scroll_to(component, Atspi.ScrollType.ANYWHERE)
            )
        except Exception:
            return False

    def is_rendered(self) -> bool:
        """Whether the node occupies a sane, visible box."""

        if not {"showing", "visible"} <= self.states:
            return False
        bounds = self.window_bounds()
        return 0 < bounds.width < 100_000 and 0 < bounds.height < 100_000

    def walk(self, depth: int = 0) -> Iterator[tuple[int, "Node"]]:
        yield depth, self
        if depth >= MAX_DEPTH:
            return
        for child in self.children:
            yield from child.walk(depth + 1)

    def find_all(
        self,
        *,
        role: str | None = None,
        name: str | None = None,
        name_matches: str | None = None,
        description: str | None = None,
        states: Iterable[str] = (),
        without_states: Iterable[str] = (),
        rendered: bool = True,
        predicate: Callable[["Node"], bool] | None = None,
    ) -> list["Node"]:
        wanted = frozenset(states)
        unwanted = frozenset(without_states)
        pattern = re.compile(name_matches) if name_matches else None
        found = []
        for _, node in self.walk():
            if role is not None and node.role != role:
                continue
            if name is not None and node.name != name:
                continue
            if pattern is not None and not pattern.search(node.name):
                continue
            if description is not None and node.description != description:
                continue
            node_states = node.states
            if not wanted <= node_states:
                continue
            if unwanted & node_states:
                continue
            if rendered and not node.is_rendered():
                continue
            if predicate is not None and not predicate(node):
                continue
            found.append(node)
        return found

    def find(self, **criteria) -> "Node | None":
        found = self.find_all(**criteria)
        return found[0] if found else None

    def dump(self, limit: int = 4000) -> str:
        """A readable snapshot of the subtree, used in failure reports."""

        lines = []
        for depth, node in self.walk():
            if len(lines) >= limit:
                lines.append("... truncated")
                break
            bounds = node.window_bounds()
            interesting = sorted(node.states & REPORTED_STATES)
            line = (
                f"{'  ' * depth}{node.role} {node.name!r} "
                f"[{bounds.x},{bounds.y} {bounds.width}x{bounds.height}] "
                f"{interesting}"
            )
            if node.description:
                line += f" description={node.description!r}"
            lines.append(line)
        return "\n".join(lines)


REPORTED_STATES = frozenset(
    {
        "checked",
        "editable",
        "enabled",
        "expanded",
        "focusable",
        "focused",
        "selectable",
        "selected",
        "sensitive",
        "showing",
        "visible",
    }
)


def wait_until(
    condition: Callable[[], object],
    *,
    message: str,
    timeout: float = DEFAULT_TIMEOUT,
    on_timeout: Callable[[], str] | None = None,
):
    """Poll `condition` until it returns something truthy.

    Scenarios synchronize on observable conditions through this helper. Fixed
    sleeps are not used anywhere in the suite.
    """

    deadline = time.monotonic() + timeout
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            result = condition()
        except Exception as error:  # a node can vanish mid-poll
            last_error = error
            result = None
        if result:
            return result
        time.sleep(POLL_INTERVAL)
    report = f"timed out after {timeout:.0f}s waiting for {message}"
    if last_error is not None:
        report += f"\nlast error: {last_error!r}"
    if on_timeout is not None:
        report += "\n\naccessibility tree:\n" + on_timeout()
    raise TreeTimeout(report)


def wait_while(condition: Callable[[], object], **kwargs):
    return wait_until(lambda: not condition(), **kwargs)


def connect() -> None:
    Atspi.init()


def desktop() -> Node:
    return Node(Atspi.get_desktop(0))


def find_application(name: str) -> Node | None:
    for child in desktop().children:
        if child.name == name:
            return child
    return None
