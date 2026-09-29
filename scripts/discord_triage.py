#!/usr/bin/env python3
"""Triage Strata's Discord report forums against GitHub issues."""

from __future__ import annotations

import difflib
import json
import os
import re
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from typing import Any

DISCORD_API = "https://discord.com/api/v10"
GITHUB_API = "https://api.github.com"
GUILD_ID = "1546233919609774181"
CHECKED_FOOTER = "Strata triage · checked"
LINK_MARKER = "strata-discord-thread"
STOP_WORDS = {
    "about",
    "after",
    "again",
    "cannot",
    "could",
    "does",
    "feature",
    "from",
    "have",
    "into",
    "issue",
    "request",
    "should",
    "strata",
    "that",
    "this",
    "when",
    "with",
    "would",
}


@dataclass(frozen=True)
class ForumConfig:
    channel_id: str
    github_label: str
    approval_tag: str
    completed_tag: str


FORUMS = {
    "1546248382882910360": ForumConfig(
        channel_id="1546248382882910360",
        github_label="bug",
        approval_tag="Confirmed",
        completed_tag="Fixed",
    ),
    "1546250411575484469": ForumConfig(
        channel_id="1546250411575484469",
        github_label="enhancement",
        approval_tag="Planned",
        completed_tag="Shipped",
    ),
}


def normalized_title(title: str) -> str:
    return " ".join(re.findall(r"[a-z0-9]+", title.casefold()))


def title_token(token: str) -> str:
    for suffix, minimum in (("ing", 6), ("ed", 5), ("es", 5), ("s", 4)):
        if token.endswith(suffix) and len(token) >= minimum:
            return token[: -len(suffix)]
    return token


def title_tokens(title: str) -> set[str]:
    return {
        title_token(token)
        for token in normalized_title(title).split()
        if len(token) >= 3 and token not in STOP_WORDS
    }


def similarity(left: str, right: str) -> float:
    normalized_left = normalized_title(left)
    normalized_right = normalized_title(right)
    if not normalized_left or not normalized_right:
        return 0.0
    sequence = difflib.SequenceMatcher(None, normalized_left, normalized_right).ratio()
    left_tokens = title_tokens(left)
    right_tokens = title_tokens(right)
    shared = left_tokens & right_tokens
    if not shared:
        return 0.0
    overlap = len(shared) / len(left_tokens | right_tokens)
    if len(shared) == 1 and normalized_left != normalized_right:
        return min(sequence, overlap, 0.49)
    return max(sequence, overlap)


def duplicate_candidates(
    title: str, issues: list[dict[str, Any]], threshold: float = 0.62
) -> list[dict[str, Any]]:
    candidates = []
    for issue in issues:
        if "pull_request" in issue:
            continue
        score = similarity(title, issue.get("title", ""))
        if score >= threshold:
            candidates.append({**issue, "similarity": score})
    return sorted(candidates, key=lambda issue: issue["similarity"], reverse=True)[:3]


def report_marker(thread_id: str) -> str:
    return f"<!-- {LINK_MARKER}:{thread_id} -->"


def linked_issue(thread_id: str, issues: list[dict[str, Any]]) -> dict[str, Any] | None:
    marker = report_marker(thread_id)
    return next((issue for issue in issues if marker in (issue.get("body") or "")), None)


def issue_body(thread: dict[str, Any], message: dict[str, Any]) -> str:
    content = (message.get("content") or "No text description was supplied.").strip()
    content = content.replace("@", "@\u200b")
    if len(content) > 50_000:
        content = content[:49_997].rstrip() + "..."
    author = message.get("author", {})
    reporter = author.get("global_name") or author.get("username") or "Unknown Discord user"
    thread_url = f"https://discord.com/channels/{GUILD_ID}/{thread['id']}"
    return (
        f"## Discord report\n\n{content}\n\n"
        f"## Source\n\n- Discord thread: {thread_url}\n- Reporter: {reporter}\n\n"
        f"{report_marker(thread['id'])}"
    )


class Client:
    def __init__(self, discord_token: str, github_token: str, repository: str) -> None:
        self.discord_token = discord_token
        self.github_token = github_token
        self.repository = repository

    @staticmethod
    def _request(
        url: str, headers: dict[str, str], method: str = "GET", payload: Any = None
    ) -> Any:
        data = json.dumps(payload).encode() if payload is not None else None
        request = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(request) as response:
                body = response.read()
                return json.loads(body) if body else None
        except urllib.error.HTTPError as error:
            detail = error.read().decode(errors="replace")
            raise RuntimeError(f"{method} {url} failed ({error.code}): {detail}") from error

    def discord(self, path: str, method: str = "GET", payload: Any = None) -> Any:
        return self._request(
            DISCORD_API + path,
            {
                "Authorization": f"Bot {self.discord_token}",
                "Content-Type": "application/json",
                "User-Agent": "strata-discord-triage",
            },
            method,
            payload,
        )

    def github(self, path: str, method: str = "GET", payload: Any = None) -> Any:
        return self._request(
            GITHUB_API + path,
            {
                "Accept": "application/vnd.github+json",
                "Authorization": f"Bearer {self.github_token}",
                "Content-Type": "application/json",
                "User-Agent": "strata-discord-triage",
                "X-GitHub-Api-Version": "2022-11-28",
            },
            method,
            payload,
        )

    def all_issues(self) -> list[dict[str, Any]]:
        issues: list[dict[str, Any]] = []
        page = 1
        while True:
            batch = self.github(
                f"/repos/{self.repository}/issues?state=all&sort=updated&direction=desc"
                f"&per_page=100&page={page}"
            )
            issues.extend(batch)
            if len(batch) < 100:
                break
            page += 1
        return [issue for issue in issues if "pull_request" not in issue]


