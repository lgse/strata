# SPDX-License-Identifier: MIT
"""Minimal mode: distinctive keymap interactions through the launched app.

Keeps enter/leave (`q` / Ctrl+Shift+M), `hjkl` with type-to-search on,
yank-paste, and footer `/` find. New cases drive parent motion, `l`/Right
directory vs preview, Enter/`o` activate, footer `f`/`s`/`r`/`a`, Space vs
`i`, and `g` chords.

Rust GTK tests remain the owner for chooser chrome
(`ui::chooser::tests::minimal_chrome`), two-window teardown
(`ui::window::composition::tests::minimal_mode_*`), unyank/foreign clipboard,
empty-folder flash verbs, visual `v`/`V`, `z`/`Z` history, pin `g 1`–`9`,
Settings/F1/`~`, and context-menu hint labels.
"""

from __future__ import annotations

import pytest

from harness.browser import SEARCH_RESULTS_LABEL
from harness.modes import ALL_MODES

ROOT_ENTRIES = ["archive", "documents", "pictures", "readme.md", "todo.txt"]
OPEN_OR_PREVIEW_KEYS = ("l", "Right", "KP_Right")
PARENT_KEYS = ("h", "Left", "BackSpace")
ACTIVATE_KEYS = ("Return", "o")


@pytest.fixture
def launch_counter(test_environment):
    """Record default-handler launches so Enter/`o` can open a real file."""

    applications = test_environment.data_home / "applications"
    applications.mkdir()
    launches = test_environment.root / "minimal-launches"
    launcher = test_environment.root / "record-minimal-launch"
    launcher.write_text(f'#!/bin/sh\nprintf "%s\\n" "$@" >> "{launches}"\n')
    launcher.chmod(0o755)
    (applications / "strata-minimal.desktop").write_text(
        "[Desktop Entry]\nType=Application\nName=Minimal Result Viewer\n"
        f"Exec={launcher} %U\nMimeType=text/plain;\nNoDisplay=true\n"
    )
    (test_environment.config_home / "mimeapps.list").write_text(
        "[Default Applications]\ntext/plain=strata-minimal.desktop;\n"
        "[Added Associations]\ntext/plain=strata-minimal.desktop;\n"
    )
    return launches


def footer_prompt(strata, kind: str | None = None):
    field = strata.editable_field()
    label = field.name or field.text or ""
    assert "Minimal mode" in label, f"expected the footer prompt, got {label!r}"
    if kind is not None:
        assert kind in label, f"expected footer {kind!r}, got {label!r}"
    return field


def focused_editable(strata):
    return strata.window.find(role="text", states={"editable", "focused"})


def move_focus_to(strata, name: str) -> None:
    """Move listing focus with j/k so Right is not stolen as activate."""

    names = strata.entry_names()
    if name not in names:
        raise AssertionError(f"{name!r} is not in {names}")
    strata.wait(lambda: strata.focused_name() is not None, "a focused listing row")
    target = names.index(name)
    for _ in range(len(names) + 1):
        current = strata.focused_name()
        if current == name:
            return
        if current not in names:
            raise AssertionError(f"focus left the listing at {current!r}")
        key = "j" if names.index(current) < target else "k"
        strata.keyboard.press(key)
        strata.wait(
            lambda: strata.focused_name() != current, "listing focus to move"
        )
    raise AssertionError(f"j/k never reached {name!r}")


def dismiss_error_dialog(strata) -> None:
    dialog = strata.wait_for_dialog()
    close = dialog.find(role="button", name="Close")
    if close is not None:
        strata.pointer.click(close)
    else:
        strata.keyboard.press("Escape")
    strata.wait(lambda: strata.dialog() is None, "the error dialog to close")


def wait_for_go_chord(strata) -> None:
    strata.wait(
        lambda: strata.window.find(role="label", name="g-") is not None
        or strata.window.find(name="Go destinations") is not None,
        "the g chord to arm",
    )


