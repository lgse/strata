# SPDX-License-Identifier: MIT
"""Opening, reading, and closing the quick preview."""

from __future__ import annotations

import pytest
from pathlib import Path

from harness.fixtures import FixtureTree
from harness.modes import ALL_MODES, NEXT_ENTRY_KEY, PREVIOUS_ENTRY_KEY

PREVIEW_FIXTURE = {
    "notes.txt": "the quick brown fox\n",
    "page.md": "# Heading\n\nBody text.\n",
    "third.txt": "third preview fixture\n",
    "data.csv": "name,value\nalpha,1\n",
    "folder": {"inner.txt": "inner\n", "nested-notes.txt": "nested preview fixture\n"},
}


@pytest.fixture
def fixture_tree(request):
    """Replaces the shared fixture with file types the preview can render."""

    layout = getattr(request, "param", PREVIEW_FIXTURE)
    tree = FixtureTree.create({name: "" if isinstance(value, Path) else value for name, value in layout.items()})
    for name, value in layout.items():
        if isinstance(value, Path):
            (tree.root / name).write_bytes(value.read_bytes())
    try:
        yield tree
    finally:
        tree.cleanup()


@pytest.mark.parametrize(
    "mode,selection",
    [
        pytest.param(
            "Columns",
            "keyboard",
            marks=pytest.mark.preferences(browser_mode="columns"),
            id="columns-keyboard",
        ),
        pytest.param(
            "Icons",
            "keyboard",
            marks=pytest.mark.preferences(browser_mode="icons"),
            id="icons-keyboard",
        ),
        pytest.param(
            "List",
            "keyboard",
            marks=pytest.mark.preferences(browser_mode="list"),
            id="list-keyboard",
        ),
        pytest.param(
            "List",
            "pointer",
            marks=pytest.mark.preferences(browser_mode="list"),
            id="list-pointer",
        ),
    ],
)
def test_space_opens_and_closes_the_quick_preview(strata, mode, selection):
    before = strata.entry_names()
    if selection == "keyboard":
        strata.select_entry_with_keyboard("notes.txt")
    else:
        strata.select_entry("notes.txt")

    strata.keyboard.press("space")

    strata.wait(
        lambda: strata.preview_shows("the quick brown fox"),
        "the preview to render the file's text",
    )

    preview = strata.preview()
    assert preview.find(role="label", name="notes.txt") is not None
    assert strata.preview_shows("text/plain")
    assert strata.entry_names() == before
    close = preview.find(role="button", name="Close preview (Space)")
    strata.pointer.click(close)
    strata.wait(lambda: strata.preview() is None, "the preview to close")


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("selection", ["keyboard", "pointer"])
def test_space_previews_a_filtered_result_without_changing_the_query(strata, mode, selection):
    strata.select_entry("notes.txt")
    strata.keyboard.press("ctrl+f")
    field = strata.editable_field()
    strata.keyboard.type_text("nested-notes")
    strata.wait(
        lambda: strata.matches() == ["nested-notes.txt"],
        "the nested search result",
    )
    if selection == "keyboard":
        strata.keyboard.press("Down")
    else:
        result = strata.window.find(name="nested-notes.txt", role="list item")
        assert result is not None
        strata.pointer.click(result, modifiers=("ctrl",))

    strata.keyboard.press("space")
    strata.wait(
        lambda: strata.preview_shows("nested preview fixture"),
        "Space to preview the search result, not the stale directory selection",
    )
    assert field.text == "nested-notes"
    assert strata.matches() == ["nested-notes.txt"]
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview() is None, "Space to close the filtered preview")
    assert field.text == "nested-notes"
    assert strata.matches() == ["nested-notes.txt"]


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("selection", ["keyboard", "pointer"])
def test_preview_follows_the_selection(strata, mode, selection):
    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")
    strata.wait(
        lambda: strata.preview_shows("the quick brown fox"),
        "the first preview to render",
    )

    if selection == "keyboard":
        strata.keyboard.press(NEXT_ENTRY_KEY[mode])
    else:
        strata.select_entry("page.md")
    strata.wait_for_selection(["page.md"])

    strata.wait(
        lambda: strata.preview_shows("Body text."),
        "the preview to follow the newly selected file",
    )
    assert strata.focused_name() == "page.md"
    assert not strata.preview_shows("the quick brown fox")


