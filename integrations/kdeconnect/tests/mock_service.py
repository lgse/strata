#!/usr/bin/python3
# SPDX-License-Identifier: MIT
"""KDE Connect stand-in for private-bus integration tests; never transfers files."""

import json
import os
from pathlib import Path
import sys

from gi.repository import Gio, GLib

SERVICE = "org.kde.kdeconnect"
ROOT = "/modules/kdeconnect"
XML = """<node>
<interface name="org.kde.kdeconnect.daemon">
 <method name="deviceNames"><arg type="b" direction="in"/><arg type="b" direction="in"/><arg type="a{ss}" direction="out"/></method>
 <signal name="deviceListChanged"/>
</interface>
<interface name="org.kde.kdeconnect.device">
 <method name="hasPlugin"><arg type="s" direction="in"/><arg type="b" direction="out"/></method>
</interface>
<interface name="org.kde.kdeconnect.device.share">
 <method name="shareUrls"><arg type="as" direction="in"/></method>
</interface>
</node>"""


class MockService:
    def __init__(self, control, record, ready):
        self.control, self.record, self.ready = map(Path, (control, record, ready))
        self.previous = None
        self.registrations = []
        self.connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        self.info = Gio.DBusNodeInfo.new_for_xml(XML)
        self.connection.register_object(ROOT, self.info.interfaces[0], self.call, None, None)
        self.owner = Gio.bus_own_name_on_connection(
            self.connection, SERVICE, Gio.BusNameOwnerFlags.NONE,
            lambda *args: self.ready.write_text(self.connection.get_unique_name()),
            lambda *args: sys.exit("Cannot own the private mock service name"),
        )
        self.update()
        GLib.timeout_add(50, self.update)

    def state(self):
        return json.loads(self.control.read_text())

    def update(self):
        current = self.control.read_text()
        if current == self.previous:
            return True
        state = json.loads(current)
        for registration in self.registrations:
            self.connection.unregister_object(registration)
        self.registrations.clear()
        for device in state["devices"]:
            path = ROOT + "/devices/" + device
            for suffix, interface in (("", 1), ("/share", 2)):
                self.registrations.append(self.connection.register_object(
                    path + suffix, self.info.interfaces[interface], self.call, None, None,
                ))
        self.previous = current
        self.connection.emit_signal(None, ROOT, SERVICE + ".daemon", "deviceListChanged", None)
        return True

    def call(self, connection, sender, path, interface, method, parameters, invocation):
        args = parameters.unpack()
        state = self.state()
        if method == "deviceNames":
            assert args == (True, True), args
            devices = {key: value["name"] for key, value in state["devices"].items()
                       if value.get("paired", True) and value.get("reachable", True)}
            invocation.return_value(GLib.Variant("(a{ss})", (devices,)))
        elif method == "hasPlugin":
            assert args == ("kdeconnect_share",), args
            device = path.rsplit("/", 1)[1]
            invocation.return_value(GLib.Variant("(b)", (state["devices"][device].get("share", True),)))
        elif method == "shareUrls":
            device = path.split("/")[-2]
            with self.record.open("a") as stream:
                stream.write(json.dumps({"device": device, "uris": args[0],
                                         "owner": connection.get_unique_name()}) + "\n")
            error = state.get("error")
            if error:
                invocation.return_dbus_error(error, "Mock submission error")
            else:
                invocation.return_value(GLib.Variant("()", ()))


if __name__ == "__main__":
    if not os.environ.get("DBUS_SESSION_BUS_ADDRESS"):
        sys.exit("An explicitly isolated session bus is required")
    service = MockService(*sys.argv[1:])
    GLib.MainLoop().run()
