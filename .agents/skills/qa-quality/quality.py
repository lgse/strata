#!/usr/bin/env python3
"""Scores QA scenarios with the qa-quality rubric and edits their `quality` header field.

    python3 .agents/skills/qa-quality/quality.py score 4 3 4 4
    python3 .agents/skills/qa-quality/quality.py write FILE 4 3 4 4 "behaviour; deciding factor"
    python3 .agents/skills/qa-quality/quality.py similar FILE [--top 8]
    python3 .agents/skills/qa-quality/quality.py report [FOLDER...]

`write` replaces the header's `quality` value in place, or adds it as the last
key, and changes no other byte of the file. `similar` ranks other scenarios by
shared steps. `report` prints every recorded score as tab-separated rows.
"""

import argparse
import glob
import json
import os
import re
import tempfile
import sys
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[3]
SCENARIOS = REPOSITORY / "tests/qa/scenarios"
RISK_WEIGHTS = {"impact": 3, "criticality": 3, "diagnostic": 2}
DIMENSIONS = ("impact", "coverage", "criticality", "diagnostic")
SPACE = re.compile(r"[ \t\r\n]*")
IGNORED_STEP_KEYS = {"message", "timeout_ms"}


def score(impact: int, coverage: int, criticality: int, diagnostic: int) -> int:
    for value in (impact, coverage, criticality, diagnostic):
        if not 1 <= value <= 5:
            raise ValueError("every dimension is an integer from 1 to 5")
    risk = (
        RISK_WEIGHTS["impact"] * (impact - 1)
        + RISK_WEIGHTS["criticality"] * (criticality - 1)
        + RISK_WEIGHTS["diagnostic"] * (diagnostic - 1)
    )
    return (50 * (coverage - 1) * risk + 32) // 64


def value_spans(header: str) -> tuple[dict[str, tuple[int, int]], int]:
    decoder = json.JSONDecoder()
    index = SPACE.match(header, 0).end()
    if header[index:index + 1] != "{":
        raise ValueError("the header is not a JSON object")
    index = SPACE.match(header, index + 1).end()
    spans: dict[str, tuple[int, int]] = {}
    if header[index:index + 1] == "}":
        return spans, index
    while True:
        key, index = decoder.raw_decode(header, index)
        index = SPACE.match(header, index).end()
        if header[index:index + 1] != ":":
            raise ValueError("the header is not a JSON object")
        start = SPACE.match(header, index + 1).end()
        _, end = decoder.raw_decode(header, start)
        spans[key] = (start, end)
        index = SPACE.match(header, end).end()
        if header[index:index + 1] == "}":
            return spans, index
        if header[index:index + 1] != ",":
            raise ValueError("the header is not a JSON object")
        index = SPACE.match(header, index + 1).end()


def with_quality(header: str, quality: dict) -> str:
    text = json.dumps(quality, separators=(",", ":"), ensure_ascii=False)
    spans, close = value_spans(header)
    if "quality" in spans:
        start, end = spans["quality"]
        updated = header[:start] + text + header[end:]
    else:
        updated = header[:close] + ("," if spans else "") + '"quality":' + text + header[close:]
    if json.loads(updated) != {**json.loads(header), "quality": quality}:
        raise ValueError("rewriting quality changed something else")
    return updated


def write(path: Path, impact: int, coverage: int, criticality: int, diagnostic: int, note: str) -> None:
    note = note.strip()
    problem = note_problem(note)
    if problem:
        raise ValueError(problem)
    quality = {
        "impact": impact,
        "coverage": coverage,
        "criticality": criticality,
        "diagnostic": diagnostic,
        "score": score(impact, coverage, criticality, diagnostic),
        "note": note,
    }
    data = path.read_bytes()
    newline = data.find(b"\n")
    first, rest = (data, b"") if newline < 0 else (data[:newline], data[newline:])
    updated = with_quality(first.decode("utf-8"), quality).encode("utf-8")
    replace_atomically(path, updated + rest)
    print(f"{path}\t{quality['score']}")


def note_problem(note: object) -> str | None:
    if not isinstance(note, str) or not note.strip():
        return "the note is a non-empty string"
    if note.splitlines() != [note]:
        return "the note is one line: no character that str.splitlines() breaks on"
    return None


