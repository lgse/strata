# SPDX-License-Identifier: MIT

from contextlib import ExitStack
from pathlib import Path
from types import SimpleNamespace
import ctypes
import os
import stat

from strata_actions import context

# Never fall back to os.rename(): destinations can appear after preflight.
try:
    renameat2 = ctypes.CDLL(None, use_errno=True).renameat2
except AttributeError as error:
    raise RuntimeError("Batch rename requires Linux renameat2 support") from error
renameat2.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
renameat2.restype = ctypes.c_int
RENAME_NOREPLACE = 1

ctx = context()
with ExitStack() as handles:
    directories = {}
    sources = set()
    targets = set()
    plan = []
    for index, path in enumerate(ctx.paths, start=ctx.position or 1):
        source = Path(path)
        if source.parent not in directories:
            directory = os.open(source.parent, os.O_RDONLY | os.O_DIRECTORY)
            handles.callback(os.close, directory)
            directories[source.parent] = directory
        directory = directories[source.parent]
        parent_info = os.fstat(directory)
        parent_key = (parent_info.st_dev, parent_info.st_ino)
        source_key = (parent_key, os.fsencode(source.name))
        if source_key in sources:
            raise ValueError(f"Duplicate input: {source!s}")
        sources.add(source_key)
        info = os.stat(source.name, dir_fd=directory, follow_symlinks=False)
        if not stat.S_ISREG(info.st_mode):
            raise ValueError(f"Only regular files can be renamed: {source!s}")
        item = SimpleNamespace(
            filename=source.name, stem=source.stem, suffix=source.suffix,
            index=index, total=ctx.total or ctx.count, path=source, batch=ctx,
        )
        name = new_name(item)
        if not isinstance(name, str) or name in ("", ".", "..") or "/" in name or "\0" in name:
            raise ValueError(f"The naming function must return a single file name: {name!r}")
        target_key = (parent_key, os.fsencode(name))
        if target_key in targets:
            raise ValueError(f"Duplicate destination: {name!r}")
        targets.add(target_key)
        if name != source.name:
            try:
                os.stat(name, dir_fd=directory, follow_symlinks=False)
            except FileNotFoundError:
                pass
            else:
                raise FileExistsError(f"Destination already exists: {source.with_name(name)!s}")
        plan.append((directory, source, name))

    # Individual renames are atomic; the batch is not.
    ctx.progress(0, len(plan), "Renaming files")
    for completed, (directory, source, name) in enumerate(plan, start=1):
        if name == source.name:
            ctx.log(f"Unchanged: {source.name!r}")
        else:
            if renameat2(directory, os.fsencode(source.name), directory, os.fsencode(name), RENAME_NOREPLACE) != 0:
                error = ctypes.get_errno()
                raise OSError(error, os.strerror(error), str(source.with_name(name)))
            ctx.log(f"{source.name!r} -> {name!r}")
            ctx.output(str(source.with_name(name)))
        ctx.progress(completed, len(plan), "Renaming files")
