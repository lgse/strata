# SPDX-License-Identifier: MIT

from strata_actions import context

ctx = context()
for index, path in enumerate(ctx.paths, start=1):
    ctx.log(f"Processing {path}")
    ctx.progress(index, ctx.count, "Processing selected paths")