def test_list_preview_keyboard_navigation_preserves_horizontal_scroll(strata, fixture_tree):
    strata.switch_view("List")
    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("the quick brown fox"), "the first preview")

    def list_scroll_origin():
        container = strata.pane()
        list_view = container.find(description="Files")
        assert list_view is not None, "the List file view"
        panes = [
            ancestor for ancestor in list_view.ancestors() if ancestor.role == "scroll pane"
        ]
        assert len(panes) >= 2, "the List view content is wrapped by its listing scroller"
        return panes[0].window_bounds().x

    origin = list_scroll_origin()
    for key, name, preview in (
        ("Down", "page.md", "Body text."),
        ("Down", "third.txt", "third preview fixture"),
        ("Up", "page.md", "Body text."),
    ):
        strata.keyboard.press(key)
        strata.wait_for_selection([name])
        strata.wait(lambda: strata.focused_name() == name, f"focus on {name}")
        strata.wait(lambda: strata.preview_shows(preview), f"preview for {name}")
        assert list_scroll_origin() == origin, (origin, list_scroll_origin())


@pytest.mark.parametrize("mode", ALL_MODES)
def test_preview_follows_extended_selection_without_collapsing_it(strata, mode):
    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("the quick brown fox"), "the first preview")

    strata.keyboard.press(f"shift+{NEXT_ENTRY_KEY[mode]}")

    strata.wait_for_selection(["notes.txt", "page.md"])
    strata.wait(lambda: strata.preview_shows("Body text."), "the newly focused preview")
    assert strata.focused_name() == "page.md"


@pytest.mark.parametrize("mode", ALL_MODES)
def test_preview_hides_on_a_folder_and_resumes_when_selection_moves(strata, mode):
    strata.select_entry_with_keyboard("data.csv")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("alpha"), "the file preview")

    strata.keyboard.press(PREVIOUS_ENTRY_KEY[mode])

    strata.wait_for_selection(["folder"])
    if mode == "Icons":
        strata.wait(lambda: strata.preview_shows("No preview for this selection"), "the folder's reserved preview space")
    else:
        strata.wait(lambda: strata.preview() is None, "the folder to dismiss the preview")
    strata.keyboard.press(NEXT_ENTRY_KEY[mode])
    strata.wait_for_selection(["data.csv"])
    strata.wait(lambda: strata.preview_shows("alpha"), "the still-enabled preview to resume")


@pytest.mark.preferences(browser_mode="list")
def test_marquee_does_not_reopen_enabled_preview(strata):
    strata.select_entry_with_keyboard("data.csv")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("alpha"), "the enabled preview")
    strata.keyboard.press("Up")
    strata.wait_for_selection(["folder"])
    strata.wait(lambda: strata.preview() is None, "the folder to hide preview")
    strata.settle(strata.entry("third.txt"))

    last = strata.entry("third.txt").screen_bounds()
    first = strata.entry("data.csv").screen_bounds()
    start = (last.x + last.width // 2, last.y + last.height + 60)
    end = (first.x + 50, first.center[1])
    strata.pointer.drag_points(start, end, release=False)
    try:
        strata.wait_for_selection(["data.csv", "notes.txt", "page.md", "third.txt"])
        assert strata.preview() is None, "held marquee must not open preview"
    finally:
        strata.pointer.connection.button(1, False)
    assert strata.preview() is None
    strata.select_entry_with_keyboard("notes.txt")
    strata.wait(lambda: strata.preview_shows("the quick brown fox"), "keyboard preview to resume")


def test_preview_hides_on_shift_range_folder_focus(strata):
    strata.switch_view("List")
    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("the quick brown fox"), "the file preview")

    strata.keyboard.press("shift+Up")
    strata.wait_for_selection(["data.csv", "notes.txt"])
    strata.wait(lambda: strata.focused_name() == "data.csv", "the focused upper file")
    strata.wait(lambda: strata.preview_shows("alpha"), "the upper file preview")

    strata.keyboard.press("shift+Up")
    strata.wait_for_selection(["folder", "data.csv", "notes.txt"])
    strata.wait(lambda: strata.focused_name() == "folder", "the focused folder")
    strata.wait(lambda: strata.preview() is None, "the focused folder to dismiss the preview")

    strata.keyboard.press("shift+Down")
    strata.wait_for_selection(["data.csv", "notes.txt"])
    strata.wait(lambda: strata.preview_shows("alpha"), "preview to resume after the folder")


