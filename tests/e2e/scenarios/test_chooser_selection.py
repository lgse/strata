# SPDX-License-Identifier: MIT

from pathlib import Path
from urllib.parse import unquote, urlparse
import os

import pytest
from gi.repository import GLib

from harness import tree
from harness.browser import ENTRY_ROLES
from harness.portal import open_file_request


def _find_row(chooser, name):
    for role in ENTRY_ROLES:
        node = chooser.find(role=role, name=name)
        if node is not None:
            return node
    return None


def _row(chooser, name):
    return tree.wait_until(lambda: _find_row(chooser, name), message=f"the {name!r} row")


def _selected(chooser, name):
    row = _find_row(chooser, name)
    return row is not None and row.has_state("selected")


@pytest.mark.parametrize("browser_mode", ["list", "columns"])
def test_enter_returns_every_selected_file_in_a_multiple_open_request(
    strata_binary, headless_display, test_environment, fixture_tree, keyboard, pointer, browser_mode
):
    options = {
        "current_folder": GLib.Variant("ay", os.fsencode(fixture_tree.root) + b"\0"),
        "multiple": GLib.Variant("b", True),
    }
    with open_file_request(
        strata_binary, headless_display, test_environment, fixture_tree,
        title="Multiple selection test", options=options, browser_mode=browser_mode,
    ) as (chooser, response):
        pointer.click(_row(chooser, "readme.md"))
        pointer.click(_row(chooser, "todo.txt"), modifiers=["ctrl"])
        tree.wait_until(
            lambda: _selected(chooser, "readme.md") and _selected(chooser, "todo.txt"),
            message="both files selected",
        )
        keyboard.press("Return")

        tree.wait_until(response.done, message="portal response", timeout=20)
        status, values = response.result().unpack()
        assert status == 0
        chosen = {Path(unquote(urlparse(uri).path)).name for uri in values["uris"]}
        assert chosen == {"readme.md", "todo.txt"}
