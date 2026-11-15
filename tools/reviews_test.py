#!/usr/bin/env python3
"""Tests of the review gate (tools/reviews.py, tools/review_record.py and
their use in `just merge`, ticket #327). GitHub is never asked. The git
steps run against a throwaway repository. `just lint` runs them."""

from __future__ import annotations

import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import changes  # noqa: E402
import merge  # noqa: E402
import review_record  # noqa: E402
import reviews  # noqa: E402

ENV = {
    "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.com",
    "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.com",
}
SHA = "a" * 40


def line(kind: str, sha: str, verdict: str = "PASS", model: str = "sonnet") -> reviews.Record:
    parsed = reviews.parse(reviews.first_line(kind, model, sha, verdict))
    assert parsed is not None
    return parsed


class Parsing(unittest.TestCase):
    def test_a_record_is_read_from_the_first_line_and_the_findings_below_are_ignored(self):
        self.assertEqual(
            reviews.parse(f"Review: spec by opus on {SHA}: FAIL\n\nFindings: one blocker."),
            reviews.Record("spec", "opus", SHA, "FAIL"),
        )

    def test_a_comment_that_is_not_a_record_records_nothing(self):
        for body in [
            "",
            "LGTM",
            f"Looks fine.\nReview: spec by opus on {SHA}: PASS",  # not the first line
            f"Review: spec by opus on {SHA[:7]}: PASS",  # a short sha
            f"Review: style by opus on {SHA}: PASS",  # no such kind
            f"Review: spec by fable on {SHA}: PASS",  # never Fable
            f"Review: spec by opus on {SHA}: PASSED",
        ]:
            self.assertIsNone(reviews.parse(body), body)


class WhichKinds(unittest.TestCase):
    def test_product_or_harness_code_needs_standards_spec_and_spec(self):
        for path in ["crates/core/src/lib.rs", "crates/test-harness/tests/handoff.rs", "data/systemd/x.service",
                     "Cargo.lock", "Cargo.toml", "rust-toolchain.toml"]:
            self.assertEqual(reviews.kinds_needed(["fixture.md", path]), ("standards-spec", "spec"), path)

    def test_docs_and_tooling_need_one_tooling_review(self):
        paths = ["docs/fixture/page.md", "tools/land.py", "justfile", ".github/workflows/ci.yml",
                 "deny.toml", "crates.md"]
        self.assertEqual(reviews.kinds_needed(paths), ("tooling",))


