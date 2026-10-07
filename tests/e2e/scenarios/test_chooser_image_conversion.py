# SPDX-License-Identifier: MIT

from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from io import BytesIO
from pathlib import Path
from threading import Thread
from urllib.parse import unquote, urlparse
import os
import shutil
import uuid

import pytest
from gi.repository import Gio, GLib
from PIL import Image

from harness import tree
from harness.environment import process_environment
from harness.process import ManagedProcess, terminate

BACKEND = "org.freedesktop.impl.portal.desktop.strata"
DESKTOP = "/org/freedesktop/portal/desktop"
APPLICATION = "io.github.lgse.Strata.FileChooser"


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
    test_environment.write_preferences({"browser_mode": "list"})
    environment = {**process_environment(), **test_environment.variables(), **headless_display.environment}
    backend = ManagedProcess.spawn("portal", [str(strata_binary), "--portal"], log_dir=test_environment.root, env=environment, cwd=fixture_tree.root)
    connection = Gio.DBusConnection.new_for_address_sync(
        environment["DBUS_SESSION_BUS_ADDRESS"],
        Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
        None, None,
    )
    pool = ThreadPoolExecutor(max_workers=1)
    handle = f"{DESKTOP}/request/strata_test/image_{uuid.uuid4().hex}"
    future = None
    try:
        tree.wait_until(lambda: connection.call_sync(
            "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameHasOwner",
            GLib.Variant("(s)", (BACKEND,)), None, Gio.DBusCallFlags.NONE, 1000, None,
        ).unpack()[0], message="portal backend registration", timeout=20)
        options = {
            "current_folder": GLib.Variant("ay", os.fsencode(fixture_tree.root) + b"\0"),
            "filters": GLib.Variant("a(sa(us))", [("PNG images", [(1, "image/png")])]),
        }
        parameters = GLib.Variant("(osssa{sv})", (handle, "", "", "PNG conversion test", options))
        future = pool.submit(connection.call_sync, BACKEND, DESKTOP, "org.freedesktop.impl.portal.FileChooser", "OpenFile", parameters, GLib.VariantType.new("(ua{sv})"), Gio.DBusCallFlags.NONE, 60000, None)

        def window():
            application = tree.find_application(APPLICATION)
            return application.find(name="PNG conversion test") if application else None

        chooser = tree.wait_until(window, message="PNG chooser", timeout=20)

        def submit(url):
            entry = tree.wait_until(lambda: chooser.find(role="text", states={"editable"}), message="Name field")
            pointer.click(entry)
            keyboard.press("ctrl+a")
            keyboard.type_text(url)
            keyboard.press("Return")

        yield chooser, future, submit
    finally:
        if future is not None and not future.done():
            try:
                connection.call_sync(BACKEND, handle, "org.freedesktop.impl.portal.Request", "Close", None, None, Gio.DBusCallFlags.NONE, 2000, None)
            except GLib.Error:
                pass
        terminate(backend.popen)
        pool.shutdown(wait=True, cancel_futures=True)
        connection.close_sync(None)


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
