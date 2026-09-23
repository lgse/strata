"""Scratch repro: burst-capture the completion popover right after a keystroke."""

from harness.artifacts import artifact_root


def test_completion_popover_during_typing(strata):
    fixture = strata.fixture
    for name in ["alpha-one", "alpha-two", "alpha-three", "beta-dir"]:
        fixture.path(name).mkdir(exist_ok=True)

    strata.keyboard.press("ctrl+l")
    strata.editable_field()

    out = artifact_root() / "completion-flash"
    out.mkdir(parents=True, exist_ok=True)

    # Get the popover open and settled first.
    prefix = str(fixture.root) + "/a"
    strata.keyboard.type_text(prefix)

    # Burst-capture 40 frames immediately after one candidate-changing keystroke.
    strata.keyboard.type_text("l")
    for i in range(40):
        strata.screenshot(out / f"burst-{i:02d}.png")
