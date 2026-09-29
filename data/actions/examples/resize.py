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
    folder = Path(tempfile.mkdtemp(prefix="strata-resize-", dir=source.parent))
    output = folder / "resized.png"
    # Fit within 1024 x 1024, preserving aspect ratio; never enlarge an image.
    with source.open("rb") as input_file, output.open("xb") as output_file:
        subprocess.run(
            [magick, "-[0]", "-auto-orient", "-resize", "1024x1024>", "png:-"],
            stdin=input_file, stdout=output_file, check=True,
        )
    ctx.output(str(output))
    ctx.log(f"Created {output}")
    ctx.progress(index, ctx.count, "Resizing images")
