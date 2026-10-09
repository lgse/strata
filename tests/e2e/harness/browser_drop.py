# SPDX-License-Identifier: MIT
"""An external GTK drag source offering the formats used by web browsers."""

from __future__ import annotations

import sys
from contextlib import contextmanager
from pathlib import Path

APPLICATION_ID = "org.strata.E2EBrowserDrop"


@contextmanager
def browser_drop_source(strata, kind: str, payload: bytes):
    from . import tree
    from .environment import process_environment
    from .process import ManagedProcess, terminate

    path = strata.environment.root / "browser-drop-payload"
    path.write_bytes(payload)
    process = ManagedProcess.spawn(
        "browser-drop-source",
        [sys.executable, str(Path(__file__).resolve()), kind, str(path)],
        log_dir=strata.environment.root,
        env={
            **process_environment(),
            **strata.environment.variables(),
            **strata.display.environment,
        },
    )
    try:
        def source():
            if process.exited():
                raise AssertionError(process.tail())
            application = tree.find_application(APPLICATION_ID)
            return application.find(role="button", name="Drag") if application else None

        yield strata.wait(source, "the external browser drag source")
    finally:
        terminate(process.popen)
        bounds = strata.window.screen_bounds()
        strata.keyboard.connection.focus_surface(bounds.width, bounds.height)


def serve(kind: str, path: Path) -> None:
    import gi

    gi.require_version("Gtk", "4.0")
    from gi.repository import Gdk, GLib, Gtk

    GLib.set_prgname(APPLICATION_ID)
    providers = []
    payload = path.read_bytes()
    if kind in {"image", "binary", "named-binary"}:
        mime = {"image": "image/png", "binary": "application/octet-stream", "named-binary": 'application/octet-stream;name="browser-photo.jpg"'}[kind]
        providers.append(Gdk.ContentProvider.new_for_bytes(mime, GLib.Bytes.new(payload)))
        payload = b"https://example.invalid/extensionless-image"
    if kind in {"image", "binary", "named-binary", "link"}:
        providers.append(Gdk.ContentProvider.new_for_bytes("text/uri-list", GLib.Bytes.new(payload + b"\r\n")))
    providers.append(Gdk.ContentProvider.new_for_bytes("text/plain;charset=utf-8", GLib.Bytes.new(payload)))
    provider = Gdk.ContentProvider.new_union(providers)
    application = Gtk.Application(application_id=APPLICATION_ID)

    def activate(app):
        window = Gtk.ApplicationWindow(application=app, title="Browser drop source")
        window.set_decorated(False)
        button = Gtk.Button(label="Drag")
        source = Gtk.DragSource(actions=Gdk.DragAction.COPY)
        source.connect("prepare", lambda *_: provider)
        button.add_controller(source)
        window.set_child(button)
        window.present()

    application.connect("activate", activate)
    application.run([])


if __name__ == "__main__":
    serve(sys.argv[1], Path(sys.argv[2]))
