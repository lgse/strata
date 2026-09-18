# SPDX-License-Identifier: MIT
"""The saved thumbnail-worker stepper is discoverable and responds to pointer input."""

import pytest


@pytest.mark.preferences(thumbnail_workers=16)
def test_thumbnail_workers_stepper_persists_changes_and_resets(strata):
    settings = strata.window.find(role="button", name="Settings")
    assert settings is not None and settings.activate()
    search = strata.wait(
        lambda: strata.window.find(role="text", name="Search settings"),
        "settings search",
    )
    strata.pointer.click(search)
    strata.keyboard.type_text("thumbnail workers")

    def control(name):
        return strata.wait(
            lambda: strata.window.find(role="button", name=name),
            name,
        )

    for name, expected in [
        ("Decrease thumbnail workers", "15"),
        ("Increase thumbnail workers", "16"),
    ]:
        strata.pointer.click(control(name))
        strata.wait(
            lambda: strata.environment.read_preferences().get("thumbnail_workers") == expected,
            f"worker count saved as {expected}",
        )
    assert not control("Increase thumbnail workers").has_state("sensitive")
    reset = control("16")
    assert reset.description == "Reset thumbnail workers"
    strata.pointer.click(reset)
    strata.wait(
        lambda: strata.environment.read_preferences().get("thumbnail_workers") in {"1", "2", "3", "4"},
        "CPU-based default restored",
    )
