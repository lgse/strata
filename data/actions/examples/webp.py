# SPDX-License-Identifier: MIT

from pathlib import Path
import subprocess
import tempfile

from strata_actions import context, find_tool, require_tool

ctx = context()
magick = find_tool("magick") or require_tool("convert")
for index, path in enumerate(ctx.paths, start=1):
    source = Path(path)
    if not source.is_file():
        raise ValueError(f"Not a file: {source}")
    # A private, unique output folder cannot overwrite an existing file or link.
    folder = Path(tempfile.mkdtemp(prefix="strata-webp-", dir=source.parent))
    output = folder / "converted.webp"
    # Stream the image so ImageMagick cannot interpret its name as image syntax.
    with source.open("rb") as input_file, output.open("xb") as output_file:
        subprocess.run(
            [magick, "-[0]", "-auto-orient", "-quality", "85", "webp:-"],
            stdin=input_file, stdout=output_file, check=True,
        )
    ctx.output(str(output))
    ctx.log(f"Created {output}")
    ctx.progress(index, ctx.count, "Converting images to WebP")