def quality_problem(quality: object) -> str | None:
    if not isinstance(quality, dict):
        return "quality is not an object"
    expected = {*DIMENSIONS, "score", "note"}
    if set(quality) != expected:
        return f"keys {sorted(quality)} are not {sorted(expected)}"
    for key in (*DIMENSIONS, "score"):
        value = quality[key]
        if type(value) is not int:
            return f"{key} is not an integer"
    if not all(1 <= quality[key] <= 5 for key in DIMENSIONS):
        return "a dimension is outside 1-5"
    expected_score = score(*(quality[key] for key in DIMENSIONS))
    if quality["score"] != expected_score:
        return f"score {quality['score']} should be {expected_score}"
    return note_problem(quality["note"])


def replace_atomically(path: Path, data: bytes) -> None:
    mode = path.stat().st_mode & 0o7777
    handle, temporary = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(handle, "wb") as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    except BaseException:
        Path(temporary).unlink(missing_ok=True)
        raise


def steps(path: Path) -> set[str]:
    found = set()
    for line in path.read_text(encoding="utf-8").splitlines()[1:]:
        if not line.strip():
            continue
        step = json.loads(line)
        if isinstance(step, dict):
            step = {k: v for k, v in step.items() if k not in IGNORED_STEP_KEYS}
        found.add(json.dumps(step, sort_keys=True))
    return found


def folder_path(folder: str) -> Path:
    for candidate in (SCENARIOS / folder, Path(folder)):
        if candidate.is_dir():
            return candidate.resolve()
    raise ValueError(f"no scenario folder {folder!r} (neither {SCENARIOS / folder} nor ./{folder})")


def files(folders: list[str]) -> list[Path]:
    if not folders:
        return sorted(Path(p) for p in glob.glob(f"{SCENARIOS}/*/*.jsonl"))
    return sorted(path for folder in folders for path in folder_path(folder).glob("*.jsonl"))


def similar(path: Path, top: int) -> None:
    mine = steps(path)
    ranked = []
    for other in files([]):
        if other.resolve() == path.resolve():
            continue
        theirs = steps(other)
        shared = len(mine & theirs)
        if shared:
            ranked.append((shared / len(mine | theirs), shared, len(mine), other))
    ranked.sort(key=lambda row: (-row[0], str(row[3])))
    print("jaccard\tshared/mine\tscenario")
    for jaccard, shared, total, other in ranked[:top]:
        print(f"{jaccard:.2f}\t{shared}/{total}\t{other.relative_to(SCENARIOS)}")


def report(folders: list[str]) -> None:
    paths = files(folders)
    print("score\timpact\tcoverage\tcriticality\tdiagnostic\tscenario\tnote")
    for path in paths:
        with path.open(encoding="utf-8") as handle:
            header = json.loads(handle.readline())
        quality = header.get("quality")
        try:
            name = path.relative_to(SCENARIOS.resolve())
        except ValueError:
            name = path
        if quality is None:
            print(f"-\t-\t-\t-\t-\t{name}\t(unscored)")
            continue
        problem = quality_problem(quality)
        if problem:
            print(f"!\t!\t!\t!\t!\t{name}\t(malformed quality: {problem})")
            continue
        print(
            f"{quality['score']}\t{quality['impact']}\t{quality['coverage']}\t"
            f"{quality['criticality']}\t{quality['diagnostic']}\t{name}\t{quality['note']}"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    scoring = commands.add_parser("score", help="print the score for four dimension values")
    for dim in DIMENSIONS:
        scoring.add_argument(dim, type=int)
    writing = commands.add_parser("write", help="record the quality field in a scenario header")
    writing.add_argument("file", type=Path)
    for dim in DIMENSIONS:
        writing.add_argument(dim, type=int)
    writing.add_argument("note")
    near = commands.add_parser("similar", help="rank scenarios that share steps with FILE")
    near.add_argument("file", type=Path)
    near.add_argument("--top", type=int, default=8)
    listing = commands.add_parser("report", help="list recorded scores as TSV")
    listing.add_argument("folders", nargs="*")
    args = parser.parse_args()
    try:
        if args.command == "score":
            print(score(args.impact, args.coverage, args.criticality, args.diagnostic))
        elif args.command == "write":
            write(args.file, args.impact, args.coverage, args.criticality, args.diagnostic, args.note)
        elif args.command == "similar":
            similar(args.file, args.top)
        else:
            report(args.folders)
    except (ValueError, OSError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
