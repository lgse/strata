# SPDX-License-Identifier: MIT

from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from io import BytesIO
from pathlib import Path
from threading import Thread
from urllib.parse import unquote, urlparse
import os
import shutil

import pytest
from gi.repository import GLib
from PIL import Image

from harness import tree
from harness.portal import open_file_request


@contextmanager
def image_server(body):
    requests = []

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append(self.path)
            self.send_response(200)
            # Neither the URL nor the HTTP type is authoritative.
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    worker = Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/download.png", requests
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=5)


@pytest.fixture
def png_chooser(strata_binary, headless_display, test_environment, fixture_tree, keyboard, pointer):
    options = {
        "current_folder": GLib.Variant("ay", os.fsencode(fixture_tree.root) + b"\0"),
        "filters": GLib.Variant("a(sa(us))", [("PNG images", [(1, "image/png")])]),
    }
    with open_file_request(
        strata_binary, headless_display, test_environment, fixture_tree,
        title="PNG conversion test", options=options, browser_mode="list",
    ) as (chooser, future):

        def submit(url):
            entry = tree.wait_until(lambda: chooser.find(role="text", states={"editable"}), message="Name field")
            pointer.click(entry)
            keyboard.press("ctrl+a")
            keyboard.type_text(url)
            keyboard.press("Return")

        yield chooser, future, submit


def test_png_conversion_requires_consent_and_reuses_the_download(png_chooser):
    chooser, response, submit = png_chooser
    source = BytesIO()
    Image.new("RGB", (23, 17), "red").save(source, format="JPEG")
    with image_server(source.getvalue()) as (url, requests):
        submit(url)
        prompt = tree.wait_until(lambda: chooser.find(role="dialog", name="Convert image to PNG?"), message="conversion confirmation", timeout=20)
        assert not response.done()
        assert prompt.find(role="button", name="Cancel").activate()
        tree.wait_until(lambda: chooser.find(role="dialog", name="Convert image to PNG?") is None, message="declined conversion")
        assert not response.done()
        submit(url)
        prompt = tree.wait_until(lambda: chooser.find(role="dialog", name="Convert image to PNG?"), message="retry confirmation", timeout=20)
        assert prompt.find(role="button", name="Convert to PNG").activate()
        tree.wait_until(response.done, message="PNG portal response", timeout=20)
        status, values = response.result().unpack()
        assert status == 0
        output = Path(unquote(urlparse(values["uris"][0]).path))
        try:
            assert output.suffix == ".png"
            with Image.open(output) as png:
                assert png.format == "PNG"
                assert png.size == (23, 17)
                red, green, blue = png.convert("RGB").getpixel((5, 5))
                assert red > 240 and green < 10 and blue < 10
            assert len(requests) == 1, "retry must reuse the downloaded original"
        finally:
            shutil.rmtree(output.parent)


@pytest.mark.parametrize("invalid", ["animated", "corrupt"])
def test_png_conversion_errors_keep_the_chooser_open(png_chooser, invalid):
    chooser, response, submit = png_chooser
    if invalid == "animated":
        source = BytesIO()
        first = Image.new("RGB", (2, 3), "red")
        second = Image.new("RGB", (2, 3), "blue")
        first.save(source, format="GIF", save_all=True, append_images=[second], duration=100)
        body = source.getvalue()
        expected = "Animated images cannot be converted"
    else:
        body = b"\xff\xd8\xffcorrupt JPEG"
        expected = "damaged or uses unsupported"
    with image_server(body) as (url, _requests):
        submit(url)
        tree.wait_until(lambda: any(expected in node.name for _, node in chooser.walk() if node.role == "alert"), message="image conversion error", timeout=20, on_timeout=chooser.dump)
        assert not response.done()
        assert chooser.find(role="dialog", name="Convert image to PNG?") is None
        assert chooser.find(role="button", name="Open").has_state("sensitive")