def submit_footer(strata, key: str, text: str, kind: str):
    strata.keyboard.press(key)
    field = footer_prompt(strata, kind)
    if field.text:
        strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text(text)
    strata.wait(lambda: text in field.text, f"the {kind} prompt to hold {text!r}")
    strata.keyboard.press("Return")
    return field


def type_footer_query(strata, text: str) -> None:
    """Type into the footer one character at a time.

    Live search rebuilds the results list and can steal focus; re-acquire the
    prompt before each character so later keys are not browse verbs.
    """

    for character in text:
        field = focused_editable(strata)
        if field is None or "Minimal mode" not in (field.name or ""):
            field = strata.wait(
                lambda: strata.window.find(
                    role="text", name_matches="Minimal mode"
                ),
                "the footer prompt to stay available",
            )
            strata.pointer.click(field)
            field = footer_prompt(strata)
        strata.keyboard.type_text(character)


def search_hit_row(strata, name: str):
    results = strata.window.find(role="list", name=SEARCH_RESULTS_LABEL)
    nodes = (
        results.find_all(role="list item") if results is not None else strata.entries()
    )
    for node in nodes:
        if node.name == name:
            return node
    return None


def focus_search_hit(strata, name: str) -> None:
    strata.wait(lambda: search_hit_row(strata, name), f"the search hit {name!r}")
    for _ in range(12):
        row = search_hit_row(strata, name)
        if row is not None and (
            row.has_state("focused") or strata.focused_name() == name
        ):
            return
        previous = strata.focused_name()
        strata.keyboard.press("j")
        strata.wait(
            lambda: strata.focused_name() != previous
            or (
                search_hit_row(strata, name) is not None
                and search_hit_row(strata, name).has_state("focused")
            ),
            "search listing focus to move",
        )
    raise AssertionError(f"j never focused search hit {name!r}")


def search_and_keep(strata, query: str, expected: str):
    strata.keyboard.press("s")
    footer_prompt(strata, "search")
    type_footer_query(strata, query)
    strata.wait(
        lambda: expected in strata.matches(),
        f"s to list {expected!r}",
    )
    strata.keyboard.press("Escape")
    strata.wait(
        lambda: focused_editable(strata) is None and expected in strata.matches(),
        "first Esc to keep search hits and close the prompt",
    )


@pytest.mark.preferences(minimal_mode=True, type_to_search=True)
@pytest.mark.parametrize("mode", ALL_MODES)
def test_minimal_hjkl_moves_without_filtering(strata, mode):
    strata.switch_view(mode)
    assert strata.entry_names() == ROOT_ENTRIES
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
    strata.keyboard.press("j")
    strata.wait_for_focused_entry("todo.txt")
    assert strata.entry_names() == ROOT_ENTRIES
    strata.keyboard.press("k")
    strata.wait_for_focused_entry("readme.md")
    assert strata.entry_names() == ROOT_ENTRIES
    assert focused_editable(strata) is None


@pytest.mark.preferences(minimal_mode=True)
@pytest.mark.parametrize("mode", ALL_MODES)
def test_minimal_enter_and_leave_round_trip(strata, mode):
    strata.switch_view(mode)
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
    # `q` leaves the mode; toggling back re-enters it.
    strata.keyboard.press("q")
    strata.wait(
        lambda: strata.environment.read_preferences().get("minimal_mode") == "false",
        "q to leave minimal mode",
    )
    strata.wait_for_focused_entry("readme.md")
    assert strata.entry_names() == ROOT_ENTRIES
    strata.keyboard.press("ctrl+shift+m")
    strata.wait(
        lambda: strata.environment.read_preferences().get("minimal_mode") == "true",
        "Ctrl+Shift+M to re-enter minimal mode",
    )
    strata.wait_for_focused_entry("readme.md")


