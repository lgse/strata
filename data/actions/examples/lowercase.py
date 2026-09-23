# SPDX-License-Identifier: MIT


def new_name(context):
    """Return a filename, not a path.

    filename is the original name including its extension; index is 1-based.
    stem, suffix, total, path (pathlib.Path), and batch (Strata context) are
    also available, just as in the Batch rename recipe.
    """
    return context.filename.lower().replace(" ", "-")
