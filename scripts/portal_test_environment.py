# SPDX-License-Identifier: GPL-3.0-or-later

import os


def isolated_process_environment(root):
    environment = os.environ.copy()
    for key, directory in (
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CACHE_HOME", "cache"),
    ):
        environment[key] = str(root / directory)
    environment["GIO_USE_VFS"] = "local"
    environment["GDK_DEBUG"] = "no-portals"
    environment["GTK_A11Y"] = "none"
    return environment
