# SPDX-License-Identifier: MIT

import io
import unittest
from unittest.mock import Mock, patch
from urllib.request import Request

from e2e_diagnostics import (LogRedirectHandler, MAX_LOG_BYTES, classify_log,
                             failure_summary, job_log, require_dependencies)


def failed_job(index=1, name="E2E build and plan", conclusion="failure"):
    return {"id": index, "name": name, "conclusion": conclusion,
            "html_url": f"https://github.com/example/strata/actions/runs/10/job/{index}",
            "steps": [{"name": "Build the tested revision", "conclusion": conclusion}]}


class ClassificationTests(unittest.TestCase):
    def test_rendered_commands_are_not_mistaken_for_executed_errors(self):
        command = "##[group]Run echo 'Runtime cache did not match its exact pinned key'\n##[endgroup]\n"
        self.assertIsNone(classify_log(command))
        reason = classify_log(command + "Error: strata-e2e:ci-runtime: image not known\n")
        self.assertIn("same engine", reason)
        self.assertNotIn("cache did not match", reason)

    def test_snapshot_errors_are_attributed_to_bootstrap_not_gui_assertions(self):
        log = "\n".join(
            f"E: Failed to fetch https://snapshot.ubuntu.com/ubuntu/date/dists/noble/InRelease  {code} Error"
            for code in (503, 500, 502, 503))
        reason = classify_log(log)
        self.assertIn("HTTP 500/502/503", reason)
        self.assertIn("not a GUI assertion", reason)

    def test_commands_urls_and_unrelated_ubuntu_errors_are_not_outage_evidence(self):
        for log in ("RUN apt-get update https://snapshot.ubuntu.com/ubuntu/date",
                    "error: exit code 100", "some request returned 503",
                    "E: Failed to fetch https://archive.ubuntu.com/ubuntu/pkg  503 Error",
                    "E: Failed to fetch https://snapshot.ubuntu.com.evil.test/pkg  503 Error"):
            with self.subTest(log=log):
                self.assertIsNone(classify_log(log))

    def test_compiler_integrity_and_provenance_failures_are_distinct(self):
        self.assertIn("E0382", classify_log("error[E0382]: use of moved value"))
        self.assertIn("integrity", classify_log("E: Hash Sum mismatch"))
        self.assertIn("bundle", classify_log("E2E bundle: source differs"))


class SummaryTests(unittest.TestCase):
    def test_build_failure_explains_no_tests_and_links_to_the_failed_step(self):
        job = failed_job()
        title, summary = failure_summary("failure", "skipped", [job], {1:
            "E: Failed to fetch http://snapshot.ubuntu.com/ubuntu/date/pkg  502 Bad Gateway"}, set())
        self.assertIn("bootstrap", title)
        self.assertIn("No E2E scenarios ran", summary)
        self.assertIn(job["html_url"], summary)
        self.assertIn("Build the tested revision", summary)
        self.assertIn("Main cannot restore PR-scoped caches", summary)
        self.assertIn("entire CI workflow", summary)

    def test_failed_shards_are_not_reported_as_zero_executed_scenarios(self):
        job = failed_job(2, "E2E shard 0")
        title, summary = failure_summary("success", "failure", [job], {}, {2})
        self.assertIn("shard execution", title)
        self.assertNotIn("No E2E scenarios ran", summary)
        self.assertIn("logs were unavailable", summary)
        self.assertIn("could not be extracted", summary)

    def test_timeout_without_log_evidence_does_not_claim_an_ubuntu_outage(self):
        _, summary = failure_summary("timed_out", "skipped",
                                     [failed_job(conclusion="timed_out")], {}, set())
        self.assertIn("timeout alone does not establish", summary)
        self.assertNotIn("Ubuntu Snapshot returned", summary)

    def test_success_needs_no_diagnostic_api_calls(self):
        jobs, publish = Mock(), Mock()
        with patch.dict("os.environ", {"BUILD_RESULT": "success", "SHARD_RESULT": "success"}), \
             patch("builtins.print"):
            require_dependencies(jobs, publish)
        jobs.assert_not_called()
        publish.assert_not_called()

    def test_log_requests_are_bounded_but_all_failed_jobs_are_linked(self):
        jobs = [failed_job(index, f"E2E shard {index}") for index in range(5)]
        publish = Mock()
        with patch.dict("os.environ", {"BUILD_RESULT": "success", "SHARD_RESULT": "failure"}), \
             patch("e2e_diagnostics.job_log", return_value="") as logs, patch("builtins.print"), \
             self.assertRaises(ValueError):
            require_dependencies(lambda: jobs, publish)
        self.assertEqual(logs.call_count, 3)
        for job in jobs:
            self.assertIn(job["html_url"], publish.call_args.args[0])

    def test_api_failure_still_fails_with_an_actionable_summary(self):
        publish = Mock()
        with patch.dict("os.environ", {"BUILD_RESULT": "failure", "SHARD_RESULT": "skipped"}), \
             patch("builtins.print"), self.assertRaises(ValueError):
            require_dependencies(Mock(side_effect=OSError("unavailable")), publish)
        self.assertIn("Job metadata was unavailable", publish.call_args.args[0])


class LogTransportTests(unittest.TestCase):
    def test_redirect_strips_github_credentials_before_contacting_blob_storage(self):
        request = Request("https://api.github.com/repos/example/strata/actions/jobs/1/logs",
                          headers={"Authorization": "Bearer test-only"})
        redirected = LogRedirectHandler().redirect_request(
            request, None, 302, "Found", {}, "https://example.blob.core.windows.net/log?sig=test-only")
        self.assertIsNone(redirected.get_header("Authorization"))
        self.assertEqual(request.get_header("Authorization"), "Bearer test-only")
        with self.assertRaisesRegex(ValueError, "insecure"):
            LogRedirectHandler().redirect_request(request, None, 302, "Found", {}, "http://example.test/log")

    def test_log_fetch_has_a_size_limit_and_api_timeout(self):
        response = io.BytesIO(b"x" * (MAX_LOG_BYTES + 1))
        opener = Mock()
        opener.open.return_value = response
        with patch.dict("os.environ", {"GITHUB_REPOSITORY": "example/strata", "GH_TOKEN": "test-only",
                                       "GITHUB_API_URL": "https://api.github.com"}), \
             patch("e2e_diagnostics.build_opener", return_value=opener):
            self.assertEqual(len(job_log(123)), MAX_LOG_BYTES)
        self.assertEqual(opener.open.call_args.kwargs["timeout"], 10)
        self.assertTrue(opener.open.call_args.args[0].full_url.endswith("/actions/jobs/123/logs"))


if __name__ == "__main__":
    unittest.main()
