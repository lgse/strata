# SPDX-License-Identifier: MIT
"""A fake GVfs remote volume monitor for QA runs with `"volume_monitor":true`.

GIO's remote volume monitor module (`libgioremote-volume-monitor.so`, from gvfs)
reads `tests/qa/volume_monitors/*.monitor` through `GVFS_MONITOR_DIR` and lists
the drives, volumes and mounts this process serves on the run's private session
bus, as it would those of `gvfs-udisks2-volume-monitor`. Scenarios change them
with `dbus_send` calls to the control interface:

    Add(s device)   JSON {"id", "name", "root", "mounted": false, "drive": true,
                    "can_eject": true, "mount_error": null}; plugs the device in
    Remove(s id)    unplugs it, removing its mount, volume and drive

`root` is the folder its mount shows. With `"can_eject": false` the drive
reports no eject, so Strata offers Unmount. Eject unplugs the device; Unmount
keeps its volume. A `mount_error` makes every mount fail with that GIO message
(`G_IO_ERROR_FAILED`). Each
call the application makes is appended to the file named by the first argument
as `<method> <device id>`. Prints `ready` once it owns its name.
"""

import json
import sys

from gi.repository import Gio, GLib

NAME = "org.strata.QaVolumeMonitor"
MONITOR_PATH = "/org/gtk/Private/RemoteVolumeMonitor"
MONITOR_INTERFACE = "org.gtk.Private.RemoteVolumeMonitor"
CONTROL_PATH = "/org/strata/QaVolumeMonitor"
CONTROL_INTERFACE = "org.strata.QaVolumeMonitor"

DRIVE = "(ssssbbbbbbbbuasa{ss}sa{sv})"
VOLUME = "(ssssssbbssa{ss}sa{sv})"
MOUNT = "(ssssssbsassa{sv})"


def signal(name, struct):
    return (f"<signal name='{name}'><arg type='s'/><arg type='s'/>"
            f"<arg type='{struct}'/></signal>")


def method(name, arguments=(), returns=()):
    parts = [f"<arg type='{kind}' direction='in'/>" for kind in arguments]
    parts += [f"<arg type='{kind}' direction='out'/>" for kind in returns]
    return f"<method name='{name}'>{''.join(parts)}</method>"


OPERATION = ("s", "s", "u", "s")
INTERFACES = Gio.DBusNodeInfo.new_for_xml(
    f"<node><interface name='{MONITOR_INTERFACE}'>"
    + "".join(signal(name, DRIVE) for name in ("DriveChanged", "DriveConnected", "DriveDisconnected"))
    + "".join(signal(name, VOLUME) for name in ("VolumeChanged", "VolumeAdded", "VolumeRemoved"))
    + "".join(signal(name, MOUNT) for name in ("MountChanged", "MountAdded", "MountRemoved"))
    + method("IsSupported", returns=("b",))
    + method("List", returns=(f"a{DRIVE}", f"a{VOLUME}", f"a{MOUNT}"))
    + method("CancelOperation", ("s",), ("b",))
    + method("MountUnmount", OPERATION)
    + method("VolumeMount", OPERATION)
    + method("DriveEject", OPERATION)
    + method("DrivePollForMedia", ("s", "s"))
    + method("DriveStart", OPERATION)
    + method("DriveStop", OPERATION)
    + "</interface>"
    f"<interface name='{CONTROL_INTERFACE}'>"
    + method("Add", ("s",)) + method("Remove", ("s",))
    + "</interface></node>"
)

calls_path = sys.argv[1]
devices = {}
connection = None


def record(call, device):
    with open(calls_path, "a", encoding="utf-8") as calls:
        calls.write(f"{call} {device}\n")


def drive_struct(device):
    return (f"drive-{device['id']}", device["name"], "drive-removable-media",
            "drive-removable-media-symbolic", device["can_eject"], False, True, True, True,
            False, False, False, 0, [f"volume-{device['id']}"], {}, "",
            {"is-removable": GLib.Variant("b", True)})


def volume_struct(device):
    mount = f"mount-{device['id']}" if device["mounted"] else ""
    drive = f"drive-{device['id']}" if device["drive"] else ""
    return (f"volume-{device['id']}", device["name"], "drive-removable-media",
            "drive-removable-media-symbolic", f"uuid-{device['id']}", "", True, False, drive,
            mount, {"uuid": f"uuid-{device['id']}"}, "",
            {"is-removable": GLib.Variant("b", True)})