@pytest.mark.preferences(minimal_mode=True)
def test_minimal_yank_paste_copies_into_subdirectory(strata):
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
    strata.keyboard.press("y")
    strata.open_directory("archive")
    strata.keyboard.press("p")
    strata.wait(
        lambda: strata.fixture.path("archive/readme.md").exists(),
        "the yanked file to land in the pasted directory",
    )
    assert strata.fixture.path("readme.md").exists()


@pytest.mark.preferences(minimal_mode=True)
@pytest.mark.parametrize("mode", ALL_MODES)
def test_minimal_footer_find_jumps_without_filtering(strata, mode):
    strata.switch_view(mode)
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
    strata.keyboard.press("/")
    field = strata.editable_field()
    assert "Minimal mode" in (field.name or field.text or "")
    strata.keyboard.type_text("todo")
    strata.wait(
        lambda: "todo.txt" in strata.selected_names(),
        "find to select todo.txt without grabbing list focus",
    )
    field = strata.editable_field()
    assert "Minimal mode" in (field.name or field.text or "")
    assert strata.entry_names() == ROOT_ENTRIES
    assert strata.fixture.path("documents").exists()
    strata.keyboard.press("Return")
    strata.wait_for_focused_entry("todo.txt")
    strata.keyboard.press("n")
    strata.wait_for_focused_entry("todo.txt")
    assert strata.entry_names() == ROOT_ENTRIES


@pytest.mark.preferences(minimal_mode=True, type_to_search=True)
@pytest.mark.parametrize("mode", ALL_MODES)
def test_minimal_parent_keys_leave_a_directory(strata, mode):
    strata.switch_view(mode)
    root = strata.fixture.root.name
    for key in PARENT_KEYS:
        strata.open_directory("documents")
        strata.wait_for_directory("documents")
        strata.keyboard.press(key)
        strata.wait_for_directory(root)
        strata.wait_for_entries(ROOT_ENTRIES)
        if mode == "Columns":
            strata.wait(
                lambda: strata.pane_names() == [root],
                "the Miller child pane to close",
            )
        else:
            strata.wait_for_focused_entry("documents")
        assert focused_editable(strata) is None


@pytest.mark.preferences(minimal_mode=True, type_to_search=True)
@pytest.mark.parametrize("mode", ALL_MODES)
def test_minimal_l_and_right_open_a_directory(strata, mode):
    strata.switch_view(mode)
    root = strata.fixture.root.name
    for key in OPEN_OR_PREVIEW_KEYS:
        strata.select_entry("readme.md")
        strata.wait_for_focused_entry("readme.md")
        strata.wait_for_directory(root)
        move_focus_to(strata, "documents")
        assert strata.current_directory() == root
        strata.keyboard.press(key)
        strata.wait_for_directory("documents")
        strata.entry("notes.txt", directory="documents")
        assert strata.preview() is None
        strata.keyboard.press("h")
        strata.wait_for_directory(root)


@pytest.mark.preferences(minimal_mode=True, type_to_search=True)
def test_minimal_l_and_right_preview_a_file(launch_counter, strata):
    root = strata.fixture.root.name
    strata.select_entry("todo.txt")
    strata.wait_for_focused_entry("todo.txt")
    for key in OPEN_OR_PREVIEW_KEYS:
        assert strata.preview() is None
        strata.keyboard.press(key)
        strata.wait(
            lambda: strata.preview_shows("todo"),
            f"{key} to preview the focused file",
        )
        strata.wait_for_directory(root)
        assert strata.entry_names() == ROOT_ENTRIES
        assert not launch_counter.exists()
        strata.keyboard.press(key)
        strata.wait(lambda: strata.preview() is None, f"{key} to close preview")
    assert not launch_counter.exists()


@pytest.mark.preferences(minimal_mode=True, type_to_search=True)
def test_minimal_enter_and_o_open_a_directory(strata):
    root = strata.fixture.root.name
    for key in ACTIVATE_KEYS:
        strata.select_entry("readme.md")
        strata.wait_for_focused_entry("readme.md")
        move_focus_to(strata, "documents")
        assert strata.current_directory() == root
        strata.keyboard.press(key)
        strata.wait_for_directory("documents")
        strata.entry("notes.txt", directory="documents")
        assert strata.preview() is None
        strata.keyboard.press("h")
        strata.wait_for_directory(root)


