import unittest

from discord_triage import (
    CHECKED_FOOTER,
    FORUMS,
    duplicate_candidates,
    issue_body,
    linked_issue,
    report_marker,
    similarity,
    triage_thread,
)


class FakeClient:
    def __init__(self, messages):
        self.messages = messages
        self.discord_calls = []
        self.github_calls = []
        self.repository = "lgse/strata"

    def discord(self, path, method="GET", payload=None):
        if method == "GET":
            return self.messages
        self.discord_calls.append((path, method, payload))
        return {"id": "discord-message"}

    def github(self, path, method="GET", payload=None):
        self.github_calls.append((path, method, payload))
        return {
            "number": 42,
            "title": payload["title"],
            "html_url": "https://github.com/lgse/strata/issues/42",
            "body": payload["body"],
            "state": "open",
        }


class DiscordTriageTests(unittest.TestCase):
    def setUp(self):
        self.config = FORUMS["1546248382882910360"]
        self.tags = [
            {"name": "New", "id": "new"},
            {"name": "Potential Duplicate", "id": "duplicate"},
            {"name": "Ready for Review", "id": "ready"},
            {"name": "Confirmed", "id": "confirmed"},
            {"name": "Fixed", "id": "fixed"},
        ]
        self.forum = {"available_tags": self.tags}
        self.thread = {
            "id": "123",
            "parent_id": self.config.channel_id,
            "name": "Preview pane crashes on large PDF",
            "applied_tags": ["new"],
        }
        self.starter = {
            "id": "123",
            "content": "Open a large PDF and press Space.",
            "author": {"username": "reporter", "global_name": "Reporter"},
        }

    def test_similar_titles_require_meaningful_shared_words(self):
        self.assertGreater(similarity("Preview pane crashes", "Crash in preview pane"), 0.62)
        self.assertLess(similarity("Preview pane crashes", "Preview colors look muted"), 0.5)

    def test_duplicate_candidates_are_ranked_and_ignore_pull_requests(self):
        issues = [
            {
                "number": 1,
                "title": "Crash in preview pane for large PDFs",
                "html_url": "https://example.test/1",
                "state": "open",
            },
            {
                "number": 2,
                "title": "Preview pane crashes on large PDF",
                "html_url": "https://example.test/2",
                "state": "closed",
                "pull_request": {},
            },
        ]
        matches = duplicate_candidates(self.thread["name"], issues)
        self.assertEqual([issue["number"] for issue in matches], [1])

    def test_report_marker_finds_an_existing_link(self):
        issue = {"body": f"Imported report\n\n{report_marker('123')}"}
        self.assertIs(linked_issue("123", [issue]), issue)
        self.assertIsNone(linked_issue("other", [issue]))

    def test_issue_body_links_the_source_without_mentions(self):
        self.starter["content"] += " Please ask @maintainer."
        body = issue_body(self.thread, self.starter)
        self.assertIn("Open a large PDF", body)
        self.assertIn("@\u200bmaintainer", body)
        self.assertNotIn("@maintainer", body)
        self.assertIn("Reporter", body)
        self.assertIn("https://discord.com/channels/1546233919609774181/123", body)
        self.assertIn(report_marker("123"), body)

    def test_duplicate_is_linked_and_tagged_without_creating_an_issue(self):
        client = FakeClient([self.starter])
        issues = [
            {
                "number": 7,
                "title": "Large PDF crashes the preview pane",
                "html_url": "https://github.com/lgse/strata/issues/7",
                "state": "open",
                "body": "",
            }
        ]
        result = triage_thread(
            client, "bot", self.config, self.forum, self.thread, issues
        )
        self.assertIsNone(result)
        self.assertFalse(client.github_calls)
        posted = client.discord_calls[0][2]["embeds"][0]
        self.assertIn("#7", posted["description"])
        self.assertEqual(posted["footer"]["text"], CHECKED_FOOTER)
        self.assertEqual(client.discord_calls[1][2]["applied_tags"], ["duplicate"])

    def test_confirmed_non_duplicate_creates_and_links_a_github_issue(self):
        self.thread["applied_tags"] = ["confirmed"]
        client = FakeClient([self.starter])
        issues = []
        result = triage_thread(
            client, "bot", self.config, self.forum, self.thread, issues
        )
        self.assertEqual(result["number"], 42)
        payload = client.github_calls[0][2]
        self.assertEqual(payload["labels"], ["bug", "from discord"])
        self.assertIn(report_marker("123"), payload["body"])
        self.assertIn("Created GitHub issue", client.discord_calls[0][2]["content"])

    def test_checked_thread_waits_for_moderator_without_reposting(self):
        checked = {
            "id": "message",
            "author": {"id": "bot"},
            "embeds": [{"footer": {"text": CHECKED_FOOTER}}],
        }
        client = FakeClient([self.starter, checked])
        result = triage_thread(
            client, "bot", self.config, self.forum, self.thread, []
        )
        self.assertIsNone(result)
        self.assertFalse(client.discord_calls)
        self.assertFalse(client.github_calls)


if __name__ == "__main__":
    unittest.main()
