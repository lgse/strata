#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Distinguish a failed scenario assertion from a broken mutation test run."""

import sys
import xml.etree.ElementTree as ET
from pathlib import Path


def detected(report: Path, exit_code: int) -> bool:
    if exit_code != 1 or not report.is_file():
        return False
    try:
        root = ET.parse(report).getroot()
    except (ET.ParseError, OSError):
        return False
    return bool(root.findall(".//testcase/failure")) and not root.findall(
        ".//testcase/error"
    )


if __name__ == "__main__":
    sys.exit(0 if detected(Path(sys.argv[1]), int(sys.argv[2])) else 1)
