# SPDX-License-Identifier: GPL-3.0-or-later
"""Session and per-scenario fixtures for the end-to-end suite."""

from __future__ import annotations

import os
import signal
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

from harness import screenshots, tree  # noqa: E402
from harness.application import Application, build_binary  # noqa: E402
from harness.artifacts import ArtifactCollector  # noqa: E402
from harness.browser import Strata  # noqa: E402
from harness.display import HeadlessDisplay  # noqa: E402
from harness.environment import TestEnvironment  # noqa: E402
from harness.fixtures import FixtureTree  # noqa: E402
from harness.interaction import Keyboard, Pointer  # noqa: E402
from harness.xtest import XTestConnection  # noqa: E402

# Retry infrastructure startup only, never interaction assertions.
DISPLAY_START_ATTEMPTS = 2


def pytest_addoption(parser: pytest.Parser) -> None:
    parser.addoption(
        "--keep-artifacts",
        action="store_true",
        help="write the failure artifact bundle for passing scenarios too",
    )


@pytest.fixture(scope="session")
def strata_binary() -> Path:
    return build_binary()


@pytest.fixture(scope="session")
def headless_display() -> HeadlessDisplay:
    last_error: Exception | None = None
    for _ in range(DISPLAY_START_ATTEMPTS):
        display = HeadlessDisplay()
        try:
            display.start()
        except Exception as error:
            last_error = error
            ArtifactCollector(test_name="infrastructure").write("startup-error.txt", str(error))
            display.stop()
            continue
        try:
            # libatspi reads these once, on the first `Atspi.init()`.
            os.environ.update(display.environment)
            tree.connect()
            yield display
        finally:
            display.stop()
        return
    raise AssertionError(f"the headless display could not be started: {last_error!r}")


@pytest.fixture(scope="session")
def xtest(headless_display: HeadlessDisplay) -> XTestConnection:
    connection = XTestConnection(headless_display.display)
    try:
        yield connection
    finally:
        tree.set_surface_origin_provider(None)
        connection.close()


@pytest.fixture
def keyboard(xtest: XTestConnection) -> Keyboard:
    return Keyboard(xtest)


@pytest.fixture
def pointer(xtest: XTestConnection) -> Pointer:
    return Pointer(xtest)


@pytest.fixture
def fixture_tree() -> FixtureTree:
    """A fresh fixture tree per scenario."""

    tree_ = FixtureTree.create()
    try:
        yield tree_
    finally:
        tree_.cleanup()


@pytest.fixture
def test_environment() -> TestEnvironment:
    environment = TestEnvironment()
    try:
        yield environment
    finally:
        environment.cleanup()


@pytest.fixture
def preferences() -> dict[str, object]:
    """Overrides a scenario applies before the application starts.

    Parametrise it with `@pytest.mark.preferences(browser_mode="icons")`.
    """

    return {}


@pytest.fixture
def strata(
    request: pytest.FixtureRequest,
    strata_binary: Path,
    headless_display: HeadlessDisplay,
    test_environment: TestEnvironment,
    fixture_tree: FixtureTree,
    keyboard: Keyboard,
    pointer: Pointer,
) -> Strata:
    overrides: dict[str, object] = dict(request.getfixturevalue("preferences"))
    for marker in request.node.iter_markers("preferences"):
        overrides.update(marker.kwargs)
    test_environment.write_preferences(overrides)

    application = Application(
        display=headless_display,
        environment=test_environment,
        location=fixture_tree.root,
    )
    window = Strata(
        application=application,
        keyboard=keyboard,
        pointer=pointer,
        fixture=fixture_tree,
        environment=test_environment,
        display=headless_display,
    )
    failed = False
    try:
        application.start()
        yield window
    except BaseException:
        failed = True
        raise
    finally:
        try:
            if failed or request.node.stash.get(_REPORT_KEY, False) or request.config.getoption(
                "--keep-artifacts"
            ):
                _collect_artifacts(request, window)
        finally:
            application.stop()


_REPORT_KEY = pytest.StashKey[bool]()


def _collect_artifacts(request: pytest.FixtureRequest, window: Strata) -> None:
    collector = ArtifactCollector(test_name=request.node.name)
    collector.collect(
        display=window.display.display,
        accessibility_tree=window.application.diagnostics(),
        application_log=window.application.log(),
        fixture_listing=window.fixture.listing(),
        session_logs=window.display.logs(),
    )
    print(f"\nfailure artifacts: {collector.directory}")


_TERMINATION_HANDLER_KEY = pytest.StashKey[object]()


def _terminate_session(_number, _frame):
    raise KeyboardInterrupt("headless test session terminated")


def pytest_unconfigure(config: pytest.Config) -> None:
    previous = config.stash.get(_TERMINATION_HANDLER_KEY, None)
    if previous is not None:
        signal.signal(signal.SIGTERM, previous)


def pytest_configure(config: pytest.Config) -> None:
    config.stash[_TERMINATION_HANDLER_KEY] = signal.signal(signal.SIGTERM, _terminate_session)
    config.addinivalue_line(
        "markers", "preferences(**values): seed Strata preferences for a scenario"
    )
    config.addinivalue_line(
        "markers", "baseline: a scenario that compares against a golden screenshot"
    )


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_makereport(item: pytest.Item, call: pytest.CallInfo):
    outcome = yield
    report = outcome.get_result()
    if report.when == "call" and report.failed:
        item.stash[_REPORT_KEY] = True


@pytest.fixture
def baseline(request: pytest.FixtureRequest):
    """Compare a capture with its committed golden image."""

    def compare(window: Strata, name: str) -> screenshots.Comparison:
        collector = ArtifactCollector(test_name=request.node.name)
        actual = window.screenshot(collector.directory / f"{name}.capture.png")
        comparison = screenshots.compare_to_baseline(name, actual, collector.directory)
        assert comparison.matched, (
            comparison.summary
            + f"\nExpected, actual, and diff images are in {collector.directory}"
            + "\nRegenerate deliberately with STRATA_E2E_UPDATE_BASELINES=1."
        )
        return comparison

    return compare
