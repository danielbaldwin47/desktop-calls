#!/usr/bin/env python3
"""Tests of how `just merge` says to bring a PR up to date (tools/merge.py,
tickets #181 and #247), and of the lock its final step takes. GitHub is never
asked. `just lint` runs them."""

from __future__ import annotations

import contextlib
import fcntl
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import merge  # noqa: E402

PR = {"number": 42, "headRefOid": "a" * 40, "headRefName": "fix-the-thing", "baseRefName": "main"}
HERE = "/work/here"


def compare(behind: int, files: list[str]) -> str:
    commits = [{"sha": f"{n:040x}", "subject": f"Commit {n}"} for n in range(behind)]
    return json.dumps({"behind": behind, "commits": commits, "files": files})


def worktrees(*branches: tuple[str, str]) -> str:
    """`git worktree list --porcelain` for (directory, branch) pairs."""
    return "\n".join(f"worktree {path}\nHEAD {'b' * 40}\nbranch refs/heads/{branch}\n" for path, branch in branches)


def no_real_git(*args, **_):
    raise AssertionError(
        f"`git {' '.join(args)}` ran for real: a test must stub merge.git. On GitHub the "
        "checkout belongs to another user, so git refuses it (\"dubious ownership\")."
    )


def setUpModule():
    """Every test runs against a merge.git that fails loudly, so one that
    forgets to stub git fails on any machine, not only where the checkout's
    owner differs from the user (#196). A test's own patch replaces it."""
    patcher = mock.patch.object(merge, "git", no_real_git)
    patcher.start()
    unittest.addModuleCleanup(patcher.stop)


def git_stub(listing: str):
    def git(*args, **_):
        return listing if args[0] == "worktree" else HERE + "\n"

    return git


class Next(unittest.TestCase):
    def next_step(self, pr: dict, *checkouts: tuple[str, str]) -> str:
        with mock.patch.object(merge, "git", git_stub(worktrees(*checkouts))):
            return merge.bring_up_to_date(pr)

    def test_a_branch_checked_out_nowhere_is_switched_to_and_landed(self):
        self.assertIn(
            "Next: git fetch origin && git switch fix-the-thing && just land 42\n",
            self.next_step(PR, (HERE, "other")),
        )

    def test_a_branch_checked_out_here_is_landed_from_here(self):
        message = self.next_step(PR, (HERE, "fix-the-thing"))
        self.assertIn("Next: just land 42\n", message)
        self.assertNotIn("git switch", message)

    def test_a_branch_checked_out_in_another_worktree_is_landed_there(self):
        message = self.next_step(PR, (HERE, "other"), ("/work/agent", "fix-the-thing"))
        self.assertIn("Next: cd /work/agent && just land 42\n", message)
        self.assertNotIn("git switch", message)

    def test_it_names_the_base_the_pr_targets(self):
        pr = {**PR, "baseRefName": "release/1.0"}
        self.assertIn("merges origin/release/1.0 into the branch", self.next_step(pr))

    def test_it_says_what_land_does_and_that_it_stops_on_a_conflict(self):
        message = self.next_step(PR)
        self.assertIn("waits for GitHub's \"CI passed\"", message)
        self.assertIn("On a conflict it stops and says what to run.", message)


class BehindMain(unittest.TestCase):
    """Every refusal for a head behind the base says the same Next step."""

    def refusal(self, behind: int, files: list[str], named: dict[str, str] | None = None) -> str:
        with (
            mock.patch.object(merge, "gh", return_value=compare(behind, files)),
            mock.patch.object(merge, "docs_named_by_code", return_value=named or {}),
            mock.patch.object(merge, "bring_up_to_date", return_value="Next: the one command"),
            self.assertRaises(merge.Refused) as raised,
        ):
            merge.require_contains_base(PR)
        return str(raised.exception)

    def test_code_changed_on_main(self):
        self.assertIn("Next: the one command", self.refusal(1, ["crates/core/src/lib.rs"]))

    def test_too_many_commits_to_list(self):
        self.assertIn("Next: the one command", self.refusal(400, ["fixture-notes.md"]))

    def test_docs_that_code_names(self):
        message = self.refusal(1, ["docs/fixture/page.md"], {"docs/fixture/page.md": "a.rs"})
        self.assertIn("which a.rs names", message)
        self.assertIn("Next: the one command", message)

    def test_none_of_them_sends_the_user_to_update_branch_a_force_push_or_a_rebase(self):
        with mock.patch.object(merge, "git", git_stub(worktrees())):
            text = merge.bring_up_to_date(PR)
        self.assertNotIn("Next: gh pr update-branch", text)
        self.assertNotIn("--force", text)
        self.assertNotIn("git rebase", text)
        self.assertNotIn("git merge --no-edit", text)

    def test_a_docs_only_gap_is_still_let_through(self):
        with mock.patch.object(merge, "gh", return_value=compare(1, ["docs/fixture/index.md"])):
            with mock.patch.object(merge, "docs_named_by_code", return_value={}):
                with mock.patch.object(merge, "git", git_stub(worktrees())):
                    allowed = merge.require_contains_base(PR)
        self.assertIn("Letting through 1 docs-only commit", allowed)


