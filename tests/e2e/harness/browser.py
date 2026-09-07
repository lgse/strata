# SPDX-License-Identifier: GPL-3.0-or-later
"""Page objects for the Strata window.

Scenarios talk to this class rather than to raw accessible nodes, so a change
in widget nesting is absorbed here instead of in twelve test files.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from . import screenshots, tree
from .application import Application
from .environment import TestEnvironment
from .fixtures import FixtureTree
from .display import HeadlessDisplay
from .interaction import Keyboard, Pointer
from .tree import Bounds, Node, wait_until

ENTRY_ROLES = ("list item", "table cell")
# `ui::accessibility::describe_pane` names every browser pane after its
# directory and describes it with the presentation that drew it.
VIEW_DESCRIPTIONS = {
    "Columns view": "Columns",
    "Icons view": "Icons",
    "List view": "List",
}
# `ui::accessibility::describe_entry_container` marks the list or grid itself.
ENTRY_CONTAINER_DESCRIPTION = "Files"
# `ui::inline_search::SEARCH_RESULTS_LABEL`.
SEARCH_RESULTS_LABEL = "Search results"
# `ui::preview::PREVIEW_LABEL`.
PREVIEW_LABEL = "Preview"
VIEW_MENU_LABELS = {"Columns": "Columns", "Icons": "Icons", "List": "List"}


@dataclass
class Strata:
    """One running window, driven the way a person would drive it."""

    application: Application
    keyboard: Keyboard
    pointer: Pointer
    fixture: FixtureTree
    environment: TestEnvironment
    display: HeadlessDisplay

    def __post_init__(self) -> None:
        tree.set_surface_origin_provider(self.pointer.connection.surface_origin)

    # ------------------------------------------------------------------ tree

    @property
    def window(self) -> Node:
        return self.application.root

    def diagnostics(self) -> str:
        return self.application.diagnostics()

    def wait(self, condition, message: str, timeout: float = tree.DEFAULT_TIMEOUT):
        return wait_until(
            condition, message=message, timeout=timeout, on_timeout=self.diagnostics
        )

    # ------------------------------------------------------------- containers

    def containers(self) -> list[Node]:
        """Every browser pane on screen, in left-to-right visual order.

        Panes rather than lists: an empty directory shows a placeholder in
        place of its list, and the pane must stay addressable either way.
        """

        found = [
            node
            for node in self.window.find_all(rendered=True)
            if node.description in VIEW_DESCRIPTIONS
        ]
        return sorted(found, key=lambda node: node.window_bounds().x)

    def entry_container(self, directory: str | None = None) -> Node | None:
        """The list or grid inside a pane, absent while the pane is empty."""

        pane = self._pane_or_none(directory)
        if pane is None:
            return None
        return pane.find(description=ENTRY_CONTAINER_DESCRIPTION)

    def view_mode(self) -> str:
        containers = self.containers()
        if not containers:
            raise AssertionError("no browser pane is on screen")
        return VIEW_DESCRIPTIONS[containers[0].description]

    def wait_for_view(self, mode: str) -> None:
        self.wait(
            lambda: self._view_mode_or_none() == mode,
            f"the browser to switch to the {mode} view",
        )

    def _view_mode_or_none(self) -> str | None:
        try:
            return self.view_mode()
        except (AssertionError, KeyError):
            return None

    def pane(self, directory: str | None = None) -> Node:
        """The entry container for a directory, or the deepest one."""

        return self.wait(
            lambda: self._pane_or_none(directory),
            f"the {directory or 'active'} pane",
        )

    def _pane_or_none(self, directory: str | None) -> Node | None:
        containers = self.containers()
        if not containers:
            return None
        if directory is None:
            return containers[-1]
        matching = [node for node in containers if node.name == directory]
        return matching[-1] if matching else None

    def pane_names(self) -> list[str]:
        return [node.name for node in self.containers()]

    def focused_pane(self) -> Node | None:
        for pane in reversed(self.containers()):
            if pane.find(states={"focused"}, rendered=False):
                return pane
        return None

    def current_directory(self) -> str | None:
        """The directory the browser is working in.

        Columns keeps parent panes on screen, so the pane holding keyboard
        focus is the current one; the single-pane views have only one.
        """

        pane = self.focused_pane()
        if pane is not None:
            return pane.name
        containers = self.containers()
        return containers[-1].name if containers else None

    def wait_for_directory(self, name: str) -> None:
        self.wait(
            lambda: self.current_directory() == name,
            f"the browser to be working in {name!r}",
        )

    # ---------------------------------------------------------------- entries

    def entries(self, directory: str | None = None) -> list[Node]:
        container = self.entry_container(directory)
        if container is None:
            return []
        found: list[Node] = []
        for role in ENTRY_ROLES:
            found.extend(node for node in container.find_all(role=role) if node.name)
        return sorted(
            found, key=lambda node: (node.window_bounds().y, node.window_bounds().x)
        )

    def is_empty(self, directory: str | None = None) -> bool:
        return self.entry_container(directory) is None

    def matches(self, directory: str | None = None) -> list[str]:
        """What a recursive query currently lists.

        Columns replaces the pane's own listing with the matches; the
        single-pane views show them in a separate results list.
        """

        results = self.window.find(role="list", name=SEARCH_RESULTS_LABEL)
        if results is not None:
            return [row.name for row in results.find_all(role="list item") if row.name]
        return self.entry_names(directory)

    def entry_names(self, directory: str | None = None) -> list[str]:
        return [node.name for node in self.entries(directory)]

    def entry(self, name: str, directory: str | None = None) -> Node:
        """Wait for an entry to appear, then return it.

        With no directory the search covers every pane on screen, so a
        scenario does not have to know which Miller column holds the entry.
        """

        return self.wait(
            lambda: self._entry_or_none(name, directory),
            f"the entry {name!r}"
            + (f" in {directory!r}" if directory else "")
            + " to appear",
        )

    def _entry_or_none(self, name: str, directory: str | None) -> Node | None:
        if directory is not None:
            candidates = self.entries(directory)
        else:
            candidates = [
                node
                for pane in self.containers()
                for node in self.entries(pane.name)
            ]
        for node in candidates:
            if node.name == name:
                return node
        return None

    def wait_for_entry_gone(self, name: str, directory: str | None = None) -> None:
        self.wait(
            lambda: self._entry_or_none(name, directory) is None,
            f"the entry {name!r} to disappear",
        )

    def wait_for_entries(self, names: list[str], directory: str | None = None) -> None:
        self.wait(
            lambda: self.entry_names(directory) == names,
            f"the pane to list {names}",
        )

    # -------------------------------------------------------------- selection

    def selected_names(self, directory: str | None = None) -> list[str]:
        """Selected entries in one pane, deepest pane by default."""

        return [
            node.name
            for node in self.entries(directory)
            if node.has_state("selected")
        ]

    def all_selected_names(self) -> list[str]:
        """Selected entries across every pane on screen."""

        return [
            node.name
            for pane in self.containers()
            for node in self.entries(pane.name)
            if node.has_state("selected")
        ]

    def wait_for_selection(
        self, names: list[str], directory: str | None = None
    ) -> None:
        self.wait(
            lambda: self.selected_names(directory) == names,
            f"the selection to become {names}",
        )

    def focused_name(self) -> str | None:
        for role in ENTRY_ROLES:
            for node in self.window.find_all(role=role, states={"focused"}):
                if node.name:
                    return node.name
        return None

    def wait_for_focused_entry(self, name: str) -> None:
        self.wait(
            lambda: self.focused_name() == name,
            f"keyboard focus to reach {name!r}",
        )

    def focused_node(self) -> Node | None:
        found = self.window.find_all(states={"focused"})
        return found[0] if found else None

    # ---------------------------------------------------------------- actions

    def click_entry(self, name: str, directory: str | None = None) -> Node:
        node = self.entry(name, directory)
        self.pointer.click(node)
        return node

    def click_entry_with(
        self,
        name: str,
        modifiers: list[str],
        directory: str | None = None,
    ) -> Node:
        node = self.entry(name, directory)
        self.pointer.click(node, modifiers=modifiers)
        return node

    def double_click_entry(self, name: str, directory: str | None = None) -> Node:
        node = self.entry(name, directory)
        self.pointer.double_click(node)
        return node

    def select_entry(self, name: str, directory: str | None = None) -> Node:
        """Click an entry and wait for it to report itself selected."""

        self.click_entry(name, directory)
        return self.wait(
            lambda: self._selected_entry_or_none(name, directory),
            f"{name!r} to become selected",
        )

    def _selected_entry_or_none(self, name: str, directory: str | None):
        node = self._entry_or_none(name, directory)
        return node if node is not None and node.has_state("selected") else None

    def open_directory(self, name: str, directory: str | None = None) -> None:
        """Open a directory the way the active presentation expects."""

        if self.view_mode() == "Columns":
            self.select_entry(name, directory)
        else:
            self.double_click_entry(name, directory)
        self.wait_for_directory(name)

    def empty_point(self, directory: str | None = None) -> tuple[int, int]:
        """A point inside a pane that is below its last entry."""

        bounds = self.pane(directory).screen_bounds()
        return bounds.x + bounds.width // 2, bounds.y + bounds.height - 14

    def background_point(self, directory: str | None = None) -> tuple[int, int]:
        """A point on the entry list itself, below the last entry."""

        container = self.entry_container(directory)
        if container is None:
            # The bottom edge can be Columns' paste-target footer, not its content.
            return self.pane(directory).screen_bounds().center
        bounds = container.screen_bounds()
        entries = self.entries(directory)
        lowest = max(
            (node.screen_bounds().y + node.screen_bounds().height for node in entries),
            default=bounds.y,
        )
        y = min(bounds.y + bounds.height - 6, max(lowest + 12, bounds.y + 6))
        return bounds.x + bounds.width // 2, y

    def hover_pane(self, directory: str | None = None) -> Node:
        """Point at a pane so pointer-targeted actions apply to it."""

        pane = self.pane(directory)
        self.pointer.move_to(*self.empty_point(directory))
        return pane

    def paste_target(self) -> str | None:
        """The pane Strata says Ctrl+V would paste into."""

        for pane in self.containers():
            if pane.find(role="label", name_matches="Paste here"):
                return pane.name
        return None

    def paste_into(self, directory: str | None = None) -> None:
        """Aim the paste at a pane, then paste into it."""

        pane = self.hover_pane(directory)
        # Columns marks the pane Ctrl+V would target; the single-pane views
        # have only one candidate and show no marker.
        self.wait(
            lambda: self.paste_target() in (pane.name, None),
            f"the paste target to become {pane.name!r}",
        )
        self.keyboard.press("ctrl+v")

    # Enough steps to cross any fixture directory in the suite.
    KEYBOARD_SELECT_STEPS = 40

    def select_entry_with_keyboard(self, name: str) -> Node:
        """Move the keyboard cursor onto an entry without using the pointer."""

        next_key = "Right" if self.view_mode() == "Icons" else "Down"
        self.keyboard.press("Home")
        self.wait(lambda: self.focused_name() is not None, "the keyboard cursor")
        for _ in range(self.KEYBOARD_SELECT_STEPS):
            if self.focused_name() == name:
                return self.entry(name)
            previous = self.focused_name()
            self.keyboard.press(next_key)
            self.wait(
                lambda: self.focused_name() != previous,
                f"the cursor to move past {previous!r}",
            )
        raise AssertionError(f"the keyboard cursor never reached {name!r}")

    def open_context_menu(self, name: str, directory: str | None = None) -> Node:
        self.pointer.right_click(self.entry(name, directory))
        return self.wait(self.context_menu, f"the context menu for {name!r}")

    def context_menu(self) -> Node | None:
        return self.window.find(role="menu")

    def wait_for_menu_closed(self) -> None:
        self.wait(lambda: self.context_menu() is None, "the menu to close")

    def menu_items(self) -> list[str]:
        menu = self.context_menu()
        return [node.name for node in menu.find_all(role="menu item")] if menu else []

    def menu_item(self, label: str) -> Node:
        return self.wait(
            lambda: (self.context_menu() or self.window).find(
                role="menu item", name=label
            ),
            f"the menu item {label!r}",
        )

    def choose_menu_item(self, label: str) -> None:
        item = self.wait(
            lambda: (self.context_menu() or self.window).find(
                role="menu item", name=label
            ),
            f"the menu item {label!r}",
        )
        self.pointer.click(item)
        self.wait_for_menu_closed()

    def dismiss_menu(self) -> None:
        self.keyboard.press("Escape")
        self.wait_for_menu_closed()

    def open_appearance_menu(self) -> Node:
        button = self.window.find(role="toggle button", name="Appearance")
        if button is None:
            raise AssertionError("the Appearance button is missing")
        self.pointer.click(button)
        return self.wait(
            lambda: self.window.find(role="button", name="Columns"),
            "the appearance menu to open",
        )

    def switch_view(self, mode: str) -> None:
        """Switch presentation through the appearance menu."""

        self.open_appearance_menu()
        option = self.wait(
            lambda: self.window.find(role="button", name=VIEW_MENU_LABELS[mode]),
            f"the {mode} option",
        )
        self.pointer.click(option)
        self.wait_for_view(mode)

    def sidebar_button(self, name: str) -> Node:
        node = self.window.find(role="button", name=name)
        if node is None:
            raise AssertionError(f"no sidebar button named {name!r}")
        return node

    def header_button(self, name: str) -> Node:
        node = self.window.find(role="button", name=name)
        if node is None:
            raise AssertionError(f"no button named {name!r}")
        return node

    # A tall settings page can extend past the window, and GTK still reports
    # those widgets as showing, so scrolling is driven by geometry.
    REVEAL_SCROLL_ATTEMPTS = 12

    def on_screen(self, node: Node) -> bool:
        window = self.window.screen_bounds()
        bounds = node.screen_bounds()
        return (
            bounds.height > 0
            and bounds.y >= window.y
            and bounds.y + bounds.height <= window.y + window.height
        )

    def reveal(self, **criteria) -> Node:
        """Find a control that may be scrolled out of view and bring it on."""

        node = self.wait(
            lambda: self.window.find(rendered=False, **criteria),
            f"a control matching {criteria}",
        )
        if self.on_screen(node):
            return node
        node.scroll_into_view()
        window = self.window.screen_bounds()
        centre = (window.x + window.width // 2, window.y + window.height // 2)
        for _ in range(self.REVEAL_SCROLL_ATTEMPTS):
            if self.on_screen(node):
                return self.settle(node)
            below = node.screen_bounds().y > centre[1]
            self.pointer.scroll(centre, clicks=3, down=below)
        self.wait(lambda: self.on_screen(node), f"{node!r} to be scrolled into view")
        return node

    def settle(self, node: Node, samples: int = 3) -> Node:
        """Wait until a node stops moving.

        Scroll deceleration keeps running after the wheel stops, and clicking a
        position measured mid-scroll lands on the wrong control.
        """

        history: list[Bounds] = []

        def still() -> bool:
            history.append(node.screen_bounds())
            del history[:-samples]
            return len(history) == samples and len(set(history)) == 1

        self.wait(still, f"{node!r} to stop moving")
        return node

    def editable_field(self) -> Node:
        return self.wait(
            lambda: self.window.find(role="text", states={"editable", "focused"}),
            "an editable field to take focus",
        )

    def preview(self) -> Node | None:
        """The quick preview drawer, when it is on screen."""

        return self.window.find(name=PREVIEW_LABEL)

    def preview_shows(self, text: str) -> bool:
        preview = self.preview()
        if preview is None:
            return False
        for role in ("label", "text"):
            for node in preview.find_all(role=role, rendered=False):
                if text in node.name or text in node.text:
                    return True
        return False

    def dialog(self) -> Node | None:
        return self.window.find(role="dialog") or self.window.find(role="alert")

    def wait_for_dialog(self) -> Node:
        return self.wait(self.dialog, "a dialog to appear")

    def dialog_button(self, label: str) -> Node:
        dialog = self.wait_for_dialog()
        node = dialog.find(role="button", name=label)
        if node is None:
            raise AssertionError(
                f"no {label!r} button in the dialog\n{dialog.dump()}"
            )
        return node

    # ---------------------------------------------------------------- capture

    def screenshot(self, destination: Path) -> Path:
        return screenshots.capture(self.display.display, destination)

    def park_pointer(self) -> None:
        """Move the pointer out of the window so hover styling is stable."""

        bounds = self.window.screen_bounds()
        self.pointer.move_to(bounds.width - 4, bounds.height - 4)

    def window_bounds(self) -> Bounds:
        return self.window.screen_bounds()
