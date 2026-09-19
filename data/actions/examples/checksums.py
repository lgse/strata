# SPDX-License-Identifier: MIT

from pathlib import Path
import hashlib
import os

from strata_actions import context

ctx = context()
for index, path in enumerate(ctx.paths, start=1):
    source = Path(path)
    if not source.is_file():
        raise ValueError(f"Not a file: {source}")
    with source.open("rb") as input_file:
        digest = hashlib.sha256()
        for block in iter(lambda: input_file.read(1024 * 1024), b""):
            digest.update(block)
    # GNU sha256sum format, including escaping for unusual native file names.
    name = os.fsencode(source.name)
    escaped = name.replace(b"\\", b"\\\\").replace(b"\n", b"\\n").replace(b"\r", b"\\r")
    marker = b"\\" if escaped != name else b""
    output = Path(str(source) + ".sha256")
    # Exclusive creation refuses existing files and symlinks, even dangling ones.
    with output.open("xb") as output_file:
        output_file.write(marker + digest.hexdigest().encode("ascii") + b"  " + escaped + b"\n")
    ctx.output(str(output))
    ctx.log(f"Created {output}")
    ctx.progress(index, ctx.count, "Calculating SHA-256 checksums")
