# SPDX-License-Identifier: MIT

PATTERN = "{index:03d}_{filename}"


def new_name(context):
    """Return one file name, not a path.

    context.filename: original name, including its extension
    context.stem: original name without its last extension
    context.suffix: last extension, including the dot (or an empty string)
    context.index: 1-based position in Strata's supplied batch order
    context.total: number of selected files in the job
    context.path: original absolute pathlib.Path
    context.batch: the full Strata context() with paths, log(), progress(), etc.

    Available pattern fields: filename, stem, suffix, index, total.
    Examples: "photo_{index:04d}{suffix}" or "{stem}_edited{suffix}"
    """
    return PATTERN.format(
        filename=context.filename,
        stem=context.stem,
        suffix=context.suffix,
        index=context.index,
        total=context.total,
    )
