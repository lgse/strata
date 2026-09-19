# SPDX-License-Identifier: MIT

from pathlib import Path
import subprocess
import tempfile

from strata_actions import context, require_tool

ctx = context()
ffmpeg = require_tool("ffmpeg")
for index, path in enumerate(ctx.paths, start=1):
    source = Path(path)
    if not source.is_file():
        raise ValueError(f"Not a file: {source}")
    folder = Path(tempfile.mkdtemp(prefix="strata-mp4-", dir=source.parent))
    output = folder / "converted.mp4"
    # H.264 with yuv420p requires even dimensions.
    subprocess.run(
        [ffmpeg, "-nostdin", "-hide_banner", "-loglevel", "error", "-n",
         "-protocol_whitelist", "file,pipe", "-i", str(source),
         "-map", "0:v:0", "-map", "0:a:0?",
         "-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2",
         "-c:v", "libx264", "-crf", "23", "-preset", "medium",
         "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k",
         "-movflags", "+faststart", str(output)],
        check=True,
    )
    ctx.output(str(output))
    ctx.log(f"Created {output}")
    ctx.progress(index, ctx.count, "Converting videos to MP4")