@pytest.mark.preferences(minimal_mode=True)
def test_minimal_enter_and_o_open_a_file(launch_counter, strata):
    root = strata.fixture.root.name
    expected = strata.fixture.path("todo.txt")
    for key in ACTIVATE_KEYS:
        before = (
            len(launch_counter.read_text().splitlines())
            if launch_counter.exists()
            else 0
        )
        strata.select_entry("todo.txt")
        strata.wait_for_focused_entry("todo.txt")
        assert strata.preview() is None
        strata.keyboard.press(key)
        strata.wait(
            lambda: launch_counter.exists()
            and len(launch_counter.read_text().splitlines()) > before,
            f"{key} to launch the focused file",
        )
        strata.wait_for_directory(root)
        assert strata.preview() is None
        assert expected.exists()
    received = launch_counter.read_text()
    assert str(expected) in received or expected.name in received


@pytest.mark.preferences(minimal_mode=True)
def test_minimal_footer_filter_hides_rows_and_ctrl_f_does_not(strata):
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
    strata.keyboard.press("ctrl+f")
    strata.wait(
        lambda: strata.focused_name() in ROOT_ENTRIES
        and strata.entry_names() == ROOT_ENTRIES,
        "Ctrl+F to keep the unfiltered listing",
    )
    assert focused_editable(strata) is None
    strata.keyboard.press("f")
    field = footer_prompt(strata, "filter")
    strata.keyboard.type_text("todo")
    strata.wait(lambda: field.text == "todo", "the filter query to be typed")
    strata.wait(
        lambda: strata.matches() == ["todo.txt"],
        "footer f to hide non-matching rows",
    )
    strata.keyboard.press("Return")
    strata.wait(
        lambda: focused_editable(strata) is None and strata.matches() == ["todo.txt"],
        "Enter to keep the footer filter",
    )
    assert strata.dialog() is None
    strata.keyboard.press("Escape")
    strata.wait_for_entries(ROOT_ENTRIES)


@pytest.mark.preferences(minimal_mode=True, filter_include_subfolders=False)
def test_minimal_search_keeps_hits_after_first_escape(strata):
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
    search_and_keep(strata, "photo", "photo.txt")
    assert strata.dialog() is None
    assert "archive" not in strata.matches()


@pytest.mark.preferences(minimal_mode=True, filter_include_subfolders=False)
@pytest.mark.parametrize("mode", ALL_MODES)
def test_minimal_search_hit_l_and_right(strata, mode):
    strata.switch_view(mode)
    root = strata.fixture.root.name
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
    search_and_keep(strata, "photo", "photo.txt")
    focus_search_hit(strata, "photo.txt")
    for key in ("l", "Right"):
        strata.keyboard.press(key)
        strata.wait(
            lambda: strata.preview_shows("photo"),
            f"{key} to preview the search-hit file",
        )
        assert strata.dialog() is None
        strata.wait_for_directory(root)
        strata.keyboard.press(key)
        strata.wait(lambda: strata.preview() is None, f"{key} to close the hit preview")
    strata.keyboard.press("Escape")
    strata.wait_for_directory(root)
    strata.wait_for_entries(ROOT_ENTRIES)

    strata.select_entry("readme.md")
    search_and_keep(strata, "documents", "documents")
    focus_search_hit(strata, "documents")
    strata.keyboard.press("l")
    strata.wait_for_directory("documents")
    strata.entry("notes.txt", directory="documents")
    assert strata.preview() is None
    assert strata.dialog() is None