class InARepository(unittest.TestCase):
    """main, and a PR branch `work` off it, with origin/main set by hand."""

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        previous = os.getcwd()
        os.chdir(self.root)
        self.addCleanup(os.chdir, previous)
        patcher = mock.patch.dict(os.environ, ENV)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.git("init", "--quiet", "--initial-branch=main")
        self.commit("shared.txt", "one\ntwo\nthree\nfour\nfive\nsix\nseven\n", "Start")
        self.git("update-ref", "refs/remotes/origin/main", "HEAD")
        self.git("switch", "--quiet", "-c", "work")

    def git(self, *args: str) -> str:
        return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout.strip()

    def commit(self, path: str, text: str, message: str) -> str:
        (self.root / path).parent.mkdir(parents=True, exist_ok=True)
        (self.root / path).write_text(text)
        self.git("add", "--all")
        self.git("commit", "--quiet", "-m", message)
        return self.git("rev-parse", "HEAD")

    def main_moves(self, path: str, text: str) -> None:
        self.git("switch", "--quiet", "main")
        self.git("update-ref", "refs/remotes/origin/main", self.commit(path, text, f"Main changes {path}"))
        self.git("switch", "--quiet", "work")

    def merge_main(self) -> str:
        self.git("merge", "--quiet", "--no-edit", "origin/main")
        return self.git("rev-parse", "HEAD")

    def problems(self, records: list[reviews.Record], head: str | None = None) -> list[str]:
        head = head or self.git("rev-parse", "HEAD")
        return reviews.problems(changes.run_git, records, head, "origin/main")[1]

    # -- what each PR needs

    def test_a_tooling_pr_with_no_record_is_refused(self):
        self.commit("tools/x.py", "x = 1\n", "Tool")
        self.assertEqual(self.problems([]), ["tooling: none recorded."])

    def test_a_tooling_pr_with_a_passed_tooling_review_of_its_head_is_reviewed(self):
        head = self.commit("tools/x.py", "x = 1\n", "Tool")
        self.assertEqual(self.problems([line("tooling", head)]), [])

    def test_a_product_pr_needs_both_reviews(self):
        head = self.commit("crates/core/src/lib.rs", "fn f() {}\n", "Code")
        self.assertEqual(self.problems([line("standards-spec", head)]), ["spec: none recorded."])
        self.assertEqual(self.problems([line("tooling", head)]),
                         ["standards-spec: none recorded.", "spec: none recorded."])
        self.assertEqual(self.problems([line("standards-spec", head), line("spec", head, model="opus")]), [])

    # -- verdicts

    def test_a_fail_after_a_pass_on_the_same_head_refuses_and_a_pass_after_it_lets_through(self):
        head = self.commit("tools/x.py", "x = 1\n", "Tool")
        failed = [line("tooling", head), line("tooling", head, "FAIL")]
        self.assertEqual(self.problems(failed), [f"tooling: the latest review of this head is FAIL by sonnet on {head[:7]}."])
        self.assertEqual(self.problems([*failed, line("tooling", head)]), [])

    # -- a head that moved since the review

    def test_a_new_commit_since_the_review_needs_a_new_review(self):
        reviewed = self.commit("tools/x.py", "x = 1\n", "Tool")
        new = self.commit("tools/x.py", "x = 2\n", "Fix")
        (problem,) = self.problems([line("tooling", reviewed)])
        self.assertIn(f"commits {new[:7]} changed the PR's diff since {reviewed[:7]}", problem)

    def test_a_clean_merge_of_main_keeps_the_review(self):
        reviewed = self.commit("shared.txt", "one\nTWO\nthree\nfour\nfive\nsix\nseven\n", "Work")
        self.main_moves("shared.txt", "one\ntwo\nthree\nfour\nfive\nsix\nSEVEN\n")
        self.main_moves("tools/other.py", "y = 1\n")
        self.merge_main()
        self.assertEqual(self.problems([line("tooling", reviewed)]), [])

    def test_a_merge_of_main_that_resolved_a_conflict_needs_a_new_review(self):
        reviewed = self.commit("shared.txt", "one\nTWO\nthree\nfour\nfive\nsix\nseven\n", "Work")
        self.main_moves("shared.txt", "one\n2\nthree\nfour\nfive\nsix\nseven\n")
        subprocess.run(["git", "merge", "--quiet", "--no-edit", "origin/main"], capture_output=True)
        (self.root / "shared.txt").write_text("one\nTWO and 2\nthree\nfour\nfive\nsix\nseven\n")
        self.git("commit", "--quiet", "--all", "--no-edit")
        (problem,) = self.problems([line("tooling", reviewed)])
        self.assertIn("a merge that resolved a conflict or changed more", problem)

    def test_a_merge_of_main_that_slips_in_another_change_needs_a_new_review(self):
        reviewed = self.commit("tools/x.py", "x = 1\n", "Tool")
        self.main_moves("tools/other.py", "y = 1\n")
        self.git("merge", "--quiet", "--no-commit", "origin/main")
        (self.root / "tools" / "x.py").write_text("x = 'not reviewed'\n")
        self.git("commit", "--quiet", "--all", "-m", "Merge remote-tracking branch 'origin/main' into work")
        (problem,) = self.problems([line("tooling", reviewed)])
        self.assertIn("changed the PR's diff", problem)

    def test_a_review_of_a_commit_rebased_away_does_not_count(self):
        reviewed = self.commit("tools/x.py", "x = 1\n", "Tool")
        self.git("reset", "--quiet", "--hard", "HEAD~1")
        self.commit("tools/x.py", "x = 1\n", "Tool, again")
        (problem,) = self.problems([line("tooling", reviewed)])
        self.assertIn(f"{reviewed[:7]} is not in the history of", problem)

    def test_a_review_of_a_commit_this_clone_lacks_does_not_count(self):
        self.commit("tools/x.py", "x = 1\n", "Tool")
        (problem,) = self.problems([line("tooling", "f" * 40)])
        self.assertIn("is not in this clone", problem)


