# 24. Settings portal keys beyond `color-scheme`, with change signals

| | |
| --- | --- |
| Kind | New issue |
| Drafts | U12 (S5, from S1 R5) |
| Affects | P5 (desktop settings), K2 (text scale), C1b (double-click time), C23 (animations) |
| Retires | P5's zbus reader of `org.freedesktop.portal.Settings` |
| Filing | Attach the fake-portal script as a gist and link it, instead of pasting it inline (owner, 2026-11-10). |
| Checked | Re-checked at `2a44156`, by source: `KEY = "color-scheme"` (`platform/portal_appearance.rs:15`) is still the only key; the worker reads it with `ReadOne`, else `Read` (`:82–84`), and follows `SettingChanged` for it (`:81`). Reduced motion still comes from GSettings or `kdeglobals` (`platform/preferences.rs:26–65`), sampled when a window or overlay is created or focused (`window.rs:520`, `overlay.rs:391`). The program compiles at `2a44156` (`doc_24`); it needs a compositor and was not run. In the gallery, P5's zbus reader is still there (`strata-platform/src/settings/watcher.rs:28`, `settings/mod.rs:8–12`, U12), and `mise run test-platform p5::` passed in a private headless sway (20). The fake-portal script goes in a gist (Filing row); the body below still has it inline until the gist exists. Earlier: the same at `5d421fd` and `04b38ae`; S1's fake-portal run at `78d22ed`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Linux: read Settings portal keys beyond `color-scheme`, with change signals

Context: Strata (lgse/strata), a file manager on Rune, follows the desktop's text scale,
double-click time, cursor blink and animations switch. All live in
`org.freedesktop.portal.Settings`, which Rune already reads for `color-scheme`
(`platform/portal_appearance.rs:15` at `5d421fd`, unchanged at `2a44156`).

**Repro.** Any Rune window, on a private session bus with the fake portal below, under a
Wayland compositor (headless sway is enough):

```rust
use rune::{App, AppSpec, Cx, El, col, text};
fn view(_: &(), _: &mut Cx<()>) -> El { col().w_full().h_full().child(text("portal keys")) }
fn main() { App::new(AppSpec::new("portal", (320.0, 160.0), || (), view)).run(); }
```

<details><summary>Commands and the fake portal (<code>fake_portal.py</code>, PyGObject)</summary>

```sh
cargo build
dbus-run-session -- sh -c '
  python3 -I fake_portal.py \
    --init "org.freedesktop.appearance color-scheme u 1; org.gnome.desktop.interface text-scaling-factor d 1.0; org.gnome.desktop.interface cursor-blink b true; org.gnome.desktop.interface cursor-blink-time i 1200; org.gnome.desktop.interface enable-animations b true; org.gnome.desktop.peripherals.mouse double-click i 400" \
    --schedule "3 org.gnome.desktop.interface text-scaling-factor d 1.5; 3.5 org.gnome.desktop.interface enable-animations b false; 4 org.gnome.desktop.peripherals.mouse double-click i 1200; 4.5 org.freedesktop.appearance color-scheme u 2" \
    > portal.log &
  sleep 1
  WAYLAND_DISPLAY=wayland-1 timeout 6 ./target/debug/portal
'
grep '"ev": "call"' portal.log
```

