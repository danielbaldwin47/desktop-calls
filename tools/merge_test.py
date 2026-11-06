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


def compare(behind: int, files: list[str]) -> str:
    commits = [{"sha": f"{n:040x}", "subject": f"Commit {n}"} for n in range(behind)]
    return json.dumps({"behind": behind, "commits": commits, "files": files})


class BehindMain(unittest.TestCase):
    def refusal(self, behind: int, files: list[str]) -> str:
        with mock.patch.object(merge, "gh", return_value=compare(behind, files)):
            with self.assertRaises(merge.Refused) as raised:
                merge.require_contains_base(PR)
        return str(raised.exception)

    def test_it_prints_the_merge_and_the_one_ci_local_run(self):
        message = self.refusal(1, ["crates/core/src/lib.rs"])
        self.assertIn(
            "Next: git fetch origin && git switch fix-the-thing && git merge origin/main "
            "&& just ci-local   (then `just merge 42`)",
            message,
        )

    def test_it_never_sends_the_user_to_update_branch_or_a_force_push(self):
        message = self.refusal(2, ["crates/core/src/lib.rs"])
        self.assertNotIn("Next: gh pr update-branch", message)
        self.assertNotIn("--force", message)
        self.assertNotIn("git rebase", message)

    def test_every_refusal_for_a_branch_behind_main_says_the_same_next_step(self):
        expected = merge.bring_up_to_date(PR).splitlines()[0]
        for behind, files in [(1, ["crates/core/src/lib.rs"]), (400, ["README.md"])]:
            self.assertIn(expected, self.refusal(behind, files))

    def test_a_docs_only_gap_is_still_let_through(self):
        with mock.patch.object(merge, "gh", return_value=compare(1, ["docs/wiki/index.md"])):
            with mock.patch.object(merge, "docs_named_by_code", return_value={}):
                allowed = merge.require_contains_base(PR)
        self.assertIn("Letting through 1 docs-only commit", allowed)


if __name__ == "__main__":
    unittest.main()
