#!/usr/bin/env python3
"""Tests of `just review-audit` and the "Reviews recorded" check it posts
(tools/review_audit.py), and of `just merge` leaving that check to its own
reading of the records. GitHub is never asked. `just lint` runs them."""

from __future__ import annotations

import contextlib
import io
import json
import sys
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import merge  # noqa: E402
import review_audit  # noqa: E402

HEAD = "a" * 40
PR = {"number": 42, "headRefOid": HEAD, "headRefName": "fix-the-thing", "baseRefName": "main"}
CHECK_RUNS = f"repos/{{owner}}/{{repo}}/commits/{HEAD}/check-runs?check_name=Reviews%20recorded"


class FakeGh:
    """Answers the look for an existing check run with `existing` ids, and
    records every call."""

    def __init__(self, existing: str = ""):
        self.existing = existing
        self.calls: list[tuple[str, ...]] = []

    def __call__(self, *args: str) -> str:
        self.calls.append(args)
        if args[:2] == ("api", CHECK_RUNS):
            return self.existing
        if args[:3] in (("api", "--method", "POST"), ("api", "--method", "PATCH")):
            return "{}"
        raise AssertionError(f"unexpected gh call: {args}")

    def fields(self) -> dict[str, str]:
        (write,) = [call for call in self.calls if call[1:2] == ("--method",)]
        return dict(value.split("=", 1) for flag, value in zip(write, write[1:]) if flag == "-f")


def post(gh: FakeGh, needed: tuple[str, ...], missing: list[str]) -> None:
    with contextlib.redirect_stdout(io.StringIO()):
        review_audit.post_check(gh, HEAD, needed, missing)


class TheCheck(unittest.TestCase):
    def test_a_reviewed_head_gets_a_completed_success(self):
        gh = FakeGh()
        post(gh, ("tooling",), [])
        fields = gh.fields()
        self.assertEqual(gh.calls[-1][:4], ("api", "--method", "POST", "repos/{owner}/{repo}/check-runs"))
        self.assertEqual((fields["name"], fields["head_sha"], fields["status"], fields["conclusion"]),
                         ("Reviews recorded", HEAD, "completed", "success"))

    def test_a_head_missing_a_review_gets_a_completed_failure_naming_what_and_the_next_step(self):
        gh = FakeGh()
        post(gh, ("standards-spec", "spec"), ["spec: none recorded."])
        fields = gh.fields()
        self.assertEqual((fields["status"], fields["conclusion"]), ("completed", "failure"))
        self.assertIn("spec: none recorded.", fields["output[summary]"])
        self.assertIn("just review-record", fields["output[summary]"])

    def test_a_head_that_has_the_check_gets_it_replaced_not_a_second_one(self):
        gh = FakeGh(existing="1234\n")
        post(gh, ("tooling",), [])
        self.assertEqual(gh.calls[-1][:4], ("api", "--method", "PATCH", "repos/{owner}/{repo}/check-runs/1234"))
        self.assertNotIn("head_sha", gh.fields())


class TheAudit(unittest.TestCase):
    def audit(self, verdicts: dict[int, list[str]], *flags: str) -> tuple[int, str]:
        def verdict(number):
            return "merged", HEAD, ("tooling",), verdicts[number]

        out = io.StringIO()
        with (mock.patch.object(review_audit, "verdict", verdict),
              mock.patch.object(review_audit, "post_check", lambda *_: None),
              mock.patch.object(sys, "argv", ["review_audit.py", *flags, *map(str, verdicts)]),
              contextlib.redirect_stdout(out)):
            status = review_audit.main()
        return status, out.getvalue()

    def test_one_line_per_pr_and_exit_1_if_any_lacks_a_review(self):
        status, out = self.audit({1: [], 2: ["tooling: none recorded."]})
        self.assertEqual(status, 1)
        self.assertEqual(out.splitlines(), [
            "#1 (merged, aaaaaaa): reviewed, tooling passed.",
            "#2 (merged, aaaaaaa): NOT reviewed. tooling: none recorded.",
        ])

    def test_posting_the_check_succeeds_whatever_the_verdict(self):
        self.assertEqual(self.audit({2: ["tooling: none recorded."]}, "--post-check")[0], 0)


class MergeLeavesTheCheckToItsOwnReading(unittest.TestCase):
    def test_the_reviews_check_and_its_workflow_are_neither_waited_for_nor_judged(self):
        listed = [
            {"name": "CI passed", "workflow": "CI", "event": "pull_request", "bucket": "pass", "state": "SUCCESS", "link": ""},
            {"name": "Reviews recorded", "workflow": "", "event": "", "bucket": "fail", "state": "FAILURE", "link": ""},
            {"name": "Judge the review records", "workflow": "Reviews", "event": "pull_request", "bucket": "pending",
             "state": "IN_PROGRESS", "link": ""},
        ]
        runs = [{"name": "Reviews", "event": "issue_comment", "status": "in_progress", "url": ""}]
        result = mock.Mock(stdout=json.dumps(listed), stderr="")
        with (mock.patch.object(merge.subprocess, "run", return_value=result),
              mock.patch.object(merge, "gh", return_value=json.dumps(runs))):
            found = merge.checks(PR)
        self.assertEqual([check["name"] for check in found], ["CI passed"])


if __name__ == "__main__":
    unittest.main()