def forum_tags(forum: dict[str, Any]) -> dict[str, str]:
    return {tag["name"]: tag["id"] for tag in forum.get("available_tags", [])}


def set_thread_tags(
    client: Client,
    thread: dict[str, Any],
    tags: dict[str, str],
    add: str,
    remove: tuple[str, ...],
) -> None:
    removed_ids = {tags[name] for name in remove if name in tags}
    applied = [tag for tag in thread.get("applied_tags", []) if tag not in removed_ids]
    if tags[add] not in applied:
        applied.append(tags[add])
    client.discord(f"/channels/{thread['id']}", "PATCH", {"applied_tags": applied})
    thread["applied_tags"] = applied


def has_triage_message(messages: list[dict[str, Any]], bot_id: str) -> bool:
    return any(
        message.get("author", {}).get("id") == bot_id
        and any(
            embed.get("footer", {}).get("text") == CHECKED_FOOTER
            for embed in message.get("embeds", [])
        )
        for message in messages
    )


def post_checked(
    client: Client,
    thread: dict[str, Any],
    candidates: list[dict[str, Any]],
) -> None:
    if candidates:
        lines = [
            f"• [#{issue['number']} · {issue['title']}]({issue['html_url']}) ({issue['state']})"
            for issue in candidates
        ]
        title = "Possible duplicate found"
        description = (
            "This report resembles existing GitHub issues:\n\n"
            + "\n".join(lines)
            + "\n\nA moderator can remove **Potential Duplicate** after reviewing these matches."
        )
        color = 0xE0AF68
    else:
        title = "No likely duplicate found"
        description = (
            "All open and closed GitHub issues were checked. "
            "A moderator can now mark this report **Confirmed** or **Planned** to create an issue."
        )
        color = 0x7AA2F7
    client.discord(
        f"/channels/{thread['id']}/messages",
        "POST",
        {
            "allowed_mentions": {"parse": []},
            "embeds": [
                {
                    "title": title,
                    "description": description,
                    "color": color,
                    "footer": {"text": CHECKED_FOOTER},
                }
            ],
        },
    )


def create_github_issue(
    client: Client,
    config: ForumConfig,
    thread: dict[str, Any],
    message: dict[str, Any],
) -> dict[str, Any]:
    return client.github(
        f"/repos/{client.repository}/issues",
        "POST",
        {
            "title": thread["name"],
            "body": issue_body(thread, message),
            "labels": [config.github_label, "from discord"],
        },
    )


def post_linked(client: Client, thread: dict[str, Any], issue: dict[str, Any]) -> None:
    client.discord(
        f"/channels/{thread['id']}/messages",
        "POST",
        {
            "content": f"Created GitHub issue [#{issue['number']} · {issue['title']}]({issue['html_url']}).",
            "allowed_mentions": {"parse": []},
        },
    )


def triage_thread(
    client: Client,
    bot_id: str,
    config: ForumConfig,
    forum: dict[str, Any],
    thread: dict[str, Any],
    issues: list[dict[str, Any]],
) -> dict[str, Any] | None:
    if linked_issue(thread["id"], issues):
        return None
    tags = forum_tags(forum)
    applied_names = {name for name, tag_id in tags.items() if tag_id in thread.get("applied_tags", [])}
    messages = client.discord(f"/channels/{thread['id']}/messages?limit=100")
    starter = next((message for message in messages if message["id"] == thread["id"]), None)
    if starter is None:
        return None

    candidates = duplicate_candidates(thread["name"], issues)
    if "Potential Duplicate" in applied_names:
        return None
    if candidates:
        if not has_triage_message(messages, bot_id):
            post_checked(client, thread, candidates)
        set_thread_tags(
            client,
            thread,
            tags,
            "Potential Duplicate",
            ("New", "Ready for Review", config.approval_tag),
        )
        return None

    if config.approval_tag in applied_names:
        issue = create_github_issue(client, config, thread, starter)
        post_linked(client, thread, issue)
        issues.insert(0, issue)
        return issue

    if not has_triage_message(messages, bot_id):
        post_checked(client, thread, [])
        set_thread_tags(
            client,
            thread,
            tags,
            "Ready for Review",
            ("New", "Potential Duplicate"),
        )
    return None


def run(client: Client) -> None:
    bot = client.discord("/users/@me")
    forums = {
        channel_id: client.discord(f"/channels/{channel_id}") for channel_id in FORUMS
    }
    threads = client.discord(f"/guilds/{GUILD_ID}/threads/active")["threads"]
    for channel_id in FORUMS:
        archived = client.discord(
            f"/channels/{channel_id}/threads/archived/public?limit=100"
        )["threads"]
        threads.extend(archived)
    issues = client.all_issues()
    for thread in {thread["id"]: thread for thread in threads}.values():
        config = FORUMS.get(thread.get("parent_id"))
        if config is None:
            continue
        triage_thread(client, bot["id"], config, forums[config.channel_id], thread, issues)


def main() -> None:
    client = Client(
        discord_token=os.environ["DISCORD_TRIAGE_BOT_TOKEN"],
        github_token=os.environ["GITHUB_TOKEN"],
        repository=os.environ["GITHUB_REPOSITORY"],
    )
    run(client)


if __name__ == "__main__":
    main()
