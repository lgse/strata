# SPDX-License-Identifier: MIT
"""Minimal smoke: enter/leave, hjkl, yank/paste and the accessible footer /.

Each view keeps native directory/search-hit focus routes in those smoke launches.
Three extra scenarios own boundaries controller-signal GTK tests cannot observe:
external default-handler launch counts, typed rename/create submission and error
focus walks, and native Go completion/chord routing. GTK owns detailed selection,
privacy, lifecycle, history, sidebar, chooser and reference behavior.
"""

from __future__ import annotations

import pytest

from harness.modes import ALL_MODES

ROOT_ENTRIES = ["archive", "documents", "pictures", "readme.md", "todo.txt"]
OPEN_OR_PREVIEW_KEYS = ("l", "Right", "KP_Right")
PARENT_KEYS = ("h", "Left", "BackSpace")
ACTIVATE_KEYS = ("Return", "o")


@pytest.fixture
def launch_counter(test_environment):
    """Record the real default handler's file arguments."""

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
    yield launches
    print(f"default-handler arguments: {launches.read_text().splitlines() if launches.exists() else []!r}")


def footer_prompt(strata, kind: str):
    field = strata.editable_field()
    label = field.name or field.text or ""
    assert "Minimal mode" in label and kind in label, label
    assert "focused" in field.states
    return field


def focused_editable(strata):
    return strata.window.find(role="text", states={"editable", "focused"})


