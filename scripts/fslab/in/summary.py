# SPDX-License-Identifier: MIT
"""Summarize the per-mount cargo test logs written by test.sh."""
import os
import re
import sys

RESULT = re.compile(r"^test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored")
FAILED = re.compile(r"^test (\S+) \.\.\. FAILED$")
STDOUT = re.compile(r"^---- (\S+) stdout ----$")
# After these, cifs hands out its own inode numbers, so hard-link identity checks fail closed.
SERVERINO_OFF = ("Autodisabling the use of server inode numbers", "Inode number collision detected")


def reasons(lines):
    found = {}
    current = None
    for line in lines:
        match = STDOUT.match(line)
        if match:
            current = match.group(1)
            found[current] = []
            continue
        if current is None:
            continue
        if line.startswith("failures:") or line.startswith("---- "):
            current = None
            continue
        if line.startswith("note: run with") or line.startswith("stack backtrace") or not line.strip():
            continue
        if re.match(r"^\s+\d+: ", line) or line.strip().startswith("at "):
            continue
        text = re.sub(r"^thread '.*?' \(\d+\) panicked at ", "panicked at ", line.strip())
        if text.startswith("note: Some details are omitted"):
            continue
        if len(found[current]) < 4:
            found[current].append(text)
    return found


def serverino_off(out, name):
    """The run's matching kernel messages, or None when its kernel log was not read."""
    path = os.path.join(out, f"{name}.kmsg")
    if not os.path.exists(path):
        return None
    return [line for line in open(path, errors="replace").read().splitlines()
            if any(message in line for message in SERVERINO_OFF)]


def summarize(out, name):
    log = os.path.join(out, f"{name}.log")
    if os.path.exists(os.path.join(out, f"{name}.skipped")):
        return {"name": name, "verdict": "SKIPPED (not mounted)", "failures": []}
    if not os.path.exists(log):
        return {"name": name, "verdict": "NO LOG", "failures": []}
    status, seconds = "?", "?"
    status_path = os.path.join(out, f"{name}.status")
    if os.path.exists(status_path):
        status, seconds = open(status_path).read().split()
    lines = open(log, errors="replace").read().splitlines()
    # Tests that re-run the test binary in a child print their own result
    # lines; only the last result of each "Running" block is the binary's own.
    passed = failed = ignored = 0
    last = None
    for line in lines + ["     Running <end>"]:
        if line.lstrip().startswith("Running ") and last:
            passed += int(last.group(2))
            failed += int(last.group(3))
            ignored += int(last.group(4))
            last = None
        match = RESULT.match(line)
        if match:
            last = match
    failing = list(dict.fromkeys(m.group(1) for line in lines if (m := FAILED.match(line))))
    why = reasons(lines)
    if status in ("124", "137"):
        verdict = "TIMEOUT"
    elif passed + failed == 0:
        verdict = "NO TESTS RAN" if status == "0" else f"ERROR (exit {status})"
    elif failed == 0 and status == "0":
        verdict = "PASS"
    else:
        verdict = "FAIL"
    leftovers_path = os.path.join(out, f"{name}.leftovers")
    leftovers = open(leftovers_path).read().split() if os.path.exists(leftovers_path) else []
    return {
        "name": name, "verdict": verdict, "status": status, "seconds": seconds,
        "passed": passed, "failed": failed, "ignored": ignored,
        "failures": [(test, why.get(test, [])) for test in failing], "leftovers": leftovers,
        "serverino_off": serverino_off(out, name),
    }


def main():
    out, mounts = sys.argv[1], sys.argv[2:]
    meta = open(os.path.join(out, "run.txt")).read() if os.path.exists(os.path.join(out, "run.txt")) else ""
    results = [summarize(out, name) for name in mounts]
    print("# fslab test run\n")
    print("```\n" + meta.strip() + "\n```\n")
    print("| mount | verdict | passed | failed | ignored | time (s) | leftovers in TMPDIR | kernel log |")
    print("|---|---|---|---|---|---|---|---|")
    for r in results:
        kernel = r.get("serverino_off")
        kernel = "not read" if kernel is None else "**cifs server inode numbers off**" if kernel else ""
        print(f"| {r['name']} | {r['verdict']} | {r.get('passed', '')} | {r.get('failed', '')} | "
              f"{r.get('ignored', '')} | {r.get('seconds', '')} | {len(r.get('leftovers', []))} | {kernel} |")
    for r in results:
        if not r["failures"] and not r.get("leftovers") and not r.get("serverino_off"):
            continue
        print(f"\n## {r['name']}\n")
        if r.get("serverino_off"):
            print("- **kernel: cifs turned server inode numbers off during this run.** Hard links no longer "
                  "share an inode number, so failures after it on this mount may be artefacts; rerun them "
                  f"on a fresh mount. `{' / '.join(r['serverino_off'][:3])}`")
        for test, why in r["failures"]:
            reason = " / ".join(why) if why else "(no panic message captured)"
            print(f"- `{test}`: {reason}")
        if r.get("leftovers"):
            print(f"- leftovers: {', '.join(r['leftovers'][:10])}")


if __name__ == "__main__":
    main()