PR = {"number": 42, "headRefOid": SHA, "headRefName": "work", "baseRefName": "main", "state": "OPEN"}


class Refusing(unittest.TestCase):
    def test_the_refusal_names_what_is_missing_and_the_command_to_record_a_review(self):
        with (
            mock.patch.object(merge, "review_status", return_value=(reviews.PRODUCT_KINDS, ["spec: none recorded."])),
            self.assertRaises(merge.Refused) as refused,
        ):
            merge.require_reviewed(PR)
        message = str(refused.exception)
        self.assertIn("It touches product or harness code, so it needs standards-spec and spec", message)
        self.assertIn("  spec: none recorded.", message)
        self.assertIn("Next: have aaaaaaa reviewed", message)
        self.assertIn("just review-record 42 <kind> <model> <PASS|FAIL> <findings file>", message)

    def test_it_reads_the_records_from_the_prs_comments(self):
        comments = {"comments": [{"body": "LGTM"}, {"body": reviews.first_line("tooling", "sonnet", SHA, "PASS")}]}
        seen = []

        def problems(git, records, head, base):
            seen.append((records, head, base))
            return ("tooling",), []

        with (
            mock.patch.object(merge, "git", lambda *a, **k: ""),
            mock.patch.object(merge, "gh", lambda *a: json.dumps(comments)),
            mock.patch.object(reviews, "problems", problems),
        ):
            merge.require_reviewed(PR, head="b" * 40)
        self.assertEqual(seen, [([reviews.Record("tooling", "sonnet", SHA, "PASS")], "b" * 40, "origin/main")])


class Recording(unittest.TestCase):
    def record(self, *arguments, local: str | None = SHA, findings: str | None = None) -> tuple[list, str]:
        posted = []

        def gh(*args):
            posted.append(args)
            return ""

        out = io.StringIO()
        with (
            mock.patch.object(merge, "pull_request", return_value=PR),
            mock.patch.object(merge, "gh", gh),
            mock.patch.object(review_record, "local_head_of", return_value=local),
            mock.patch.object(merge, "review_status", return_value=(("tooling",), [])),
            contextlib.redirect_stdout(out),
        ):
            review_record.record(42, *arguments, findings)
        return posted, out.getvalue()

    def test_it_posts_the_record_for_the_head_github_has_with_the_findings_below(self):
        with tempfile.NamedTemporaryFile("w", suffix=".md") as file:
            file.write("1. Nit: a name. Fixed in abc1234.\n")
            file.flush()
            posted, out = self.record("tooling", "Sonnet", "PASS", findings=file.name)
        self.assertEqual(posted, [("pr", "comment", "42", "--body",
                                   f"Review: tooling by sonnet on {SHA}: PASS\n\n1. Nit: a name. Fixed in abc1234.\n")])
        self.assertIn("Next: just land 42", out)

    def test_without_findings_it_says_none_were_recorded(self):
        posted, _ = self.record("tooling", "sonnet", "FAIL")
        self.assertTrue(posted[0][-1].endswith(f"FAIL\n\n{review_record.NO_FINDINGS}\n"))

    def test_a_local_head_github_lacks_is_refused(self):
        with self.assertRaises(merge.Refused) as refused:
            self.record("tooling", "sonnet", "PASS", local="b" * 40)
        self.assertIn("Next: git push", str(refused.exception))

    def test_bad_arguments_are_refused_before_github_is_asked(self):
        for arguments, said in [
            (("style", "sonnet", "PASS"), "Unknown kind"),
            (("spec", "fable", "PASS"), "Fable never reviews"),
            (("spec", "gpt", "PASS"), "Unknown model"),
            (("spec", "opus", "pass"), "Unknown verdict"),
        ]:
            with self.subTest(arguments), self.assertRaises(merge.Refused) as refused:
                with mock.patch.object(merge, "pull_request", side_effect=AssertionError("asked GitHub")):
                    review_record.record(42, *arguments, None)
            self.assertIn(said, str(refused.exception))


if __name__ == "__main__":
    unittest.main()
