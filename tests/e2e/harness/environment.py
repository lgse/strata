# SPDX-License-Identifier: GPL-3.0-or-later
"""An isolated, deterministic HOME and XDG environment for one test."""

from __future__ import annotations

import os
import shutil
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

# Strata records that it has already offered to become the file-chooser
# portal by creating this file in its config directory.
PORTAL_OFFER_MARKER = "portal-opt-in-v1"

GTK_SETTINGS = """[Settings]
gtk-enable-animations=false
gtk-theme-name=Adwaita
gtk-icon-theme-name=Adwaita
gtk-font-name=Cantarell 11
gtk-cursor-blink=false
gtk-double-click-time=400
gtk-cursor-theme-name=Adwaita
gtk-cursor-theme-size=24
gtk-xft-antialias=1
gtk-xft-hinting=1
gtk-xft-hintstyle=hintslight
gtk-xft-rgba=none
gtk-overlay-scrolling=false
gtk-primary-button-warps-slider=false
gtk-decoration-layout=:
"""

# Every preference the browser reads at startup, pinned so a scenario never
# inherits a default change. Scenario-specific values are layered on top.
DEFAULT_PREFERENCES: dict[str, object] = {
    "mode": "theme",
    "theme": "azure-glow",
    "folder_peeking": False,
    "single_click_previews": False,
    "search_open_files_directly": False,
    "type_to_search": True,
    "show_keybinding_hints": True,
    "reduce_motion": True,
    "browser_mode": "columns",
    "browser_density": "compact",
    "group_by_type": False,
    "list_file_clicks": 2,
    "list_folder_clicks": 1,
    "grid_file_clicks": 2,
    "grid_folder_clicks": 2,
    "explorer_file_clicks": 2,
    "explorer_folder_clicks": 2,
    "show_hidden": False,
    "text_size": "medium",
    "folders_first": True,
    "sort_key": "name",
    "sort_direction": "ascending",
    "check_for_updates": False,
    "auto_refresh_interval": 0,
    "release_channel": "stable",
    "video_preview_backend": "automatic",
    "preview_muted": True,
    "preview_volume": 1.0,
    "sidebar_order": ["desktop", "documents", "downloads", "pictures", "videos"],
}


def process_environment() -> dict[str, str]:
    # Do not inherit desktop endpoints, GTK modules, or user configuration overrides.
    return {"PATH": os.environ.get("PATH", os.defpath)}


def _render_toml(values: dict[str, object]) -> str:
    lines = []
    for key, value in values.items():
        lines.append(f"{key} = {_render_value(value)}")
    return "\n".join(lines) + "\n"


def _render_value(value: object) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return repr(value)
    if isinstance(value, list):
        return "[" + ", ".join(_render_value(item) for item in value) + "]"
    return '"' + str(value).replace("\\", "\\\\").replace('"', '\\"') + '"'


@dataclass
class TestEnvironment:
    """A throwaway HOME the application under test may write to freely."""

    root: Path = field(init=False)

    def __post_init__(self) -> None:
        self.root = Path(tempfile.mkdtemp(prefix="strata-e2e-home-", dir="/tmp"))
        for name in ("home", "config", "data", "cache", "state"):
            (self.root / name).mkdir()
        gtk = self.config_home / "gtk-4.0"
        gtk.mkdir(parents=True)
        (gtk / "settings.ini").write_text(GTK_SETTINGS)
        self.write_preferences({})
        self._dismiss_portal_offer()

    @property
    def home(self) -> Path:
        return self.root / "home"

    @property
    def config_home(self) -> Path:
        return self.root / "config"

    @property
    def data_home(self) -> Path:
        return self.root / "data"

    @property
    def cache_home(self) -> Path:
        return self.root / "cache"

    @property
    def state_home(self) -> Path:
        return self.root / "state"

    @property
    def settings_path(self) -> Path:
        return self.config_home / "strata" / "settings.toml"

    @property
    def trash_files(self) -> Path:
        return self.data_home / "Trash" / "files"

    def _dismiss_portal_offer(self) -> None:
        """Pre-answer the first-run file-chooser portal offer.

        Strata shows it once per config home; leaving it armed would put a
        modal dialog over every scenario.
        """

        marker = self.config_home / "strata" / PORTAL_OFFER_MARKER
        marker.parent.mkdir(parents=True, exist_ok=True)
        marker.write_text("1\n")

    def write_preferences(self, overrides: dict[str, object]) -> None:
        """Seed `settings.toml`. Must run before the application starts."""

        values = dict(DEFAULT_PREFERENCES)
        values.update(overrides)
        self.settings_path.parent.mkdir(parents=True, exist_ok=True)
        self.settings_path.write_text(_render_toml(values))

    def read_preferences(self) -> dict[str, str]:
        """The persisted preferences as raw `key = value` strings."""

        try:
            text = self.settings_path.read_text()
        except OSError:
            return {}
        values: dict[str, str] = {}
        for line in text.splitlines():
            key, separator, value = line.partition("=")
            if separator:
                values[key.strip()] = value.strip()
        return values

    def variables(self) -> dict[str, str]:
        """Environment variables that pin locale, rendering, and storage."""

        return {
            "HOME": str(self.home),
            "XDG_CONFIG_HOME": str(self.config_home),
            "XDG_DATA_HOME": str(self.data_home),
            "XDG_CACHE_HOME": str(self.cache_home),
            "XDG_STATE_HOME": str(self.state_home),
            "XDG_DATA_DIRS": "/usr/local/share:/usr/share",
            "XDG_CURRENT_DESKTOP": "GNOME",
            "LC_ALL": "C.UTF-8",
            "LANG": "C.UTF-8",
            "LANGUAGE": "",
            "TZ": "UTC",
            "GDK_BACKEND": "x11",
            "GSK_RENDERER": "cairo",
            "GDK_SCALE": "1",
            "GDK_DPI_SCALE": "1",
            "GTK_A11Y": "atspi",
            "NO_AT_BRIDGE": "0",
            "GTK_THEME": "Adwaita",
            "GTK_USE_PORTAL": "0",
            "GTK_OVERLAY_SCROLLING": "0",
            "LIBGL_ALWAYS_SOFTWARE": "1",
            "GALLIUM_DRIVER": "llvmpipe",
            # The gvfs probe in `main` re-executes the process when the daemon
            # is missing; setting the backends up front skips both.
            "GIO_USE_VFS": "local",
            "GIO_USE_VOLUME_MONITOR": "unix",
            "GSETTINGS_BACKEND": "memory",
            "RUST_BACKTRACE": "1",
        }

    def cleanup(self) -> None:
        shutil.rmtree(self.root, ignore_errors=True)
