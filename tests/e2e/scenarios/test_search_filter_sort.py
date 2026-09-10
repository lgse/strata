# SPDX-License-Identifier: MIT
"""Type-to-search, pane filtering, sorting, and hidden files."""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES, SINGLE_PANE_MODES

ROOT_ENTRIES = ["archive", "documents", "pictures", "readme.md", "todo.txt"]
# Folders stay grouped first, so descending is not simply the reverse.
ROOT_ENTRIES_DESCENDING = ["pictures", "documents", "archive", "todo.txt", "readme.md"]
DOUBLE_CLICK_PREFERENCES = {
    "list_folder_clicks": 2,
    "list_file_clicks": 2,
    "grid_folder_clicks": 2,
    "grid_file_clicks": 2,
    "explorer_folder_clicks": 2,
    "explorer_file_clicks": 2,
    "single_click_previews": True,
}
DOUBLE_CLICK = pytest.mark.preferences(**DOUBLE_CLICK_PREFERENCES)


@pytest.fixture
def launch_counter(test_environment):
    applications = test_environment.data_home / "applications"
    applications.mkdir()
    launches = test_environment.root / "filtered-launches"
    launcher = test_environment.root / "record-filtered-launch"
    launcher.write_text(f'#!/bin/sh\nprintf "%s\\n" "$@" >> "{launches}"\n')
    launcher.chmod(0o755)
    (applications / "strata-filtered.desktop").write_text(
        "[Desktop Entry]\nType=Application\nName=Filtered Result Viewer\n"
        f"Exec={launcher} %U\nMimeType=text/csv;\nNoDisplay=true\n"
    )
    (test_environment.config_home / "mimeapps.list").write_text(
        "[Default Applications]\ntext/csv=strata-filtered.desktop;\n"
        "[Added Associations]\ntext/csv=strata-filtered.desktop;\n"
    )
    return launches


@pytest.fixture
def root(strata) -> str:
    return strata.fixture.root.name


@pytest.mark.parametrize("mode", ALL_MODES)
def test_type_to_search_finds_matches_anywhere_in_the_tree(strata, mode, root):
    strata.select_entry("readme.md", directory=root)

    strata.keyboard.type_text("photo")

    field = strata.editable_field()
    strata.wait(lambda: field.text == "photo", "the typed query to reach the search box")
    strata.wait(
        lambda: strata.matches(root) == ["photo.txt"],
        "the search to list the nested match",
    )

    strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.entry_names(root) == ROOT_ENTRIES,
        "Escape to restore the directory listing",
    )


@pytest.mark.parametrize("mode", ALL_MODES)
def test_filtering_a_pane_narrows_the_listing(strata, mode, root):
    strata.select_entry("readme.md", directory=root)

    strata.keyboard.press("ctrl+f")
    field = strata.editable_field()
    strata.keyboard.type_text("spreadsheet")
    strata.wait(lambda: field.text == "spreadsheet", "the filter query to be typed")

    strata.wait(
        lambda: strata.matches(root) == ["spreadsheet.csv"],
        "the filter to list only matching entries",
    )

    strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.entry_names(root) == ROOT_ENTRIES,
        "Escape to restore the full listing",
    )


def assert_filtered_result_opens(strata, activation):
    strata.select_entry("documents")
    strata.keyboard.press("ctrl+f")
    field = strata.editable_field()
    strata.keyboard.type_text("documents")
    strata.wait(lambda: field.text == "documents", "the filter query")
    result = strata.wait(
        lambda: strata.window.find(role="list item", name="documents"),
        "the filtered folder result",
    )

    if activation == "click":
        strata.pointer.click(result)
    else:
        strata.keyboard.press("Down")
        strata.keyboard.press("Return")

    strata.wait_for_directory("documents")


@pytest.mark.preferences(
    **DOUBLE_CLICK_PREFERENCES, filter_include_subfolders=False
)
@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("activation", ["click", "enter"])
def test_local_filtered_results_open_with_one_activation(strata, mode, activation):
    assert_filtered_result_opens(strata, activation)


@DOUBLE_CLICK
@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("activation", ["click", "enter"])
def test_recursive_filtered_results_open_with_one_activation(strata, mode, activation):
    assert_filtered_result_opens(strata, activation)


