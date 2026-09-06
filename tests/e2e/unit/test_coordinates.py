# SPDX-License-Identifier: GPL-3.0-or-later

import ctypes

import pytest

from harness.xtest import XErrorEvent, XTestConnection, XTestError


class NativeSurfaces:
    def __init__(self, windows):
        self.windows = windows
        self.children = (ctypes.c_ulong * len(windows))(*range(len(windows)))
        self.handler = None
        self.freed = False

    def XSync(self, *_):
        pass

    def XSetErrorHandler(self, handler):
        previous, self.handler = self.handler, handler
        return previous

    def XQueryTree(self, _display, _root, _root_return, _parent, children, count):
        ctypes.cast(children, ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)))[0] = self.children
        count._obj.value = len(self.windows)
        return 1

    def XGetWindowAttributes(self, _display, window, attributes):
        values = self.windows[window]
        if isinstance(values, int):
            event = XErrorEvent(error_code=values)
            self.handler(None, ctypes.byref(event))
            return 0
        for name, value in values.items():
            setattr(attributes._obj, name, value)
        return 1

    def XFree(self, _children):
        self.freed = True


def connection_for(windows):
    connection = object.__new__(XTestConnection)
    connection._display = 1
    connection._root = 1
    connection._x11 = NativeSurfaces(windows)
    return connection


def surface(x=500, y=200, width=258, height=475, map_state=2):
    return dict(x=x, y=y, width=width, height=height, map_state=map_state)


def test_only_visible_matching_surfaces_supply_an_origin():
    connection = connection_for([surface(), surface(x=10, map_state=0), surface(width=1200)])
    assert connection.surface_origin(258, 475) == (500, 200)
    assert connection._x11.freed
    assert connection._x11.handler is None


def test_ambiguous_surfaces_fail_instead_of_guessing():
    connection = connection_for([surface(), surface(x=10)])
    with pytest.raises(XTestError, match="ambiguous"):
        connection.surface_origin(258, 475)
    assert connection._x11.freed
    assert connection._x11.handler is None


def test_disappearing_surfaces_do_not_abort_the_runner():
    connection = connection_for([3, surface()])
    assert connection.surface_origin(258, 475) == (500, 200)
    assert connection._x11.handler is None


def test_other_native_errors_fail_the_test():
    connection = connection_for([8])
    with pytest.raises(XTestError, match="cannot inspect"):
        connection.surface_origin(258, 475)
    assert connection._x11.freed
    assert connection._x11.handler is None
