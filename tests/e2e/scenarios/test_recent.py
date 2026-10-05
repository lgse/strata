# SPDX-License-Identifier: MIT
"""The pinned image has no GVfs and uses GIO_USE_VFS=local, so Recent is unavailable."""

from __future__ import annotations

RECENT_URIS = ("recent:///", "recent://")


def _switch(window, name):
    return next(
        (
            node
            for node in window.find_all(name=name, rendered=False)
            if node.role in {"check box", "toggle button", "switch"}
        ),
        None,
    )


def _navigate(strata, uri):
    strata.keyboard.press("ctrl+l")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(uri)
    strata.wait(lambda: field.text == uri, f"{uri} to be typed")
    strata.keyboard.press("Return")


def test_recent_is_absent_from_the_sidebar_without_a_platform_backend(strata):
    strata.sidebar_button("Home")
    strata.sidebar_button("Trash")

    assert strata.window.find(role="button", name="Recent") is None, (
        "Recent must not be offered when the recent backend is unavailable"
    )


def test_neither_recent_preference_state_can_add_an_unsupported_place(strata):
    settings = strata.window.find(role="button", name="Settings")
    assert settings is not None and settings.activate()
    strata.wait(
        lambda: _switch(strata.window, "Show Recent in sidebar"),
        "the Recent sidebar preference",
    )
    enabled = "pressed" in _switch(strata.window, "Show Recent in sidebar").states

    for _ in range(2):
        _switch(strata.window, "Show Recent in sidebar").activate()
        enabled = not enabled
        strata.wait(
            lambda: (
                "pressed" in _switch(strata.window, "Show Recent in sidebar").states
            )
            == enabled,
            "the Recent preference to change",
        )
        strata.wait(
            lambda: strata.window.find(role="button", name="Recent") is None,
            "Recent to stay hidden while its backend is unavailable",
        )

    strata.keyboard.press("Escape")
    strata.sidebar_button("Home")
    strata.sidebar_button("Trash")


def test_every_recent_uri_spelling_is_refused_without_disturbing_the_session(strata):
    root = strata.fixture.root.name
    strata.entry("documents")

    for uri in RECENT_URIS:
        _navigate(strata, uri)

        dialog = strata.wait(
            lambda: strata.window.find(role="dialog", name="Unable to open location"),
            f"the failure report for {uri}",
        )
        assert "backend isn't installed" in dialog.dump(), (
            f"{uri} should explain why it cannot be opened"
        )
        strata.keyboard.press("Escape")
        strata.wait(
            lambda: strata.window.find(role="dialog", name="Unable to open location")
            is None,
            "the failure report to close",
        )
        strata.wait_for_directory(root)
        strata.entry("documents")