def test_preview_renders_markdown(strata, fixture_tree):
    from PIL import Image

    Image.new("RGB", (80, 32), "green").save(fixture_tree.path("folder/local image.png"))
    fixture_tree.path("folder/shapes.svg").write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" width="80" height="32">'
        '<circle cx="16" cy="16" r="12" fill="red"/></svg>'
    )
    fixture_tree.path("page.md").write_text(
        "# Heading\n\nBody text.\n\n"
        "![Local PNG](folder/local%20image.png)\n\n"
        "![Local SVG](folder/shapes.svg)\n\n"
        "```mermaid\nflowchart LR\nA[Open] --> B[Preview]\n```\n\n"
        "![Missing fixture](folder/missing.png)\n"
    )
    strata.select_entry_with_keyboard("page.md")
    strata.keyboard.press("space")

    strata.wait(
        lambda: strata.preview_shows("Body text."),
        "the markdown preview to render its body",
    )
    for name in ("Local PNG", "Local SVG", "Mermaid diagram"):
        strata.wait(
            lambda name=name: strata.preview().find(role="image", description=name) is not None,
            f"the sandboxed Markdown media to render: {name}",
        )
    strata.wait(lambda: strata.preview_shows("Missing fixture"), "missing-image fallback")
    strata.pointer.click(strata.preview().find(role="button", name="View source"))
    strata.wait(lambda: strata.preview_shows("flowchart LR"), "original Mermaid source")
    strata.pointer.click(strata.preview().find(role="button", name="View rendered"))
    strata.wait(
        lambda: strata.preview().find(role="image", description="Mermaid diagram") is not None,
        "the cached diagram after switching back to rendered view",
    )
    strata.select_entry("notes.txt")
    strata.wait(lambda: strata.preview_shows("the quick brown fox"), "next preview")
    assert strata.preview().find(role="image", description="Mermaid diagram") is None


def test_markdown_equations_preserve_inline_prose_source_and_fallbacks(strata, fixture_tree):
    fixture_tree.path("page.md").write_text(
        "# Equations\n\nEnergy $E=mc^2$ in prose.\n\n"
        "$$\\frac{-b\\pm\\sqrt{b^2-4ac}}{2a}$$\n\n"
        "```latex\n\\sum_{n=1}^{\\infty}\\frac{1}{n^2}=\\frac{\\pi^2}{6}\n```\n\n"
        "Unsupported $\\unknowncommand{x}$ remains readable.\n"
    )
    strata.select_entry_with_keyboard("page.md")
    strata.keyboard.press("space")
    strata.wait(
        lambda: len(strata.preview().find_all(role="image", description="LaTeX equation")) == 3,
        "inline, display, and fenced equations rendered through the sandbox",
    )
    assert strata.preview_shows("Energy") and strata.preview_shows("in prose."), "\n".join(
        repr((node.role, node.text)) for node in strata.preview().find_all(rendered=False) if node.text
    )
    strata.wait(lambda: strata.preview_shows("$\\unknowncommand{x}$"), "unsupported equation fallback")
    strata.pointer.click(strata.preview().find(role="button", name="View source"))
    strata.wait(lambda: strata.preview_shows("$E=mc^2$"), "unchanged equation source")
    strata.pointer.click(strata.preview().find(role="button", name="View rendered"))
    strata.wait(
        lambda: len(strata.preview().find_all(role="image", description="LaTeX equation")) == 3,
        "equations retained across view switching",
    )


def _large_table(extension):
    rows = [(f"record-{index}", index) for index in range(1000)]
    if extension in ("csv", "tsv"):
        separator = "," if extension == "csv" else "\t"
        return separator.join(("name", "value")) + "\n" + "".join(
            f"{name}{separator}{value}\n" for name, value in rows
        )
    if extension == "md":
        return "| name | value |\n| --- | --- |\n" + "".join(
            f"| {name} | {value} |\n" for name, value in rows
        )
    return "<table><tr><th>name</th><th>value</th></tr>" + "".join(
        f"<tr><td>{name}</td><td>{value}</td></tr>" for name, value in rows
    ) + "</table>"