@DOUBLE_CLICK
@pytest.mark.parametrize("mode", SINGLE_PANE_MODES)
def test_recursive_file_double_click_launches_once(launch_counter, strata, mode):
    strata.keyboard.press("ctrl+f")
    field = strata.editable_field()
    strata.keyboard.type_text("spreadsheet")
    result = strata.wait(
        lambda: strata.window.find(role="list item", name="spreadsheet.csv"),
        "the recursive file result",
    )

    strata.pointer.double_click(result)
    strata.wait(
        lambda: launch_counter.exists() and len(launch_counter.read_text().splitlines()) >= 1,
        "the file launch",
    )
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("photo")
    strata.wait(lambda: field.text == "photo", "the follow-up query")
    strata.wait(
        lambda: strata.window.find(role="list item", name="photo.txt") is not None,
        "the follow-up results",
    )
    assert len(launch_counter.read_text().splitlines()) == 1


@pytest.mark.preferences(filter_include_subfolders=False)
@pytest.mark.parametrize("mode", ALL_MODES)
def test_directory_only_filter_matches_immediate_files_and_folders(strata, mode, root):
    strata.select_entry("readme.md", directory=root)
    strata.keyboard.press("ctrl+f")
    field = strata.editable_field()
    for query, expected in [
        ("txt", ["todo.txt"]),
        ("photo", []),
        ("archive", ["archive"]),
    ]:
        strata.keyboard.press("ctrl+a")
        strata.keyboard.type_text(query)
        strata.wait(lambda: field.text == query, "the filter query to be typed")
        strata.wait(
            lambda: strata.matches(root) == expected,
            f"directory-only matches for {query}",
        )
    strata.keyboard.press("Escape")
    strata.wait(lambda: strata.entry_names(root) == ROOT_ENTRIES, "the listing to return")


def test_dismissing_a_filter_keeps_hidden_files_hidden(strata, root):
    """A cleared query must not also clear the dotfile filter."""

    assert ".hidden.txt" not in strata.entry_names(root)

    strata.select_entry("readme.md", directory=root)
    strata.keyboard.press("ctrl+f")
    strata.editable_field()
    strata.keyboard.type_text("readme")
    strata.wait(
        lambda: strata.matches(root) == ["readme.md"],
        "the filter to narrow the listing",
    )
    strata.keyboard.press("Escape")

    strata.wait(
        lambda: strata.entry_names(root) == ROOT_ENTRIES,
        "the listing to come back without hidden files",
    )


def test_hidden_files_toggle(strata, root):
    assert ".hidden.txt" not in strata.entry_names(root)

    strata.keyboard.press("ctrl+h")

    strata.wait(
        lambda: ".hidden.txt" in strata.entry_names(root),
        "Ctrl+H to reveal hidden files",
    )

    strata.keyboard.press("ctrl+h")
    strata.wait(
        lambda: ".hidden.txt" not in strata.entry_names(root),
        "Ctrl+H to hide them again",
    )


def test_reversing_the_sort_direction(strata, root):
    assert strata.entry_names(root) == ROOT_ENTRIES

    strata.pointer.click(strata.header_button("Ascending — click to reverse"))

    strata.wait(
        lambda: strata.entry_names(root) == ROOT_ENTRIES_DESCENDING,
        "the pane to sort descending with folders still grouped first",
    )

    strata.pointer.click(strata.header_button("Descending — click to reverse"))
    strata.wait(
        lambda: strata.entry_names(root) == ROOT_ENTRIES,
        "the pane to sort ascending again",
    )


def test_sorting_by_size_reorders_the_files(strata, root):
    strata.pointer.click(strata.header_button("Choose sort field"))
    strata.pointer.click(
        strata.wait(
            lambda: strata.window.find(role="button", name="Size"),
            "the Size sort option",
        )
    )

    strata.wait(
        lambda: strata.entry_names(root)[-2:] == ["todo.txt", "readme.md"],
        "the files to be ordered by size",
    )


def test_global_search_finds_a_file_under_home(strata, root):
    """Ctrl+K searches the home directory, not the browsed location."""

    nested = strata.environment.home / "reports" / "quarterly-summary.txt"
    nested.parent.mkdir(parents=True, exist_ok=True)
    nested.write_text("summary\n")

    strata.keyboard.press("ctrl+k")
    field = strata.editable_field()
    strata.keyboard.type_text("quarterly")
    strata.wait(lambda: field.text == "quarterly", "the query to be typed")

    strata.wait(
        lambda: any(
            node.name == "quarterly-summary.txt"
            for node in strata.window.find_all(role="label")
        ),
        "the file under home to appear in the search results",
    )

    strata.keyboard.press("Escape")
    strata.wait(
        lambda: strata.window.find(role="text", states={"editable"}) is None,
        "Escape to close the search palette",
    )
