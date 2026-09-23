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
    folder = Path(tempfile.mkdtemp(prefix="strata-mp3-", dir=source.parent))
    output = folder / "audio.mp3"
    # Extract the first audio stream. A source without audio fails explicitly.
    subprocess.run(
        [ffmpeg, "-nostdin", "-hide_banner", "-loglevel", "error", "-n",
         "-protocol_whitelist", "file,pipe", "-i", str(source),
         "-map", "0:a:0", "-vn", "-c:a", "libmp3lame", "-q:a", "2",
         str(output)],
        check=True,
    )
    ctx.output(str(output))
    ctx.log(f"Created {output}")
    ctx.progress(index, ctx.count, "Extracting MP3 audio")
