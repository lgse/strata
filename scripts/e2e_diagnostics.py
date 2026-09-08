# SPDX-License-Identifier: GPL-3.0-or-later
"""Bounded, credential-safe explanations of upstream E2E job failures."""

from __future__ import annotations

import os
import re
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener

MAX_LOG_BYTES = 2 * 1024 * 1024
MAX_LOG_JOBS = 3
FAILED = {"failure", "cancelled", "timed_out", "action_required", "startup_failure"}


class LogRedirectHandler(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if urlsplit(newurl).scheme != "https":
            raise ValueError("refusing an insecure job-log redirect")
        redirected = super().redirect_request(req, fp, code, msg, headers, newurl)
        if redirected is not None:
            # GitHub redirects to signed blob URLs; never forward its API token.
            redirected.remove_header("Authorization")
        return redirected


def job_log(job_id: int) -> str:
    api = os.environ.get("GITHUB_API_URL", "https://api.github.com")
    repository = os.environ["GITHUB_REPOSITORY"]
    request = Request(f"{api}/repos/{repository}/actions/jobs/{int(job_id)}/logs", headers={
        "Authorization": f"Bearer {os.environ['GH_TOKEN']}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    })
    with build_opener(LogRedirectHandler()).open(request, timeout=10) as response:
        return response.read(MAX_LOG_BYTES).decode("utf-8", errors="replace")


def classify_log(log: str) -> str | None:
    log = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", log)
    log = re.sub(r"##\[group\]Run [\s\S]*?##\[endgroup\]", "", log)
    if re.search(r"(?m)^.*\bError: [^\n]+: image not known\s*$", log):
        return ("The selected container engine cannot find the loaded runtime. "
                "Use the same engine for image loading and scenario execution.")
    statuses = sorted(set(re.findall(
        r"E: Failed to fetch https?://snapshot\.ubuntu\.com/\S+[^\n]*?\s(5\d\d)\s", log)))
    if statuses:
        return (f"Ubuntu Snapshot returned HTTP {'/'.join(statuses)} while fetching pinned "
                "package metadata or packages. This is an upstream bootstrap/download failure, "
                "not a GUI assertion failure. Package verification remains enabled.")
    if re.search(r"E:.*(?:NO_PUBKEY|not signed|signatures? couldn't be verified|Hash Sum mismatch)", log):
        return "APT rejected package metadata or its integrity; do not bypass signature/checksum verification."
    codes = sorted(set(re.findall(r"\berror\[(E\d{4})\]", log)))
    if codes:
        return f"Rust compilation failed ({', '.join(codes[:5])}); inspect the linked compiler output."
    if ("E2E bundle:" in log or "published base input labels do not match" in log
            or "The runtime image was not built from this checkout's pinned E2E inputs." in log):
        return "The runner rejected the supplied bundle or image provenance. Rebuild the matching inputs; do not bypass these checks."
    if "Runtime cache did not match the exact pinned key" in log:
        return "The runtime cache did not match its exact pinned key; no substitute runtime was accepted."
    return None


def scenario_job(job: dict) -> bool:
    return bool(re.fullmatch(r"E2E shard \d+", job["name"]))


def markdown_text(value: str) -> str:
    return re.sub(r"([\\`*_{}\[\]()<>|])", r"\\\1", " ".join(value.split()))


def failure_summary(build_result: str, shard_result: str, jobs: list[dict],
                    logs: dict[int, str], unavailable: set[int]) -> tuple[str, str]:
    build_failed = build_result != "success"
    title = "E2E build/bootstrap failed" if build_failed else "E2E shard execution failed"
    lines = [f"## {title}", "", f"Build result: **{build_result}**. "
             f"Shard result: **{shard_result}**.", ""]
    shards = [job for job in jobs if scenario_job(job)]
    if build_failed and not any(job.get("conclusion") != "skipped" for job in shards):
        lines += ["**No E2E scenarios ran: the build/plan prerequisite did not succeed.**", ""]
    lines += ["| Job | Result | Unsuccessful step |", "| --- | --- | --- |"]
    failed_jobs = [job for job in jobs if
                   (job["name"] == "E2E build and plan" or scenario_job(job))
                   and job.get("conclusion") in FAILED]
    reasons = []
    for job in failed_jobs:
        steps = [step for step in job.get("steps", []) if step.get("conclusion") in FAILED]
        step_text = "; ".join(markdown_text(step["name"]) for step in steps) or "See job log"
        name = markdown_text(job["name"])
        link = job.get("html_url")
        if link and urlsplit(link).scheme == "https":
            name = f"[{name}]({link})"
        lines.append(f"| {name} | {job['conclusion']} | {step_text} |")
        if job.get("conclusion") == "timed_out":
            reasons.append("A runner job hit its timeout. Inspect its last active step; "
                           "a timeout alone does not establish an Ubuntu outage.")
        if reason := classify_log(logs.get(job["id"], "")):
            reasons.append(reason)
    if reasons:
        lines += ["", "### Observed cause", ""] + [f"- {reason}" for reason in dict.fromkeys(reasons)]
    else:
        lines += ["", "The exact cause could not be extracted automatically; open the linked "
                  "unsuccessful step. Do not infer an Ubuntu outage from exit code 1 alone."]
    if unavailable:
        lines += ["", "Some job logs were unavailable through the API; the job links remain authoritative."]
    lines += ["", "### Next steps", ""]
    if any("Ubuntu Snapshot returned" in reason for reason in reasons):
        lines += ["- If matching environments are already published, verify both GHCR packages "
                  "are publicly readable; missing access can force source bootstrap unnecessarily.",
                  "- After Ubuntu Snapshot recovers, run **Warm E2E dependencies** on the affected "
                  "branch, then rerun the **entire CI workflow**.",
                  "- Main cannot restore PR-scoped caches. A passing PR does not seed main's "
                  "first bootstrap; do not promote untrusted PR caches into main."]
    else:
        lines += ["- Inspect the unsuccessful step above and any shard JSON/JUnit/failure artifacts.",
                  "- Correct the underlying failure, then rerun the entire workflow."]
    lines += ["- Timing is informational and never fails passing tests. A long failed bootstrap "
              "is not evidence that GUI scenarios were slow.",
              "- Log inspection is bounded; follow the job links for full output."]
    return title, "\n".join(lines) + "\n"


def require_dependencies(workflow_jobs, publish_summary):
    build = os.environ.get("BUILD_RESULT", "missing")
    shards = os.environ.get("SHARD_RESULT", "missing")
    if build == shards == "success":
        print("E2E build and every shard succeeded; checking exact coverage next.")
        return
    # Only known GitHub result values enter annotations/Markdown, never raw env text.
    known = FAILED | {"success", "skipped", "missing"}
    build = build if build in known else "unknown"
    shards = shards if shards in known else "unknown"
    logs, unavailable = {}, set()
    try:
        jobs = workflow_jobs()
    except (OSError, ValueError, KeyError, TypeError):
        jobs = []
    failed = [job for job in jobs if
              (job["name"] == "E2E build and plan" or scenario_job(job))
              and job.get("conclusion") in FAILED]
    for job in failed[:MAX_LOG_JOBS]:
        try:
            logs[job["id"]] = job_log(job["id"])
        except (OSError, ValueError, KeyError, TypeError):
            unavailable.add(job["id"])
    title, summary = failure_summary(build, shards, jobs, logs, unavailable)
    if not jobs:
        summary += "\nJob metadata was unavailable; open **E2E build and plan** and the shard jobs in this attempt.\n"
    publish_summary(summary)
    cause = next((reason for log in logs.values() if (reason := classify_log(log))), None)
    detail = cause or "See the E2E failure summary for failed steps and log links."
    print(f"::error title={title}::{title}. Build={build}; shards={shards}. {detail}")
    raise ValueError(f"{title}; see the E2E failure summary")
