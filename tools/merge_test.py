#!/usr/bin/env python3
"""Tests of how `just merge` says to bring a PR up to date (tools/merge.py,
ticket #181). GitHub is never asked. `just lint` runs them."""

from __future__ import annotations

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


def git_stub(listing: str):
    def git(*args, **_):
        return listing if args[0] == "worktree" else HERE + "\n"

    return git


class Next(unittest.TestCase):
    def next_step(self, pr: dict, *checkouts: tuple[str, str]) -> str:
        with mock.patch.object(merge, "git", git_stub(worktrees(*checkouts))):
            return merge.bring_up_to_date(pr)

    def test_a_branch_checked_out_nowhere_is_switched_to_and_brought_level_with_github(self):
        self.assertIn(
            "Next: git fetch origin && git switch fix-the-thing "
            "&& git merge --ff-only origin/fix-the-thing "
            "&& git merge --no-edit origin/main && just ci-local   (then `just merge 42`)",
            self.next_step(PR, (HERE, "other")),
        )

    def test_a_branch_checked_out_here_is_not_switched_to(self):
        message = self.next_step(PR, (HERE, "fix-the-thing"))
        self.assertIn("Next: git fetch origin && git merge --ff-only origin/fix-the-thing", message)
        self.assertNotIn("git switch", message)

    def test_a_branch_checked_out_in_another_worktree_is_updated_there(self):
        message = self.next_step(PR, (HERE, "other"), ("/work/agent", "fix-the-thing"))
        self.assertIn("Next: cd /work/agent && git fetch origin && git merge --ff-only", message)
        self.assertNotIn("git switch", message)

    def test_it_merges_the_base_the_pr_targets_into_the_branch_it_names(self):
        pr = {**PR, "baseRefName": "release/1.0", "headRefName": "agent/fix"}
        message = self.next_step(pr)
        self.assertIn("git merge --ff-only origin/agent/fix", message)
        self.assertIn("git merge --no-edit origin/release/1.0", message)

    def test_it_says_what_to_do_on_a_conflict(self):
        self.assertIn("On a conflict: resolve it, commit, then `just ci-local`.", self.next_step(PR))


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
        self.assertIn("Next: the one command", self.refusal(400, ["README.md"]))

    def test_docs_that_code_names(self):
        message = self.refusal(1, ["docs/wiki/polish-scorecard.md"], {"docs/wiki/polish-scorecard.md": "a.rs"})
        self.assertIn("which a.rs names", message)
        self.assertIn("Next: the one command", message)

    def test_none_of_them_sends_the_user_to_update_branch_or_a_force_push(self):
        with mock.patch.object(merge, "git", git_stub(worktrees())):
            text = merge.bring_up_to_date(PR)
        self.assertNotIn("Next: gh pr update-branch", text)
        self.assertNotIn("--force", text)
        self.assertNotIn("git rebase", text)

    def test_a_docs_only_gap_is_still_let_through(self):
        with mock.patch.object(merge, "gh", return_value=compare(1, ["docs/wiki/index.md"])):
            with mock.patch.object(merge, "docs_named_by_code", return_value={}):
                allowed = merge.require_contains_base(PR)
        self.assertIn("Letting through 1 docs-only commit", allowed)


if __name__ == "__main__":
    unittest.main()
