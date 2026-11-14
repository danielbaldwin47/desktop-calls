#!/usr/bin/env python3
"""Tests of how `just merge` says to bring a PR up to date (tools/merge.py,
tickets #181 and #247), and of the lock its final step takes. GitHub is never
asked. `just lint` runs them."""

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
        self.assertIn("runs CI on the result", message)
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
    """A merge to main takes the CI lane's lock for its final recheck and the
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
            mock.patch.object(merge, "pull_request", return_value=pr),
            mock.patch.object(merge, "require_contains_base", return_value=None),
            mock.patch.object(merge, "wait_for_checks", step("wait")),
            mock.patch.object(merge, "require_checks_passed", step("checked")),
            mock.patch.object(merge, "require_mergeable", side_effect=lambda pr: pr),
            mock.patch.object(merge, "require_no_step_waits_on_a_closed_ticket", step("ticket")),
            mock.patch.object(merge.ci_local, "take_turn", step("take the lock")),
            mock.patch.object(merge, "gh", gh),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            merge.merge(42, 0, lane_held=lane_held)
        return steps

    def test_a_bare_merge_waits_for_checks_without_the_lock_then_holds_it_to_the_squash(self):
        self.assertEqual(
            self.merge(lane_held=False),
            ["wait", "checked", "take the lock", "ticket", "squash"],
        )

    def test_land_holds_the_lock_itself_so_merge_does_not_take_it_again(self):
        self.assertEqual(self.merge(lane_held=True), ["wait", "checked", "ticket", "squash"])


if __name__ == "__main__":
    unittest.main()
