# SPDX-License-Identifier: MIT
"""Browser-style external data becomes safe files at the drop destination."""

import io
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest
from PIL import Image

from harness.browser_drop import browser_drop_source
from harness.modes import ALL_MODES


@pytest.mark.parametrize("mode", ALL_MODES)
@pytest.mark.parametrize("kind", ["image", "text", "link"])
def test_external_browser_content_drops_on_folder(strata, mode, kind):
    text = "Selected paragraph: café 日本語\n<b>plain text</b>\tend\n"
    address = "https://example.org/path?q=two%20words#section"
    if kind == "image":
        stream = io.BytesIO()
        Image.new("RGBA", (3, 2), (30, 80, 120, 255)).save(stream, format="PNG")
        payload = stream.getvalue()
        expected = "image.png"
    elif kind == "text":
        payload = text.encode()
        expected = "Dropped Text.txt"
    else:
        payload = address.encode()
        expected = "example.org.desktop"
    destination = strata.fixture.path(f"archive/{expected}")
    with browser_drop_source(strata, kind, payload) as source:
        strata.pointer.drag(source, strata.entry("archive"))
        strata.wait(destination.exists, "dropped browser content to be saved")
    if kind == "image":
        with Image.open(destination) as image:
            assert image.size == (3, 2)
            assert image.convert("RGBA").getpixel((1, 1)) == (30, 80, 120, 255)
        assert not strata.fixture.path("archive/example.invalid.desktop").exists()
    elif kind == "text":
        assert destination.read_text() == text
    else:
        contents = destination.read_text()
        assert "Type=Link\n" in contents
        assert f"URL={address}\n" in contents
        assert "Exec=" not in contents
        assert destination.stat().st_mode & 0o111 == 0


@pytest.fixture
def browser_image_url():
    stream = io.BytesIO()
    Image.new("RGB", (3, 2), (30, 80, 120)).save(stream, format="JPEG")
    payload = stream.getvalue()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Type", "image/jpeg")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, *_args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/photo-extensionless?q=80&auto=format"
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def test_url_only_image_drop_saves_image_not_shortcut(strata, browser_image_url):
    destination = strata.fixture.path("archive/photo-extensionless.jpg")
    with browser_drop_source(strata, "link", browser_image_url.encode()) as source:
        strata.pointer.drag(source, strata.entry("archive"))
        strata.wait(destination.exists, "URL-only drag to save the image rather than a shortcut")
    with Image.open(destination) as image:
        assert image.format == "JPEG"
        assert image.convert("RGB").getpixel((1, 1)) == (31, 80, 120)
    assert not list(strata.fixture.path("archive").glob("*.desktop"))


@pytest.mark.parametrize("kind, expected", [("binary", "image.jpg"), ("named-binary", "browser-photo.jpg")])
def test_browser_file_contents_take_precedence_over_url(strata, kind, expected):
    stream = io.BytesIO()
    Image.new("RGB", (3, 2), (30, 80, 120)).save(stream, format="JPEG")
    payload = stream.getvalue()
    destination = strata.fixture.path(f"archive/{expected}")
    with browser_drop_source(strata, kind, payload) as source:
        strata.pointer.drag(source, strata.entry("archive"))
        strata.wait(destination.exists, "browser-supplied file contents to be saved")
    assert destination.read_bytes() == payload
    assert not list(strata.fixture.path("archive").glob("*.desktop"))


@pytest.mark.parametrize("target", ["background", "breadcrumb", "sidebar", "tab"])
def test_browser_text_drop_uses_destination_surface(strata, target):
    expected_name = "Dropped Text.txt"
    if target == "breadcrumb":
        strata.open_directory("documents")
        destination = strata.fixture.path(expected_name)
        node = strata.wait(
            lambda: strata.window.find(role="button", name=strata.fixture.root.name),
            "parent breadcrumb",
        )
    elif target == "sidebar":
        destination = strata.environment.home / expected_name
        node = strata.sidebar_button("Home")
    elif target == "tab":
        destination = strata.fixture.path(expected_name)
        strata.keyboard.press("ctrl+t")
        strata.open_directory("documents")
        node = strata.wait(
            lambda: strata.window.find(role="page tab", name=strata.fixture.root.name),
            "destination tab",
        )
    else:
        destination = strata.fixture.path(expected_name)
        node = strata.pane()
    with browser_drop_source(strata, "text", b"selected text\n") as source:
        if target == "background":
            strata.pointer.drag_points(strata.pointer.drag_origin(source), strata.background_point())
        else:
            strata.pointer.drag(source, node)
        strata.wait(destination.exists, "text saved at the surface's directory")
    assert destination.read_text() == "selected text\n"


@pytest.mark.parametrize("reveal", [
    pytest.param(False, marks=pytest.mark.preferences(open_folder_after_drop=False)),
    pytest.param(True, marks=pytest.mark.preferences(open_folder_after_drop=True)),
])
def test_browser_content_drop_obeys_open_folder_preference(strata, reveal):
    destination = strata.fixture.path("archive/Dropped Text.txt")
    with browser_drop_source(strata, "text", b"paragraph\n") as source:
        strata.pointer.drag(source, strata.entry("archive"))
        strata.wait(destination.exists, "the dropped text to be saved")
    if reveal:
        strata.wait_for_directory("archive")
        strata.wait_for_selection(["Dropped Text.txt"], "archive")
    else:
        assert strata.current_directory() == strata.fixture.root.name
    assert destination.read_text() == "paragraph\n"


@pytest.fixture
def web_browser_app(test_environment):
    applications = test_environment.data_home / "applications"
    applications.mkdir()
    output = test_environment.root / "opened-url"
    launcher = test_environment.root / "record-url"
    launcher.write_text(f'#!/bin/sh\nprintf "%s\\n" "$@" > "{output}"\n')
    launcher.chmod(0o755)
    (applications / "strata-test-browser.desktop").write_text(
        "[Desktop Entry]\nType=Application\nName=Test Browser\n"
        f"Exec={launcher} %u\nMimeType=x-scheme-handler/https;\n"
    )
    (test_environment.config_home / "mimeapps.list").write_text(
        "[Default Applications]\nx-scheme-handler/https=strata-test-browser.desktop;\n"
    )
    return output


def test_dropped_web_link_opens_the_default_browser(web_browser_app, strata):
    address = "https://example.org/page?q=two%20words#part"
    destination = strata.fixture.path("archive/example.org.desktop")
    with browser_drop_source(strata, "link", address.encode()) as source:
        strata.pointer.drag(source, strata.entry("archive"))
        strata.wait(destination.exists, "web shortcut saved")
    assert not web_browser_app.exists()
    strata.open_directory("archive")
    strata.select_entry_with_keyboard("example.org.desktop")
    strata.keyboard.press("Return")
    strata.wait(web_browser_app.exists, "the default browser to receive the URL")
    assert web_browser_app.read_text().splitlines() == [address]
