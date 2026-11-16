#!/usr/bin/env python3
"""Tests of the rerun gate (tools/reruns.py, tools/ci_rerun.py, its use in
`just merge`, and `just ci-wait`'s report of reruns on main; ticket #396).
GitHub is never asked: gh is a stand-in that answers from a table and
records what was posted. `just lint` runs them."""

from __future__ import annotations

import contextlib
import io
import json
import sys
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ci_rerun  # noqa: E402
import ci_wait  # noqa: E402
import merge  # noqa: E402
import reruns  # noqa: E402

RUN = "43768035550"
LINK = f"https://github.com/o/r/actions/runs/{RUN}/job/133019867940"
HEAD = "a" * 40
PR = {"number": 42, "headRefOid": HEAD, "headRefName": "fix-the-thing", "baseRefName": "main"}
SUMMARY = {"name": "CI passed", "workflow": "CI", "event": "pull_request", "bucket": "pass", "link": LINK}


def comment(body: str, who: str = "OWNER", hidden: bool = False) -> dict:
    return {"body": body, "authorAssociation": who, "isMinimized": hidden}


class FakeGh:
    """Answers gh calls from `answers` (first argument words to output, or an
    exception) and records each call."""

    def __init__(self, **answers):
        self.answers = answers
        self.calls: list[tuple[str, ...]] = []

    def __call__(self, *args: str) -> str:
        self.calls.append(args)
        for key, answer in self.answers.items():
            if " ".join(args).startswith(key.replace("_", " ")) or key in args:
                if isinstance(answer, Exception):
                    raise answer
                return answer
        return ""

    def posted(self) -> list[tuple[str, ...]]:
        return [call for call in self.calls if call[1:2] == ("comment",) or call[:2] == ("run", "rerun")]


class Records(unittest.TestCase):
    def test_a_record_is_read_from_the_first_line(self):
        self.assertEqual(reruns.parse(f"Rerun: {RUN} cause: #378\n\nRecorded."), reruns.Record(RUN, 378))

    def test_a_comment_that_is_not_a_record_records_nothing(self):
        for body in ["", "Rerun it", f"Cause below.\nRerun: {RUN} cause: #378", f"Rerun: {RUN} cause: 378",
                     f"Rerun: {RUN}: #378"]:
            self.assertIsNone(reruns.parse(body), body)

    def test_only_visible_comments_by_those_who_can_push_count(self):
        line = reruns.first_line(RUN, 378)
        comments = [comment(line, "NONE"), comment(line, "CONTRIBUTOR"), comment(line, hidden=True)]
        self.assertEqual(reruns.records_in(comments), [])
        self.assertEqual(reruns.records_in([comment(line, "COLLABORATOR")]), [reruns.Record(RUN, 378)])

    def test_the_run_is_read_from_a_checks_link(self):
        self.assertEqual(reruns.run_of(LINK), RUN)
        self.assertIsNone(reruns.run_of(""))


class Problem(unittest.TestCase):
    def test_a_first_attempt_needs_no_record(self):
        self.assertIsNone(reruns.problem(RUN, 1, []))

    def test_a_rerun_needs_a_record_naming_that_run(self):
        self.assertIn(f"run {RUN} passed on attempt 2", reruns.problem(RUN, 2, []))
        self.assertIsNotNone(reruns.problem(RUN, 2, [reruns.Record("1", 378)]))
        self.assertIsNone(reruns.problem(RUN, 3, [reruns.Record(RUN, 378)]))


class MergeRefusesAnUnrecordedRerun(unittest.TestCase):
    def gate(self, attempt: int, comments: list[dict]) -> FakeGh:
        gh = FakeGh(api=str(attempt), pr=json.dumps({"comments": comments}))
        with mock.patch.object(merge, "checks", return_value=[SUMMARY]), mock.patch.object(merge, "gh", gh):
            merge.require_no_unrecorded_rerun(PR)
        return gh

    def test_a_first_attempt_lands_without_reading_comments(self):
        gh = self.gate(1, [])
        self.assertFalse([call for call in gh.calls if call[0] == "pr"])

    def test_a_rerun_without_a_record_is_refused_with_the_next_step(self):
        with self.assertRaises(merge.Refused) as refused:
            self.gate(2, [comment("LGTM")])
        self.assertIn(f"run {RUN} passed on attempt 2", str(refused.exception))
        self.assertIn("just ci-rerun <run> <issue>", str(refused.exception))

    def test_a_rerun_with_a_record_lands(self):
        self.gate(2, [comment(reruns.first_line(RUN, 378))])

    def test_a_record_by_an_outsider_does_not_count(self):
        with self.assertRaises(merge.Refused):
            self.gate(2, [comment(reruns.first_line(RUN, 378), "NONE")])