@pytest.mark.parametrize("fixture_tree,filename", [
    ({f"table.{extension}": _large_table(extension)}, f"table.{extension}")
    for extension in ("csv", "tsv", "md", "html")
], indirect=["fixture_tree"])
@pytest.mark.preferences(render_documents_by_default=True)
def test_large_table_header_sort_reaches_rows_beyond_old_limits(strata, filename):
    strata.select_entry_with_keyboard(filename)
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("record-0"), "the first table row")
    header = strata.preview().find(role="column header", name="value")
    if header is None:
        header = strata.preview().find(role="filler", name="value")
    assert header is not None
    strata.pointer.click(header)
    strata.pointer.click(header)
    strata.wait(lambda: strata.preview_shows("record-999"), "numeric descending sort across every loaded row")
    assert strata.preview().find(role="button", name="Copy table") is None
    cell = strata.preview().find(role="label", name="record-999")
    assert cell is not None
    strata.pointer.click(cell)
    strata.keyboard.press("End")
    for _ in range(3):
        strata.keyboard.press("shift+Left")
    strata.keyboard.press("ctrl+c")
    strata.keyboard.press("ctrl+l")
    field = strata.editable_field()
    strata.keyboard.press("ctrl+a")
    strata.keyboard.press("ctrl+v")
    strata.wait(lambda: field.text == "999", "only the selected cell text to reach the clipboard")
    strata.keyboard.press("Escape")
    first = strata.preview().find(role="label", name="999").screen_bounds()
    last = strata.preview().find(role="label", name="997").screen_bounds()
    start = (first.x, first.center[1])
    end = (last.x + last.width - 1, last.center[1])
    for origin, destination in [(start, end), (end, start)]:
        strata.pointer.drag_points(origin, destination)
        strata.keyboard.press("ctrl+c")
        assert table_clipboard_text(strata) == "999\nrecord-998\t998\nrecord-997\t997"
    def click_cell_and_copy():
        strata.pointer.click(strata.preview().find(role="label", name="record-998"))
        strata.keyboard.press("ctrl+c")

    assert table_clipboard_text(strata, before_read=click_cell_and_copy) == "selection-cleared"
    preview = strata.preview().screen_bounds()
    strata.pointer.drag_points(start, (end[0], preview.y + preview.height - 12), release=False)
    try:
        strata.wait(lambda: strata.preview_shows("record-950"), "selection drag to autoscroll through recycled rows")
    finally:
        strata.pointer.connection.button(1, False)
    strata.keyboard.press("ctrl+c")
    copied = table_clipboard_text(strata)
    assert copied.startswith("999\nrecord-998\t998\n")
    assert "record-970\t970\n" in copied

    def dismiss_and_copy():
        strata.pointer.click(strata.preview(), at=(preview.x + 180, preview.y + 16))
        strata.keyboard.press("ctrl+c")

    assert table_clipboard_text(strata, before_read=dismiss_and_copy) == "selection-cleared"


def table_clipboard_text(strata, *, before_read=None):
    from gi.repository import Gdk, GLib

    display = Gdk.Display.open(strata.display.display)
    assert display is not None
    result = []
    try:
        if before_read is not None:
            display.get_clipboard().set("selection-cleared")
            display.flush()
            before_read()
        display.get_clipboard().read_text_async(
            None, lambda clipboard, response: result.append(clipboard.read_text_finish(response))
        )

        def received():
            context = GLib.MainContext.default()
            while context.pending():
                context.iteration(False)
            return bool(result)

        strata.wait(received, "selected table text on the private display clipboard")
        return result[0]
    finally:
        display.close()


