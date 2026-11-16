#!/usr/bin/env python3
"""Tests of the nightly run's decisions (tools/nightly.py) and of where its
jobs live (.github/workflows/nightly.yml, not ci.yml). GitHub is never
asked: gh is a stand-in that answers from a table and records what was
posted. `just lint` runs them."""

from __future__ import annotations

import contextlib
import io
import json
import re
import sys
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import nightly  # noqa: E402

REPOSITORY = Path(__file__).resolve().parent.parent
CI = (REPOSITORY / ".github/workflows/ci.yml").read_text()
NIGHTLY = (REPOSITORY / ".github/workflows/nightly.yml").read_text()

HEAD = "a" * 40
GREEN = "b" * 40
RUN_URL = "https://github.com/o/r/actions/runs/43800000001"
GREEN_URL = "https://github.com/o/r/actions/runs/43700000001"
JOBS = [
    {"name": "Decide", "conclusion": "success", "url": f"{RUN_URL}/job/1"},
    {"name": "just ci-checklist-group-6 group_6_times", "conclusion": "failure", "url": f"{RUN_URL}/job/2"},
    {"name": "just ci-checklist-group-6 step_6_2_", "conclusion": "success", "url": f"{RUN_URL}/job/3"},
    {"name": "just scorecard", "conclusion": "timed_out", "url": f"{RUN_URL}/job/4"},
    {"name": "Report a red night", "conclusion": "", "url": f"{RUN_URL}/job/5"},
]


class Decide(unittest.TestCase):
    def test_a_scheduled_night_runs_only_when_main_moved_since_the_last_green_one(self):
        self.assertFalse(nightly.should_run("schedule", HEAD, HEAD))
        self.assertTrue(nightly.should_run("schedule", HEAD, GREEN))

    def test_a_first_night_runs(self):
        self.assertTrue(nightly.should_run("schedule", HEAD, None))

    def test_a_dispatched_night_always_runs(self):
        self.assertTrue(nightly.should_run("workflow_dispatch", HEAD, HEAD))

    def test_it_names_the_last_green_run_for_the_report(self):
        green = {"headSha": GREEN, "url": GREEN_URL, "databaseId": 1}
        with mock.patch.object(nightly, "last_green", return_value=green), \
                contextlib.redirect_stderr(io.StringIO()):
            lines = nightly.decide("schedule", HEAD)
        self.assertEqual(lines, ["run=true", f"since={GREEN}", f"since_run={GREEN_URL}"])


def artifact(sha: str, branch: str = "main", expired: bool = False, run: int = 1) -> dict:
    return {"expired": expired, "created_at": f"2026-11-16T0{run}:00:00Z",
            "workflow_run": {"id": run, "head_branch": branch, "head_sha": sha}}


class Results(unittest.TestCase):
    def test_the_newest_run_on_main_at_or_before_head_is_scored(self):
        artifacts = [
            artifact("c" * 40, branch="fix-the-thing", run=4),
            artifact("d" * 40, run=3),  # on main, but not in HEAD's history
            artifact(HEAD, expired=True, run=2),
            artifact(GREEN, run=1),
        ]
        run = nightly.results_run(artifacts, lambda sha: sha in (HEAD, GREEN))
        self.assertEqual(run["id"], 1)

    def test_newest_is_by_when_it_was_kept_whatever_order_github_lists(self):
        artifacts = [artifact(GREEN, run=1), artifact(HEAD, run=2)]
        self.assertEqual(nightly.results_run(artifacts, lambda sha: True)["id"], 2)

    def test_none_kept_is_none(self):
        self.assertIsNone(nightly.results_run([], lambda sha: True))

    def test_with_none_kept_the_night_fails_saying_what_to_do(self):
        def gh(*args: str) -> str:
            if args[:2] == ("run", "list"):
                return json.dumps([{"databaseId": 7, "status": "completed"}])
            return json.dumps({"artifacts": []})
        with mock.patch.object(nightly, "gh", gh), self.assertRaises(nightly.Failed) as failed:
            nightly.results(HEAD)
        self.assertIn("Next: gh workflow run CI --ref main", str(failed.exception))


class Body(unittest.TestCase):
    def test_it_names_the_run_the_failed_jobs_and_the_commits_since_the_last_green_night(self):
        text = nightly.body(RUN_URL, HEAD, nightly.failed_jobs(JOBS), GREEN, GREEN_URL,
                            ["1234567 Quiet Hours: a thing"])
        self.assertIn(f"Nightly run {RUN_URL} failed on main at aaaaaaa.", text)
        self.assertIn(f"- just ci-checklist-group-6 group_6_times: {RUN_URL}/job/2", text)
        self.assertIn(f"- just scorecard: {RUN_URL}/job/4", text)
        self.assertNotIn("step_6_2_", text)
        self.assertNotIn("Decide", text)
        self.assertIn(f"Commits since the last green nightly ({GREEN_URL}, at bbbbbbb):", text)
        self.assertIn("- 1234567 Quiet Hours: a thing", text)
        self.assertIn("Next: just ci-wait 43800000001", text)

    def test_no_green_night_yet_and_no_new_commits_are_each_said(self):
        self.assertIn("No nightly has been green yet", nightly.body(RUN_URL, HEAD, [], "", "", []))
        self.assertIn("No commits since", nightly.body(RUN_URL, HEAD, [], GREEN, GREEN_URL, []))