def mount_struct(device):
    root = Gio.File.new_for_path(device["root"]).get_uri()
    return (f"mount-{device['id']}", device["name"], "drive-removable-media",
            "drive-removable-media-symbolic", f"uuid-{device['id']}", root, True,
            f"volume-{device['id']}", [], "", {})


def emit(name, object_id, struct, value):
    print(f"emitting {name} {object_id}", file=sys.stderr, flush=True)
    connection.emit_signal(None, MONITOR_PATH, MONITOR_INTERFACE, name,
                           GLib.Variant(f"(ss{struct})", (NAME, object_id, value)))


def mount(device):
    device["mounted"] = True
    emit("MountAdded", f"mount-{device['id']}", MOUNT, mount_struct(device))
    emit("VolumeChanged", f"volume-{device['id']}", VOLUME, volume_struct(device))


def unmount(device):
    if not device["mounted"]:
        return
    emit("MountRemoved", f"mount-{device['id']}", MOUNT, mount_struct(device))
    device["mounted"] = False
    emit("VolumeChanged", f"volume-{device['id']}", VOLUME, volume_struct(device))


def plug(device):
    if device["drive"]:
        emit("DriveConnected", f"drive-{device['id']}", DRIVE, drive_struct(device))
    emit("VolumeAdded", f"volume-{device['id']}", VOLUME, volume_struct(device))
    if device["mounted"]:
        emit("MountAdded", f"mount-{device['id']}", MOUNT, mount_struct(device))


def unplug(device):
    if device["mounted"]:
        emit("MountRemoved", f"mount-{device['id']}", MOUNT, mount_struct(device))
    emit("VolumeRemoved", f"volume-{device['id']}", VOLUME, volume_struct(device))
    if device["drive"]:
        emit("DriveDisconnected", f"drive-{device['id']}", DRIVE, drive_struct(device))
    del devices[device["id"]]


def device_for(object_id):
    return devices.get(object_id.split("-", 1)[-1])


def listing():
    drives = [drive_struct(device) for device in devices.values() if device["drive"]]
    volumes = [volume_struct(device) for device in devices.values()]
    mounts = [mount_struct(device) for device in devices.values() if device["mounted"]]
    return GLib.Variant(f"(a{DRIVE}a{VOLUME}a{MOUNT})", (drives, volumes, mounts))


def fail(invocation, message):
    invocation.return_error_literal(Gio.io_error_quark(), Gio.IOErrorEnum.FAILED, message)


def operate(name, device, invocation):
    record(name, device["id"])
    if name == "VolumeMount":
        if device["mount_error"]:
            fail(invocation, device["mount_error"])
            return
        mount(device)
    elif name == "MountUnmount":
        unmount(device)
    elif name == "DriveEject":
        unplug(device)
    invocation.return_value(None)


def control(name, argument, invocation):
    if name == "Add":
        device = {"mounted": False, "drive": True, "can_eject": True, "mount_error": None}
        device.update(json.loads(argument))
        devices[device["id"]] = device
        plug(device)
    elif devices.get(argument):
        unplug(devices[argument])
    else:
        fail(invocation, f"no device {argument!r}")
        return
    invocation.return_value(None)


def on_call(_connection, sender, _path, interface, name, parameters, invocation):
    print(f"{sender} called {name}{parameters}", file=sys.stderr, flush=True)
    if interface == CONTROL_INTERFACE:
        control(name, parameters.unpack()[0], invocation)
    elif name == "IsSupported":
        invocation.return_value(GLib.Variant("(b)", (True,)))
    elif name == "List":
        invocation.return_value(listing())
    elif name == "CancelOperation":
        invocation.return_value(GLib.Variant("(b)", (False,)))
    elif name in ("VolumeMount", "MountUnmount", "DriveEject"):
        device = device_for(parameters.unpack()[0])
        if device is None:
            fail(invocation, f"no device for {parameters.unpack()[0]!r}")
        else:
            operate(name, device, invocation)
    else:
        fail(invocation, f"{name} is not supported by the QA volume monitor")


def on_bus(bus, _name):
    global connection
    connection = bus
    for path, interface in ((MONITOR_PATH, MONITOR_INTERFACE), (CONTROL_PATH, CONTROL_INTERFACE)):
        bus.register_object_with_closures2(
            path, INTERFACES.lookup_interface(interface), on_call, None, None)


def on_name(_bus, _name):
    print("ready", flush=True)


def on_lost(_bus, _name):
    sys.exit("lost the bus name")


open(calls_path, "w", encoding="utf-8").close()
Gio.bus_own_name(Gio.BusType.SESSION, NAME, Gio.BusNameOwnerFlags.NONE, on_bus, on_name, on_lost)
GLib.MainLoop().run()