@pytest.mark.parametrize("fixture_tree,filename", [
    ({f"book.{extension}": (Path(__file__).resolve().parents[2] / "fixtures" / "spreadsheets" / f"any_sheets.{extension}")}, f"book.{extension}")
    for extension in ("xls", "xlsx", "ods")
], indirect=["fixture_tree"])
def test_workbook_uses_sandboxed_shared_table_preview(strata, filename):
    strata.select_entry_with_keyboard(filename)
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("3"), "the sandboxed workbook cells")
    assert strata.preview().find(role="button", name="Copy table") is None
    assert strata.preview().find(role="button", name="View source") is None


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
def test_column_preview_fills_free_space_and_remembers_a_dragged_session_width(strata):
    def adjacent():
        column = strata.containers()[-1].screen_bounds()
        preview = strata.preview().screen_bounds()
        return abs(preview.x - (column.x + column.width)) <= 3

    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("the quick brown fox"), "the first preview")
    strata.wait(adjacent, "the preview to meet the last column")
    initial = strata.preview().screen_bounds().width
    strata.keyboard.press("Down")
    strata.wait(lambda: strata.preview_shows("Body text."), "keyboard selection to update the preview")
    strata.keyboard.press("space")
    strata.open_directory("folder")
    strata.select_entry_with_keyboard("inner.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("inner"), "the nested preview")
    strata.wait(adjacent, "columns to scroll left beside the minimum-width preview")
    minimum = strata.preview().screen_bounds().width
    assert minimum < initial
    assert strata.containers()[0].screen_bounds().x < strata.pane("folder").screen_bounds().x

    bounds = strata.preview().screen_bounds()
    start = (bounds.x - 1, bounds.y + bounds.height // 2)
    distance = bounds.width // 5
    strata.pointer.drag_points(start, (start[0] + distance, start[1]))
    strata.wait(
        lambda: strata.preview().screen_bounds().width < minimum - distance // 2,
        "the dragged width to override the automatic minimum",
    )
    chosen = strata.preview().screen_bounds().width
    resized = strata.preview().screen_bounds()
    window = strata.window_bounds()
    assert abs(resized.x + resized.width - window.x - window.width) <= 2
    strata.keyboard.press("space")
    strata.select_entry_with_keyboard("nested-notes.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("nested preview fixture"), "the reopened preview")
    assert abs(strata.preview().screen_bounds().width - chosen) <= 2
    strata.keyboard.press("space")
    strata.keyboard.press("alt+Left")
    strata.select_entry_with_keyboard("notes.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("the quick brown fox"), "the parent preview")
    assert abs(strata.preview().screen_bounds().width - chosen) <= 2


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
def test_columns_preview_can_reopen_after_closing(strata):
    strata.open_directory("folder")
    strata.select_entry_with_keyboard("inner.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("inner"), "the nested preview")
    close = strata.preview().find(role="button", name="Close preview (Space)")
    strata.pointer.click(close)
    strata.wait(lambda: strata.preview() is None, "the preview to close")
    strata.select_entry("nested-notes.txt")
    strata.select_entry("inner.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("inner"), "the preview to reopen")


@pytest.mark.preferences(browser_mode="columns", single_click_previews=False)
def test_narrow_window_prioritizes_the_last_column_and_restores_the_latest_preview(strata):
    browser_left = strata.pane().screen_bounds().x
    strata.open_directory("folder")
    strata.select_entry_with_keyboard("inner.txt")
    strata.keyboard.press("space")
    strata.wait(lambda: strata.preview_shows("inner"), "the initial preview")
    preferred = strata.preview().screen_bounds().width

    def resize(width):
        bounds = strata.window_bounds()
        strata.keyboard.connection.resize_surface(bounds.width, bounds.height, width, bounds.height)
        strata.wait(lambda: strata.window_bounds().width == width, "the resized window")

    def last_column_visible():
        column = strata.pane("folder").screen_bounds()
        window = strata.window_bounds()
        return column.x >= browser_left and column.x + column.width <= window.x + window.width

    resize(900)
    strata.wait(lambda: strata.preview().screen_bounds().width < preferred, "the preview minimum to yield")
    strata.wait(last_column_visible, "the entire last column to stay visible")
    column = strata.pane("folder").screen_bounds()
    assert column.x + column.width <= strata.preview().screen_bounds().x
    resize(760)
    strata.wait(lambda: strata.preview() is None, "the unusably narrow preview to hide")
    strata.wait(last_column_visible, "the last column without the preview")
    strata.keyboard.press("Down")
    strata.wait_for_selection(["nested-notes.txt"])
    assert strata.preview() is None
    resize(900)
    strata.wait(lambda: strata.preview_shows("nested preview fixture"), "the latest selection to resume")
    strata.wait(last_column_visible, "the last column beside the resumed preview")
    resize(1200)
    strata.wait(lambda: strata.preview().screen_bounds().width == preferred, "the preferred preview width to return")