class Plan(unittest.TestCase):
    ISSUE = {"number": 430, "title": nightly.TITLE, "body": "Nightly run https://x/runs/1 failed"}

    def test_with_no_open_issue_it_opens_one(self):
        other = {"number": 9, "title": "Something else", "body": ""}
        self.assertIsNone(nightly.open_issue_of([other]))
        self.assertEqual(nightly.plan(None, [], RUN_URL), ("open", None))

    def test_with_one_open_it_comments_whatever_its_labels_now(self):
        self.assertEqual(nightly.open_issue_of([self.ISSUE]), self.ISSUE)
        self.assertEqual(nightly.plan(self.ISSUE, [{"body": "older night"}], RUN_URL), ("comment", 430))

    def test_a_run_already_posted_is_not_posted_again(self):
        self.assertEqual(nightly.plan(self.ISSUE, [{"body": f"Nightly run {RUN_URL} failed"}], RUN_URL),
                         ("none", 430))
        opened_by_it = dict(self.ISSUE, body=f"Nightly run {RUN_URL} failed")
        self.assertEqual(nightly.plan(opened_by_it, [], RUN_URL), ("none", 430))


class FakeGh:
    """Answers gh's reads from a table and records every call."""

    def __init__(self, issues: list[dict], comments: list[dict] | None = None):
        self.calls: list[tuple[str, ...]] = []
        self.issues, self.comments = issues, comments or []

    def __call__(self, *args: str) -> str:
        self.calls.append(args)
        if args[:2] == ("run", "view"):
            return json.dumps({"url": RUN_URL, "jobs": JOBS})
        if args[:2] == ("issue", "list"):
            return json.dumps(self.issues)
        if args[:2] == ("issue", "view"):
            return json.dumps({"comments": self.comments})
        if args[:2] == ("issue", "create"):
            return "https://github.com/o/r/issues/431\n"
        if args[:2] == ("issue", "comment"):
            return ""
        raise AssertionError(f"unexpected gh call {args}")

    def posts(self) -> list[tuple[str, ...]]:
        return [call for call in self.calls if call[:2] in (("issue", "create"), ("issue", "comment"))]


class Report(unittest.TestCase):
    def report(self, gh: FakeGh, dry_run: bool = False) -> tuple[list[str], str]:
        err = io.StringIO()
        with mock.patch.object(nightly, "gh", gh), \
                mock.patch.object(nightly, "commits_since", return_value=["1234567 a commit"]), \
                contextlib.redirect_stderr(err):
            lines = nightly.report("43800000001", HEAD, GREEN, GREEN_URL, dry_run)
        return lines, err.getvalue()

    def test_a_red_night_opens_the_issue_labelled_needs_triage(self):
        gh = FakeGh([])
        lines, _ = self.report(gh)
        self.assertEqual(lines, ["action=open"])
        [post] = gh.posts()
        self.assertEqual(post[:6], ("issue", "create", "--title", nightly.TITLE, "--label", "needs-triage"))
        self.assertIn("group_6_times", post[-1])

    def test_with_one_open_a_red_night_comments_on_it(self):
        gh = FakeGh([{"number": 430, "title": nightly.TITLE, "body": "an older night"}])
        self.report(gh)
        [post] = gh.posts()
        self.assertEqual(post[:3], ("issue", "comment", "430"))

    def test_the_report_run_again_posts_nothing_new(self):
        gh = FakeGh([{"number": 430, "title": nightly.TITLE, "body": ""}], [{"body": f"Nightly run {RUN_URL}"}])
        lines, said = self.report(gh)
        self.assertEqual((lines, gh.posts()), (["action=none"], []))
        self.assertIn("already names", said)

    def test_a_dry_run_posts_nothing_and_prints_what_it_would(self):
        gh = FakeGh([])
        lines, said = self.report(gh, dry_run=True)
        self.assertEqual((lines, gh.posts()), (["action=open"], []))
        self.assertIn('Dry run: would open the issue "Nightly CI is red", labelled needs-triage', said)
        self.assertIn("- 1234567 a commit", said)


def jobs_of(workflow: str) -> set[str]:
    return set(re.findall(r"(?m)^  ([a-z][a-z0-9-]*):$", workflow.split("\njobs:\n", 1)[1]))


class WhereTheJobsRun(unittest.TestCase):
    def test_the_nightly_runs_on_a_schedule_and_by_hand_only(self):
        trigger = NIGHTLY.split("\njobs:")[0]
        self.assertRegex(trigger, r"(?m)^  schedule:$")
        self.assertRegex(trigger, r'(?m)^    - cron: "[^"]+"$')
        self.assertRegex(trigger, r"(?m)^  workflow_dispatch:")
        self.assertNotRegex(trigger, r"(?m)^  (push|pull_request):")

    def test_checklist_group_6_and_the_scorecard_run_nightly_and_not_in_ci(self):
        self.assertTrue({"checklist", "scorecard", "report"} <= jobs_of(NIGHTLY))
        self.assertFalse({"checklist", "scorecard"} & jobs_of(CI))
        self.assertNotIn("ci-checklist-group-6", CI)
        self.assertNotIn("automation scorecard", CI)

    def test_ci_passed_waits_for_no_job_that_moved(self):
        needs = re.search(r"(?m)^  ci-passed:\n(?:    .*\n)*?    needs: \[([^\]]*)\]", CI)
        self.assertIsNotNone(needs)
        self.assertNotIn("checklist", needs.group(1))

    def test_ci_on_main_still_keeps_the_scenario_results_the_nightly_scorecard_reads(self):
        self.assertIn(f"name: {nightly.RESULTS_ARTIFACT[:-2]}-${{{{ matrix.shard }}}}", CI)
        self.assertIn("pattern: scenario-results-*", NIGHTLY)

    def test_a_red_night_is_reported(self):
        report = NIGHTLY.split("\n  report:\n", 1)[1]
        self.assertIn("failure()", report.split("\n    steps:")[0])
        self.assertIn("python3 tools/nightly.py report", report)
        self.assertIn("issues: write", report)


if __name__ == "__main__":
    unittest.main()