def move_focus_to(strata, name: str) -> None:
    """Move among listing names. Icons uses l because tiles sit on a row."""

    names = strata.entry_names()
    assert name in names
    strata.wait(lambda: strata.focused_name() is not None, "a focused listing row")
    if strata.view_mode() == "Icons":
        strata.keyboard.press("Home")
        strata.wait(lambda: strata.focused_name() is not None, "Icons Home")
        for _ in range(len(names) + 2):
            if strata.focused_name() == name:
                return
            previous = strata.focused_name()
            strata.keyboard.press("l")
            strata.wait(
                lambda: strata.focused_name() != previous,
                f"Icons l to move past {previous!r}",
            )
        raise AssertionError(f"Icons l never reached {name!r}")
    target = names.index(name)
    for _ in range(len(names) + 1):
        current = strata.focused_name()
        if current == name:
            return
        assert current in names, f"focus left the listing at {current!r}"
        strata.keyboard.press("j" if names.index(current) < target else "k")
        strata.wait(
            lambda: strata.focused_name() != current,
            f"listing focus to move from {current!r} toward {name!r}",
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
    if strata.focused_name() is None:
        strata.keyboard.press("Escape")
    if strata.focused_name() is None:
        names = strata.entry_names()
        assert names, "listing still has rows after the error dialog"
        strata.select_entry(names[0])
    strata.wait(lambda: strata.focused_name() is not None, "file focus after error dismissal")


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
    strata.wait(lambda: field.text == text, f"the {kind} prompt to hold {text!r}")
    strata.keyboard.press("Return")


def search_hit_row(strata, name: str):
    # Icons presents hits as table cells; List and Columns use list items.
    return strata.search_result(name)


def focus_search_hit(strata, name: str) -> None:
    strata.wait(lambda: search_hit_row(strata, name), f"the search hit {name!r}")
    for _ in range(12):
        row = search_hit_row(strata, name)
        if row is not None and (row.has_state("focused") or strata.focused_name() == name):
            return
        previous = strata.focused_name()
        strata.keyboard.press("j")
        strata.wait(
            lambda: strata.focused_name() != previous
            or (search_hit_row(strata, name) is not None
                and search_hit_row(strata, name).has_state("focused")),
            f"search cursor to move toward {name!r}",
        )
    raise AssertionError(f"j never focused search hit {name!r}")


def search_and_keep(strata, query: str, expected: str):
    strata.keyboard.press("s")
    field = footer_prompt(strata, "search")
    strata.keyboard.type_text(query)
    strata.wait(lambda: expected in strata.matches(), f"s {query!r} to list {expected!r}")
    assert field.text == query
    assert "focused" in field.states, "results must not steal the footer's typing focus"
    strata.keyboard.press("Escape")
    strata.wait(
        lambda: focused_editable(strata) is None and expected in strata.matches(),
        "first Esc to keep search hits and close the prompt",
    )
    assert strata.dialog() is None


@pytest.mark.preferences(minimal_mode=True, type_to_search=True)
@pytest.mark.parametrize("mode", ALL_MODES)
def test_minimal_hjkl_moves_without_filtering(strata, mode):
    strata.switch_view(mode)
    root = strata.fixture.root.name
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
    # Icons aliases native tile motion: adjacent names sit on one row, so l/h
    # reverse. Columns and List stay linear on j/k.
    forward, back = ("l", "h") if mode == "Icons" else ("j", "k")
    strata.keyboard.press(forward)
    strata.wait_for_focused_entry("todo.txt")
    assert strata.entry_names() == ROOT_ENTRIES
    strata.keyboard.press(back)
    strata.wait_for_focused_entry("readme.md")
    assert strata.entry_names() == ROOT_ENTRIES
    assert focused_editable(strata) is None

    strata.keyboard.press("space")
    strata.wait_for_focused_entry("todo.txt")
    strata.wait(lambda: "readme.md" in strata.selected_names(), "Space to retain readme.md")
    assert strata.preview() is None
    assert strata.entry_names() == ROOT_ENTRIES
    strata.keyboard.press("Escape")

    enter_leave = () if mode == "Icons" else tuple(
        zip(OPEN_OR_PREVIEW_KEYS, PARENT_KEYS, strict=True)
    )
    for enter, leave in enter_leave:
        move_focus_to(strata, "documents")
        assert strata.current_directory() == root
        strata.keyboard.press(enter)
        strata.wait_for_directory("documents")
        strata.entry("notes.txt", directory="documents")
        strata.wait_for_focused_entry("notes.txt")
        assert strata.preview() is None
        strata.keyboard.press(leave)
        strata.wait_for_directory(root)
        strata.wait_for_entries(ROOT_ENTRIES)
        if mode == "Columns":
            strata.wait(lambda: strata.pane_names() == [root], "the Miller child to close")
        else:
            strata.wait_for_focused_entry("documents")
        assert focused_editable(strata) is None

    if mode != "Icons":
        for key in ACTIVATE_KEYS:
            move_focus_to(strata, "documents")
            strata.keyboard.press(key)
            strata.wait_for_directory("documents")
            strata.entry("notes.txt", directory="documents")
            strata.wait_for_focused_entry("notes.txt")
            assert strata.preview() is None
            strata.keyboard.press("h")
            strata.wait_for_directory(root)

    parent_keys = ("BackSpace",) if mode == "Icons" else PARENT_KEYS
    for key in parent_keys:
        strata.open_directory("documents")
        strata.entry("notes.txt", directory="documents")
        assert focused_editable(strata) is None
        strata.keyboard.press(key)
        strata.wait_for_directory(root)
        strata.wait_for_entries(ROOT_ENTRIES)
        if mode == "Columns":
            strata.wait(lambda: strata.pane_names() == [root], "pointer-opened Miller child to close")
        else:
            strata.wait_for_focused_entry("documents")
        assert focused_editable(strata) is None


@pytest.mark.preferences(minimal_mode=True, filter_include_subfolders=False)
@pytest.mark.parametrize("mode", ALL_MODES)
def test_minimal_enter_and_leave_round_trip(strata, mode):
    strata.switch_view(mode)
    root = strata.fixture.root.name
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
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

    search_and_keep(strata, "photo", "photo.txt")
    assert "archive" not in strata.matches()
    focus_search_hit(strata, "photo.txt")
    preview_keys = ("i",) if mode == "Icons" else ("l", "Right")
    for key in preview_keys:
        strata.keyboard.press(key)
        strata.wait(lambda: strata.preview_shows("photo"), f"{key} previews the native hit row")
        assert strata.dialog() is None
        strata.wait_for_directory(root)
        if mode != "Icons":
            strata.keyboard.press(key)
            assert strata.preview_shows("photo"), f"repeating {key} keeps preview"
            strata.keyboard.press("h")
            strata.wait_for_focused_entry("photo.txt")
            assert strata.preview_shows("photo")
        assert strata.matches() == ["photo.txt"]
        strata.keyboard.press("i")
        strata.wait(lambda: strata.preview() is None, "i closes hit preview")
    strata.keyboard.press("Escape")
    strata.wait_for_entries(ROOT_ENTRIES)
    strata.select_entry("readme.md")
    search_and_keep(strata, "documents", "documents")
    focus_search_hit(strata, "documents")
    strata.keyboard.press("Return" if mode == "Icons" else "l")
    strata.wait_for_directory("documents")
    strata.entry("notes.txt", directory="documents")
    strata.wait_for_focused_entry("notes.txt")
    assert strata.preview() is None
    assert strata.dialog() is None
    strata.keyboard.press("BackSpace" if mode == "Icons" else "h")
    strata.wait_for_directory(root)

    search_and_keep(strata, "notes", "notes.txt")
    strata.keyboard.press("q")
    strata.wait(
        lambda: strata.environment.read_preferences().get("minimal_mode") == "false"
        and strata.entry_names() == ROOT_ENTRIES,
        "leaving minimal mode discards retained recursive results",
    )
    strata.keyboard.press("ctrl+f")
    field = strata.editable_field()
    assert "Minimal mode" not in (field.name or "")
    assert field.text == ""
    strata.keyboard.type_text("txt")
    strata.wait(lambda: strata.matches() == ["todo.txt"], "default Ctrl+F respects saved local scope")
    assert field.text == "txt"


@pytest.mark.preferences(minimal_mode=True)
def test_minimal_yank_paste_copies_into_subdirectory(strata):
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
    strata.keyboard.press("y")
    strata.open_directory("archive")
    strata.keyboard.press("p")
    strata.wait(
        lambda: strata.fixture.path("archive/readme.md").exists(),
        "the yanked file to arrive in archive",
    )
    assert strata.fixture.path("archive/readme.md").read_bytes() == strata.fixture.path("readme.md").read_bytes()


@pytest.mark.preferences(minimal_mode=True)
@pytest.mark.parametrize("mode", ALL_MODES)
def test_minimal_footer_find_jumps_without_filtering(strata, mode):
    strata.switch_view(mode)
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
    strata.keyboard.press("/")
    field = footer_prompt(strata, "/")
    strata.keyboard.type_text("todo")
    strata.wait(lambda: "todo.txt" in strata.selected_names(), "find selects todo.txt without taking focus")
    assert field.text == "todo" and "focused" in field.states
    assert strata.entry_names() == ROOT_ENTRIES
    strata.keyboard.press("Return")
    strata.wait_for_focused_entry("todo.txt")
    strata.keyboard.press("n")
    strata.wait_for_focused_entry("todo.txt")
    assert strata.entry_names() == ROOT_ENTRIES
    strata.keyboard.press("Escape")
    strata.keyboard.press("ctrl+f")
    assert focused_editable(strata) is None
    assert strata.entry_names() == ROOT_ENTRIES
    strata.keyboard.press("f")
    field = footer_prompt(strata, "filter")
    strata.keyboard.type_text("todo")
    strata.wait(lambda: field.text == "todo" and strata.matches() == ["todo.txt"], "native f filters to todo.txt")
    strata.keyboard.press("Return")
    strata.wait(
        lambda: focused_editable(strata) is None and strata.matches() == ["todo.txt"],
        "Enter keeps the footer filter",
    )
    assert strata.dialog() is None
    strata.keyboard.press("Escape")
    strata.wait_for_entries(ROOT_ENTRIES)


@pytest.mark.preferences(minimal_mode=True, type_to_search=True)
def test_minimal_enter_and_o_open_a_file(launch_counter, strata):
    root = strata.fixture.root.name
    expected = strata.fixture.path("todo.txt")
    strata.select_entry("todo.txt")
    strata.wait_for_focused_entry("todo.txt")
    for key in OPEN_OR_PREVIEW_KEYS:
        strata.keyboard.press(key)
        strata.wait(lambda: strata.preview_shows("todo"), f"{key} previews todo.txt")
        strata.wait_for_directory(root)
        assert strata.entry_names() == ROOT_ENTRIES
        assert not launch_counter.exists()
        strata.keyboard.press(key)
        assert strata.preview_shows("todo")
        strata.keyboard.press("h")
        strata.wait_for_focused_entry("todo.txt")
        assert strata.preview_shows("todo")
        strata.keyboard.press("i")
        strata.wait(lambda: strata.preview() is None, "i closes preview without launching")
    strata.keyboard.press("i")
    strata.wait(lambda: strata.preview_shows("todo"), "i opens preview without taking keyboard ownership")
    strata.keyboard.press("i")
    strata.wait(lambda: strata.preview() is None, "i toggles preview off")
    assert not launch_counter.exists()
    assert strata.entry_names() == ROOT_ENTRIES

    for count, key in enumerate(ACTIVATE_KEYS, start=1):
        strata.wait_for_focused_entry("todo.txt")
        strata.keyboard.press(key)
        def launched_once():
            received = launch_counter.read_text().splitlines() if launch_counter.exists() else []
            assert len(received) <= count, f"{key}: expected {count} total launches, got {received!r}"
            return len(received) == count

        strata.wait(launched_once, f"{key} launches exactly once")
        strata.wait_for_directory(root)
        assert strata.preview() is None
        received = launch_counter.read_text().splitlines()
        assert len(received) == count
        assert all(argument in (str(expected), expected.as_uri()) for argument in received)
    assert len(launch_counter.read_text().splitlines()) == 2


@pytest.mark.preferences(minimal_mode=True)
def test_minimal_rename_and_create_use_the_footer(strata):
    strata.select_entry("todo.txt")
    strata.wait_for_focused_entry("todo.txt")
    submit_footer(strata, "r", "renamed.txt", "rename")
    strata.wait(lambda: strata.fixture.path("renamed.txt").exists(), "r renames the file on disk")
    assert not strata.fixture.path("todo.txt").exists()
    assert strata.fixture.path("renamed.txt").read_text() == "todo\n"
    strata.wait(lambda: strata.selected_names() == ["renamed.txt"], "the renamed file remains selected")
    assert focused_editable(strata) is None
    strata.keyboard.press("k")
    strata.wait_for_focused_entry("readme.md")
    strata.keyboard.press("j")
    strata.wait_for_focused_entry("renamed.txt")

    for text, name, folder in (("created.txt", "created.txt", False), ("newdir/", "newdir", True)):
        submit_footer(strata, "a", text, "new")
        path = strata.fixture.path(name)
        strata.wait(lambda: path.is_dir() if folder else path.is_file(), f"a creates exact {text!r}")
        strata.wait_for_focused_entry(name)
        assert focused_editable(strata) is None
        assert strata.current_directory() == strata.fixture.root.name
        submit_footer(strata, "a", text, "new")
        dismiss_error_dialog(strata)
        assert not strata.fixture.path(f"{name} (1)").exists()
        assert path.is_dir() if folder else path.is_file()


@pytest.mark.preferences(minimal_mode=True)
def test_minimal_g_then_h_goes_home(strata):
    home = strata.environment.home
    (home / "minimal-home.txt").write_text("home\n")
    strata.select_entry("readme.md")
    strata.wait_for_focused_entry("readme.md")
    strata.keyboard.press("g")
    wait_for_go_chord(strata)
    strata.keyboard.press("q")
    strata.wait_for_focused_entry("readme.md")
    strata.wait_for_directory(strata.fixture.root.name)
    strata.wait_for_entries(ROOT_ENTRIES)
    assert strata.environment.read_preferences().get("minimal_mode") == "true"

    strata.keyboard.press("g")
    wait_for_go_chord(strata)
    strata.keyboard.press("space")
    field = footer_prompt(strata, "go ›")
    strata.keyboard.type_text(str(strata.fixture.root / "doc"))
    strata.keyboard.press("Tab")
    strata.wait(lambda: field.text == str(strata.fixture.path("documents")), "asynchronous Go completion")
    assert "focused" in field.states
    strata.keyboard.press("Return")
    strata.wait_for_directory("documents")
    strata.wait_for_focused_entry("notes.txt")
    strata.keyboard.press("g")
    wait_for_go_chord(strata)
    strata.keyboard.press("h")
    strata.wait_for_directory(home.name)
    strata.entry("minimal-home.txt")
    assert strata.current_directory() != strata.fixture.root.name