```python
"""A fake org.freedesktop.portal.Settings on the session bus.

Usage: python3 -I fake_portal.py [--v1] [--schedule "T NS KEY TYPE VALUE; ..."] [--init "NS KEY TYPE VALUE; ..."]
--v1 drops ReadOne (portal version 1, Read only). TYPE is a GVariant type string.
Prints JSON lines: ready, each method call, each emitted change.
"""
import json
import sys
import time

from gi.repository import Gio, GLib

XML = """
<node><interface name="org.freedesktop.portal.Settings">
  <method name="ReadAll"><arg type="as" direction="in"/><arg type="a{sa{sv}}" direction="out"/></method>
  <method name="Read"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
  <method name="ReadOne"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
  <signal name="SettingChanged"><arg type="s"/><arg type="s"/><arg type="v"/></signal>
  <property name="version" type="u" access="read"/>
</interface></node>
"""

START = time.monotonic()
args = sys.argv[1:]
v1 = "--v1" in args


def opt(name):
    return args[args.index(name) + 1] if name in args else ""


def parse(spec):
    out = []
    for item in filter(None, (s.strip() for s in spec.split(";"))):
        out.append(item.split(None, 4))
    return out


def log(ev, **f):
    print(json.dumps({"t": round(time.monotonic() - START, 3), "w": int(time.time() * 1000), "ev": ev, **f}), flush=True)


values = {}
for ns, key, ty, val in parse(opt("--init")):
    values[(ns, key)] = GLib.Variant.parse(GLib.VariantType(ty), val, None, None)


def call(conn, sender, path, iface, method, params, inv):
    p = params.unpack()
    log("call", method=method, args=p, sender=sender)
    if method == "ReadAll":
        res = {}
        for (ns, key), v in values.items():
            if not p[0] or any(ns == want or (want.endswith("*") and ns.startswith(want[:-1])) for want in p[0]):
                res.setdefault(ns, {})[key] = v
        inv.return_value(GLib.Variant("(a{sa{sv}})", (res,)))
        return
    v = values.get((p[0], p[1]))
    if v is None:
        inv.return_dbus_error("org.freedesktop.portal.Error.NotFound", "no such key")
    elif method == "ReadOne" and not v1:
        inv.return_value(GLib.Variant.new_tuple(GLib.Variant("v", v)))
    elif method == "ReadOne":
        inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", "ReadOne")
    else:
        # Read wraps the value in a second variant, as real portals do.
        inv.return_value(GLib.Variant.new_tuple(GLib.Variant("v", GLib.Variant("v", v))))


def get_prop(conn, sender, path, iface, name):
    return GLib.Variant("u", 1 if v1 else 2)


def acquired(conn, name):
    info = Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0]
    conn.register_object("/org/freedesktop/portal/desktop", info, call, get_prop, None)
    for at, ns, key, ty, val in parse(opt("--schedule")):
        GLib.timeout_add(int(float(at) * 1000), change, conn, ns, key, ty, val)


def owned(conn, name):
    log("ready", name=name, version=1 if v1 else 2)


def change(conn, ns, key, ty, val):
    v = GLib.Variant.parse(GLib.VariantType(ty), val, None, None)
    values[(ns, key)] = v
    conn.emit_signal(None, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings", "SettingChanged",
                     GLib.Variant("(ssv)", (ns, key, v)))
    log("changed", ns=ns, key=key, value=val)
    return False


Gio.bus_own_name(Gio.BusType.SESSION, "org.freedesktop.portal.Desktop", Gio.BusNameOwnerFlags.NONE, acquired, owned,
                 lambda *_: log("name-lost"))
GLib.MainLoop().run()
```

</details>

The fake portal served `org.gnome.desktop.interface` `text-scaling-factor`, `cursor-blink`,
`cursor-blink-time` and `enable-animations`, and
`org.gnome.desktop.peripherals.mouse double-click`, beside `org.freedesktop.appearance
color-scheme`, and changed each on a schedule. Under headless sway at `78d22ed` it logged one
call from Rune, `ReadOne(org.freedesktop.appearance, color-scheme)`, and nothing else; the window
followed the `color-scheme` change only. Rune reads reduced motion from GSettings or `kdeglobals`
when a window is created or focused, which skips the portal (sandboxes) and its change signals.

**Ask.** The portal's values for these keys, or a generic Settings portal reader
(`rune::settings::read(namespace, key)` with a change callback), with change signals.

Related: (link to 25) (the double-click interval Rune uses itself), #285 (dark mode detection on
DMS).