class TheLane(unittest.TestCase):
    """A merge to main takes the landing lock for its final recheck and the
    squash, so it can't fall between `just land`'s merge of main and its own
    squash (ticket #247). `just land` holds the lock already."""

    def merge(self, lane_held: bool) -> list[str]:
        """The order of the steps of one merge."""
        steps: list[str] = []
        pr = {**PR, "mergeable": "MERGEABLE", "isDraft": False, "state": "OPEN"}

        def gh(*args):
            if args[:2] == ("pr", "merge"):
                steps.append("squash")
            return json.dumps({"mergeCommit": {"oid": "c" * 40}})

        def step(name):
            return lambda *_, **__: steps.append(name)

        with (
            # The second look is the fresh one: same head, marked to tell it apart.
            mock.patch.object(merge, "pull_request", side_effect=[pr, {**pr, "fresh": True}]),
            mock.patch.object(merge, "require_contains_base", return_value=None),
            mock.patch.object(merge, "wait_for_checks", step("wait")),
            mock.patch.object(merge, "require_checks_passed", step("checked")),
            mock.patch.object(merge, "require_no_unrecorded_rerun", step("reruns")),
            mock.patch.object(merge, "require_mergeable", side_effect=lambda pr: pr),
            mock.patch.object(merge, "require_no_step_waits_on_a_closed_ticket", step("ticket")),
            mock.patch.object(merge, "require_reviewed",
                              lambda pr: steps.append("reviews fresh" if pr.get("fresh") else "reviews")),
            mock.patch.object(merge, "take_landing_turn", step("take the lock")),
            mock.patch.object(merge, "gh", gh),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            merge.merge(42, 0, lane_held=lane_held)
        return steps

    def test_a_bare_merge_waits_for_checks_without_the_lock_then_holds_it_to_the_squash(self):
        self.assertEqual(
            self.merge(lane_held=False),
            ["reviews", "wait", "checked", "reruns", "take the lock", "ticket", "reviews fresh", "reruns", "squash"],
        )

    def test_land_holds_the_lock_itself_so_merge_does_not_take_it_again(self):
        self.assertEqual(self.merge(lane_held=True), ["reviews", "wait", "checked", "reruns", "ticket", "reviews fresh", "reruns", "squash"])

    def test_the_lock_keeps_a_second_landing_out_until_the_first_lets_go(self):
        with tempfile.TemporaryDirectory() as directory:
            lock = Path(directory) / "landing-lock"
            with mock.patch.object(merge, "LANDING_LOCK", lock):
                first = merge.take_landing_turn()
                second = open(lock, "w")
                self.addCleanup(second.close)
                with self.assertRaises(BlockingIOError):
                    fcntl.flock(second, fcntl.LOCK_EX | fcntl.LOCK_NB)
                first.close()
                fcntl.flock(second, fcntl.LOCK_EX | fcntl.LOCK_NB)  # raises if still held


class WaitingForChecks(unittest.TestCase):
    """GitHub starts CI on a push after a few seconds: a head with no check
    yet is waited for, for a while (#286)."""

    def wait(self, seen: list[list[dict]], clock: list[float]) -> int:
        looks = iter(seen)
        asked = []

        def checks(pr):
            asked.append(1)
            return next(looks)

        with (
            mock.patch.object(merge, "checks", checks),
            mock.patch.object(merge.time, "sleep"),
            mock.patch.object(merge.time, "monotonic", side_effect=clock),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            merge.wait_for_checks(PR, 30 * 60)
        return len(asked)

    def test_checks_that_appear_after_a_moment_are_waited_for(self):
        passed = [{"name": "CI passed", "bucket": "pass"}]
        self.assertEqual(self.wait([[], [], passed], [0, 0, 1, 1, 2, 2, 3, 3]), 3)

    def test_a_head_github_starts_nothing_on_is_given_up_on(self):
        late = merge.CHECKS_APPEAR_SECONDS + 1
        self.assertEqual(self.wait([[], []], [0, 0, 1, 1, late, late]), 2)


class RequiringCIPassed(unittest.TestCase):
    """Only GitHub's "CI passed" on the head judges every job, so a head
    without it passed is refused however green its other checks are (#290)."""

    def require(self, found: list[dict]) -> None:
        with mock.patch.object(merge, "checks", lambda pr: found):
            merge.require_checks_passed(PR, 0)

    def test_a_dispatched_runs_summary_alone_is_refused(self):
        found = [
            {"name": "just lint", "event": "workflow_dispatch", "bucket": "pass", "state": "SUCCESS", "link": ""},
            {"name": "CI passed (dispatched run)", "event": "workflow_dispatch", "bucket": "pass", "state": "SUCCESS", "link": ""},
        ]
        with self.assertRaises(merge.Refused) as refused:
            self.require(found)
        self.assertIn('No passed "CI passed"', str(refused.exception))
        self.assertIn("Next:", str(refused.exception))

    def test_a_passed_ci_passed_is_let_through(self):
        self.require([
            {"name": "just lint", "event": "pull_request", "workflow": "CI", "bucket": "pass", "state": "SUCCESS", "link": ""},
            {"name": "CI passed", "event": "pull_request", "workflow": "CI", "bucket": "pass", "state": "SUCCESS", "link": ""},
        ])

    def test_a_status_or_another_workflows_job_named_ci_passed_is_refused(self):
        for impostor in [
            # A commit status, as `just ci-local` posted before #286: no workflow, no event.
            {"name": "CI passed", "event": "", "workflow": "", "bucket": "pass", "state": "SUCCESS", "link": ""},
            {"name": "CI passed", "event": "pull_request", "workflow": "Other", "bucket": "pass", "state": "SUCCESS", "link": ""},
        ]:
            with self.subTest(impostor), self.assertRaises(merge.Refused) as refused:
                self.require([impostor])
            self.assertIn('No passed "CI passed" check from CI\'s pull_request run', str(refused.exception))


if __name__ == "__main__":
    unittest.main()