@pytest.mark.preferences(minimal_mode=True)
def test_minimal_rename_uses_the_footer(strata):
    strata.select_entry("todo.txt")
    strata.wait_for_focused_entry("todo.txt")
    strata.keyboard.press("r")
    field = footer_prompt(strata, "rename")
    strata.keyboard.press("ctrl+a")
    strata.keyboard.type_text("renamed.txt")
    strata.wait(lambda: field.text == "renamed.txt", "the new name to be typed")
    strata.keyboard.press("Return")
    strata.wait(
        lambda: strata.fixture.path("renamed.txt").exists(),
        "r to rename the focused file on disk",
    )
    assert not strata.fixture.path("todo.txt").exists()
    assert strata.fixture.path("renamed.txt").read_text() == "todo\n"
    strata.entry("renamed.txt")
    strata.wait(
        lambda: focused_editable(strata) is None,
        "the rename prompt to close",
    )


@pytest.mark.preferences(minimal_mode=True)
def test_minimal_create_file_and_folder_without_uniquifying(strata):
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
    submit_footer(strata, "a", "created.txt", "new")
    strata.wait(
        lambda: strata.fixture.path("created.txt").is_file(),
        "a to create the typed file on disk",
    )
    strata.wait(
        lambda: focused_editable(strata) is None,
        "the create prompt to close",
    )
    strata.entry("created.txt")

    submit_footer(strata, "a", "created.txt", "new")
    dismiss_error_dialog(strata)
    assert not strata.fixture.path("created.txt (1)").exists()
    assert strata.fixture.path("created.txt").is_file()

    submit_footer(strata, "a", "newdir/", "new")
    strata.wait(
        lambda: strata.fixture.path("newdir").is_dir(),
        "a with a trailing slash to create a folder",
    )
    strata.wait(
        lambda: focused_editable(strata) is None,
        "the folder create prompt to close",
    )
    strata.entry("newdir")
    assert strata.current_directory() == strata.fixture.root.name

    submit_footer(strata, "a", "newdir/", "new")
    dismiss_error_dialog(strata)
    assert not strata.fixture.path("newdir (1)").exists()
    assert strata.fixture.path("newdir").is_dir()


@pytest.mark.preferences(minimal_mode=True)
def test_minimal_space_toggles_selection_not_preview(strata):
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
    strata.keyboard.press("space")
    strata.wait_for_focused_entry("todo.txt")
    strata.wait(
        lambda: "readme.md" in strata.selected_names(),
        "Space to keep the previous row selected",
    )
    assert strata.preview() is None
    assert strata.entry_names() == ROOT_ENTRIES


@pytest.mark.preferences(minimal_mode=True)
def test_minimal_i_toggles_preview(strata):
    strata.select_entry("todo.txt")
    strata.wait_for_focused_entry("todo.txt")
    assert strata.preview() is None
    strata.keyboard.press("i")
    strata.wait(
        lambda: strata.preview_shows("todo"),
        "i to open the preview drawer",
    )
    strata.keyboard.press("i")
    strata.wait(lambda: strata.preview() is None, "i to close the preview drawer")
    assert strata.entry_names() == ROOT_ENTRIES


@pytest.mark.preferences(minimal_mode=True)
def test_minimal_g_then_h_goes_home(strata):
    home = strata.environment.home
    (home / "minimal-home.txt").write_text("home\n")
    strata.open_directory("documents")
    strata.wait_for_directory("documents")
    strata.keyboard.press("g")
    wait_for_go_chord(strata)
    strata.keyboard.press("h")
    strata.wait_for_directory(home.name)
    strata.entry("minimal-home.txt")
    assert strata.current_directory() != strata.fixture.root.name


@pytest.mark.preferences(minimal_mode=True)
def test_minimal_unknown_g_second_key_keeps_the_listing(strata):
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
    here = strata.current_directory()
    strata.keyboard.press("g")
    wait_for_go_chord(strata)
    strata.keyboard.press("q")
    strata.wait_for_focused_entry("readme.md")
    strata.wait_for_directory(here)
    strata.wait_for_entries(ROOT_ENTRIES)
    strata.wait(
        lambda: strata.environment.read_preferences().get("minimal_mode") == "true",
        "an unknown g chord not to leave the mode",
    )
