#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate Strata's semantic, theme-recolorable thumbnail SVGs."""

import html
import json
from pathlib import Path

HERE = Path(__file__).parent
OUTPUT = HERE.parents[1] / "data" / "thumbnails"
LUCIDE_NODES = json.loads((HERE / "lucide-nodes.json").read_text(encoding="utf-8"))

CATEGORIES = {
    "archive": "archive",
    "audio": "audio-lines",
    "audio-project": "sliders-horizontal",
    "certificate": "badge-check",
    "comics": "panels-top-left",
    "config": "braces",
    "database": "database",
    "design-cad": "pen-tool",
    "docx": "file-text",
    "ebooks": "book-open",
    "font": "baseline",
    "image": "image",
    "iso": "usb",
    "log": "list",
    "maps-gis": "map",
    "models-3d": "box",
    "music-score": "music-2",
    "package": "package",
    "playlists": "list-music",
    "pptx": "presentation",
    "scientific-data": "flask-conical",
    "spreadsheets": "table-2",
    "sql": "database-zap",
    "subtitles": "captions",
    "text-code": "code-xml",
    "video": "clapperboard",
    "virtual-disk": "hard-drive",
    "virtual-machine": "monitor-play",
    "web": "earth",
}

ICON_COLOR = "#030303"

TEMPLATE = '''<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256" viewBox="0 0 256 256">
<g transform="translate(8)">
<path d="M47 18h111l51 51v156c0 9-7 16-16 16H47c-9 0-16-7-16-16V34c0-9 7-16 16-16z" fill="#010101" stroke="#090909" stroke-width="5"/>
<path d="M158 18v39c0 9 7 16 16 16h35z" fill="#020202" stroke="#090909" stroke-width="5" stroke-linejoin="round"/>
</g>
<g transform="translate(68 89) scale(5)" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
{body}
</g>
</svg>
'''


def render_node(tag, attributes, color):
    values = {**attributes, "stroke": color}
    serialized = " ".join(
        f'{name}="{html.escape(str(value), quote=True)}"' for name, value in values.items()
    )
    return f"<{tag} {serialized}/>"


def main():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    for old in OUTPUT.glob("*.svg"):
        old.unlink()
    for category, icon in CATEGORIES.items():
        body = "\n".join(
            render_node(tag, attributes, ICON_COLOR)
            for tag, attributes in LUCIDE_NODES[icon]
        )
        source = TEMPLATE.format(body=body)
        (OUTPUT / f"strata-{category}.svg").write_text(source, encoding="utf-8")


if __name__ == "__main__":
    main()