def run_json(event: str = "pull_request", status: str = "completed", conclusion: str = "failure",
             attempt: int = 1, sha: str = HEAD) -> str:
    return json.dumps({"attempt": attempt, "headBranch": "fix-the-thing" if event == "pull_request" else "main",
                       "headSha": sha, "event": event, "status": status, "conclusion": conclusion})


ISSUE = json.dumps({"number": 378})
PRS = json.dumps([{"number": 42, "headRefOid": HEAD}])


class CiRerun(unittest.TestCase):
    def rerun(self, gh: FakeGh) -> str:
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            ci_rerun.rerun(gh, RUN, 378)
        return out.getvalue()

    def refused(self, gh: FakeGh) -> str:
        with self.assertRaises(merge.Refused) as refused:
            self.rerun(gh)
        self.assertEqual(gh.posted(), [], "a refusal posts and reruns nothing")
        return str(refused.exception)

    def test_an_issue_that_does_not_exist_is_refused(self):
        self.assertIn("#378 is not an issue", self.refused(FakeGh(api=merge.Refused("404"), run=run_json())))

    def test_a_pull_request_number_is_refused(self):
        gh = FakeGh(api=json.dumps({"number": 378, "pull_request": {}}), run=run_json())
        self.assertIn("is a pull request", self.refused(gh))

    def test_an_unfinished_run_is_refused(self):
        self.assertIn("in progress", self.refused(FakeGh(api=ISSUE, run=run_json(status="in_progress"))))

    def test_a_first_attempt_that_passed_is_refused(self):
        self.assertIn("nothing to rerun", self.refused(FakeGh(api=ISSUE, run=run_json(conclusion="success"))))

    def test_a_run_for_a_head_the_pr_has_moved_past_is_refused(self):
        gh = FakeGh(api=ISSUE, run=run_json(sha="b" * 40), pr=PRS)
        self.assertIn("the newer head has its own run", self.refused(gh))

    def test_a_red_pr_run_is_recorded_on_the_pr_and_then_rerun(self):
        gh = FakeGh(api=ISSUE, run_view=run_json(), pr_list=PRS)
        self.rerun(gh)
        (record, rerun) = gh.posted()
        self.assertEqual(record[:3], ("pr", "comment", "42"))
        self.assertEqual(reruns.parse(record[4]), reruns.Record(RUN, 378))
        self.assertEqual(rerun, ("run", "rerun", RUN, "--failed"))

    def test_a_pr_run_that_passed_on_a_later_attempt_is_recorded_and_not_rerun(self):
        gh = FakeGh(api=ISSUE, run_view=run_json(conclusion="success", attempt=2), pr_list=PRS)
        self.assertIn("nothing rerun", self.rerun(gh))
        (record,) = gh.posted()
        self.assertEqual(record[:3], ("pr", "comment", "42"))

    def test_a_red_run_on_main_is_recorded_on_the_issue(self):
        gh = FakeGh(api=ISSUE, run_view=run_json(event="push"))
        self.rerun(gh)
        (record, _) = gh.posted()
        self.assertEqual(record[:3], ("issue", "comment", "378"))
        self.assertEqual(reruns.parse(record[4]), reruns.Record(RUN, 378))


class CiWaitNamesReruns(unittest.TestCase):
    def test_only_runs_past_attempt_1_are_named(self):
        runs = [
            {"databaseId": 1, "attempt": 1, "headSha": HEAD, "conclusion": "success"},
            {"databaseId": 2, "attempt": 2, "headSha": "b" * 40, "conclusion": "success"},
            {"databaseId": 3, "attempt": 3, "headSha": "c" * 40, "conclusion": ""},
        ]
        self.assertEqual(ci_wait.main_reruns(runs), [
            "  run 2 on bbbbbbb: attempt 2, success",
            "  run 3 on ccccccc: attempt 3, not finished",
        ])


if __name__ == "__main__":
    unittest.main()
