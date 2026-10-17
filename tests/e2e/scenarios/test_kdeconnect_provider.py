# SPDX-License-Identifier: MIT
"""The opt-in provider against real private D-Bus and Strata's native menus."""

import json
from pathlib import Path
import subprocess
import sys

import pytest

from harness.process import terminate

PROVIDER = Path(__file__).resolve().parents[3] / "contrib/providers/kdeconnect"


class Devices:
    def __init__(self, folder):
        self.folder = folder
        self.control = folder / "control.json"
        self.record = folder / "record.jsonl"

    def set(self, devices):
        temporary = self.control.with_suffix(".tmp")
        temporary.write_text(json.dumps({"devices": devices}))
        temporary.replace(self.control)

    def submissions(self):
        return [json.loads(line) for line in self.record.read_text().splitlines()] if self.record.exists() else []


@pytest.fixture(autouse=True)
def kdeconnect(test_environment, headless_display):
    folder = test_environment.config_home / "strata/providers/kdeconnect"
    folder.mkdir(parents=True, mode=0o700)
    devices = Devices(folder)
    devices.set({})
    # Install only in the throwaway test configuration, never in the desktop.
    (folder / "provider.json").write_text(json.dumps({
        "version": 1, "id": "kdeconnect", "name": "KDE Connect", "icons": {},
        "command": ["/usr/bin/python3", str(PROVIDER / "provider.py")],
    }))
    ready = folder / "ready"
    with (folder / "mock.log").open("w") as log:
        service = subprocess.Popen(
            [sys.executable, str(PROVIDER / "tests/mock_service.py"),
             str(devices.control), str(devices.record), str(ready)],
            env={**test_environment.variables(), **headless_display.environment},
            stdout=log, stderr=log,
        )
        try:
            from harness.tree import wait_until
            wait_until(ready.exists, message="private KDE Connect mock startup", timeout=10)
            yield devices
        finally:
            terminate(service)


def test_direct_send_and_live_device_menu(strata, kdeconnect):
    strata.open_context_menu("todo.txt")
    assert "Send via KDE Connect" not in strata.menu_items()
    kdeconnect.set({"phone1": {"name": "Pixel"}})
    direct = "Send to Pixel via KDE Connect"
    strata.wait(lambda: direct in strata.menu_items(), "one reachable device appears live")
    strata.choose_menu_item(direct)
    strata.wait(lambda: len(kdeconnect.submissions()) == 1, "direct D-Bus submission")
    assert kdeconnect.submissions()[0]["device"] == "phone1"
    assert kdeconnect.submissions()[0]["uris"] == [strata.fixture.path("todo.txt").as_uri()]
    strata.wait(lambda: strata.window.find(role="label", name="Files submitted to Pixel: 1. KDE Connect handles transfer progress and results."), "submission notice")
    assert strata.dialog() is None, "accepted send must not open a picker or success dialog"
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: direct in strata.menu_items(), "single-device menu")
    kdeconnect.set({"phone1": {"name": "Pixel"}, "tablet2": {"name": "Tablet"}})
    strata.wait(lambda: "Send via KDE Connect" in strata.menu_items(), "second device creates a branch")
    branch = strata.menu_item("Send via KDE Connect")
    bounds = branch.screen_bounds()
    strata.pointer.move_to(bounds.x + bounds.width // 2, bounds.y + bounds.height // 2)
    strata.wait(lambda: "Tablet" in strata.menu_items(), "hover opens device submenu")
    assert len(kdeconnect.submissions()) == 1, "hovering must not send"
    kdeconnect.set({"phone1": {"name": "Pixel"}})
    strata.wait(lambda: "Tablet" not in strata.menu_items(), "disconnected target withdrawn")
    assert len(kdeconnect.submissions()) == 1
    strata.dismiss_menu()
    strata.open_context_menu("todo.txt")
    strata.wait(lambda: direct in strata.menu_items(), "remaining device is a direct action")
    kdeconnect.set({})
    strata.wait(lambda: direct not in strata.menu_items(), "last device disconnected")
    strata.wait(lambda: not strata.window.find(role="label", name="KDE Connect"), "empty section withdrawn")


def test_keyboard_submenu_sends_whole_selection_to_chosen_device(strata, kdeconnect):
    kdeconnect.set({"phone1": {"name": "Pixel"}, "tablet2": {"name": "Tablet"},
                    "offline": {"name": "Offline", "reachable": False},
                    "unpaired": {"name": "Unpaired", "paired": False},
                    "disabled": {"name": "Disabled", "share": False}})
    other = 'second 冬 $HOME;`x` %23?.txt'
    strata.fixture.path(other).write_text("disposable transfer fixture")
    strata.wait(lambda: strata.entry(other), "second selected file")
    strata.select_entry("todo.txt")
    strata.click_entry_with(other, ["ctrl"])
    strata.open_context_menu(other)
    strata.wait(lambda: "Send via KDE Connect" in strata.menu_items(), "multiple eligible devices")
    # Drive focus through the menu; no accessibility action substitutes for keys.
    strata.keyboard.press("Home")
    for _ in range(60):
        if strata.menu_item("Send via KDE Connect").has_state("focused"):
            break
        strata.keyboard.press("Down")
    else:
        raise AssertionError("keyboard never reached KDE Connect submenu")
    strata.keyboard.press("Right")
    strata.wait(lambda: "Tablet" in strata.menu_items(), "keyboard opens submenu")
    assert not {"Offline", "Unpaired", "Disabled"}.intersection(strata.menu_items())
    strata.keyboard.press("Home")
    for _ in range(4):
        if strata.menu_item("Tablet").has_state("focused"):
            break
        strata.keyboard.press("Down")
    else:
        raise AssertionError("keyboard never reached chosen device")
    strata.keyboard.press("Return")
    strata.wait_for_menu_closed()
    strata.wait(lambda: len(kdeconnect.submissions()) == 1, "batch submitted once")
    submission = kdeconnect.submissions()[0]
    assert submission["device"] == "tablet2"
    assert sorted(submission["uris"]) == sorted([strata.fixture.path(name).as_uri() for name in ("todo.txt", other)])
    strata.wait(lambda: strata.window.find(role="label", name="Files submitted to Tablet: 2. KDE Connect handles transfer progress and results."), "batch submission notice")
    assert strata.dialog() is None
