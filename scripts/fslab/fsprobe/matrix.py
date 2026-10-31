# SPDX-License-Identifier: MIT
"""Pivot fsprobe JSONL into a markdown matrix: one row per probe, one column per mount."""
import json
import sys

rows = [json.loads(line) for line in open(sys.argv[1]) if line.strip()]
labels = list(dict.fromkeys(r["label"] for r in rows))
ops = list(dict.fromkeys(r["op"] for r in rows))
cell = {(r["label"], r["op"]): r["result"] for r in rows}
fs = {r["label"]: r["fs"] for r in rows}


def short(text):
    if text.startswith("ok (") and text.endswith(")"):
        text = text[4:-1]
    return text.replace("|", "\\|").replace("\n", " ")


print("Columns (statfs type as seen by the probe):\n")
for label in labels:
    print(f"- **{label}**: {fs[label]}")
if len(sys.argv) > 2:
    print("\nMounts:\n\n```")
    print(open(sys.argv[2]).read().rstrip())
    print("```")
print()
print("| probe | " + " | ".join(labels) + " |")
print("|---|" + "---|" * len(labels))
for op in ops:
    print(f"| {short(op)} | " + " | ".join(short(cell.get((label, op), "–")) for label in labels) + " |")
