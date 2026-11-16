#!/usr/bin/env python3
"""Tests of the bound-change check (tools/bound_changes.py, ticket #398),
on diffs written out here, including #379's own. `just lint` runs them."""

from __future__ import annotations

import contextlib
import io
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import bound_changes  # noqa: E402

PATH = "crates/test-harness/tests/dialer_surface.rs"


def diff(removed: list[str], added: list[str], start: int = 10) -> str:
    """A one-hunk `git diff -U0` of PATH."""
    lines = [f"diff --git a/{PATH} b/{PATH}", f"--- a/{PATH}", f"+++ b/{PATH}",
             f"@@ -{start},{len(removed)} +{start},{len(added)} @@"]
    return "\n".join(lines + [f"-{line}" for line in removed] + [f"+{line}" for line in added]) + "\n"


def problems(removed: list[str], added: list[str], above: list[str] = ()) -> list[str]:
    """Problems in that diff, with `above` the new file's lines before the hunk at line 10."""
    text = "\n".join([*["// filler"] * (9 - len(above)), *above, *added])
    return bound_changes.problems(diff(removed, added), read=lambda _: text)


# PR #379's change (00f3411), cut to the lines the check reads.
PR_379 = [
    "const CI_COLD_KEYSTROKE_DEADLINE: Duration = Duration::from_millis(100);",
    "fn cold_keystroke_deadline() -> Duration {",
    '    if std::env::var_os("CI").is_some() && std::env::var_os(CI_LIKE).is_none() {',
    "        CI_COLD_KEYSTROKE_DEADLINE",
]


class Flags(unittest.TestCase):
    def test_pr_379s_ci_only_bound_is_flagged_twice(self):
        found = problems([], PR_379)
        self.assertEqual(found, [
            f"  {PATH}:10: adds the CI bound CI_COLD_KEYSTROKE_DEADLINE",
            f"  {PATH}:12: chooses a bound on whether this is CI",
        ])

    def test_a_raised_duration_is_flagged(self):
        self.assertEqual(
            problems(["    wait_until(&app, Duration::from_millis(300));"],
                     ["    wait_until(&app, Duration::from_secs(1));"]),
            [f"  {PATH}:10: raises a Duration from 300 ms to 1000 ms"],
        )

    def test_a_raised_constant_is_flagged(self):
        self.assertEqual(
            problems(["const DEADLINE: Duration = Duration::from_millis(50);"],
                     ["const DEADLINE: Duration = Duration::from_millis(80);"]),
            [f"  {PATH}:10: raises a Duration from 50 ms to 80 ms"],
        )

    def test_raises_written_other_ways_are_flagged(self):
        for removed, added in [
            (["const D: Duration = Duration::from_millis(100);"], ["const D: Duration = Duration::from_millis(100 * 2);"]),
            (["const D: Duration = Duration::from_millis(", "    200);"],
             ["const D: Duration = Duration::from_millis(", "    400);"]),
            (["const D: Duration = Duration::from_millis(50);"],
             ["/// Slower now.", "const SLOW_D: Duration = Duration::from_millis(500);"]),
            (["const D: Duration = Duration::new(1, 0);"], ["const D: Duration = Duration::new(5, 0);"]),
            (["const D: Duration = Duration::from_secs(1);"], ["const D: Duration = Duration::from_secs_f64(1.5);"]),
        ]:
            with self.subTest(added=added):
                self.assertEqual(len(problems(removed, added)), 1, problems(removed, added))

    def test_a_new_ci_constant_of_any_type_is_flagged(self):
        self.assertEqual(problems([], ["const CI_SLACK_MS: u64 = 300;"]),
                         [f"  {PATH}:10: adds the CI bound CI_SLACK_MS"])

    def test_a_ci_test_in_a_comment_is_not_flagged(self):
        self.assertEqual(problems([], ['// unlike var_os("CI"), this runs everywhere']), [])

    def test_each_hunk_is_judged_alone(self):
        text = diff(["const A: Duration = Duration::from_millis(80);"],
                    ["const A: Duration = Duration::from_millis(50);"]) + \
            f"@@ -40,0 +40,1 @@\n+const B: Duration = Duration::from_secs(9);\n"
        self.assertEqual(bound_changes.problems(text, read=lambda _: ""), [])

    def test_a_line_that_looks_like_a_header_inside_a_hunk_is_read_as_a_line(self):
        text = diff(["--- a/old", "    let a = Duration::from_millis(1);"],
                    ["+++ b/new", "    let a = Duration::from_millis(2);"])
        self.assertEqual(bound_changes.problems(text, read=lambda _: ""), [f"  {PATH}:11: raises a Duration from 1 ms to 2 ms"])


class LetsThrough(unittest.TestCase):
    def test_a_record_on_the_line_or_just_above_lets_each_through(self):
        record = "// bound-change: first keystroke 31 ms cold on CI, measured in run 1 (#378)"
        self.assertEqual(problems([], [record, *PR_379]), [])
        self.assertEqual(problems([], PR_379[:1], above=[record]), [])
        self.assertNotEqual(problems([], PR_379[:1], above=[record, "", "", ""]), [])

    def test_a_lowered_or_unchanged_bound_needs_no_record(self):
        self.assertEqual(problems(["const DEADLINE: Duration = Duration::from_millis(80);"],
                                  ["const DEADLINE: Duration = Duration::from_millis(50);"]), [])
        self.assertEqual(problems(["    let a = Duration::from_millis(80); // old"],
                                  ["    let a = Duration::from_millis(80); // new"]), [])

    def test_a_new_scenarios_own_deadline_needs_no_record(self):
        self.assertEqual(problems([], ["const DEADLINE: Duration = Duration::from_secs(5);"]), [])

    def test_a_deleted_file_is_not_read(self):
        text = f"diff --git a/{PATH} b/{PATH}\n--- a/{PATH}\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-const CI_X: u8 = 1;\n"
        self.assertEqual(bound_changes.problems(text, read=lambda _: self.fail("read a deleted file")), [])


class AgainstGit(unittest.TestCase):
    def test_it_reads_a_real_diff_and_new_files_whatever_the_users_git_prefixes(self):
        with tempfile.TemporaryDirectory() as repo:
            git = lambda *a: subprocess.run(["git", "-C", repo, "-c", "user.name=t", "-c", "user.email=t@t", *a],
                                            check=True, capture_output=True)
            git("init", "-q")
            git("config", "diff.mnemonicPrefix", "true")
            path = os.path.join(repo, PATH)
            os.makedirs(os.path.dirname(path))
            open(path, "w").write("fn a() {}\n")
            git("add", "-A")
            git("commit", "-qm", "base")
            open(path, "a").write(PR_379[0] + "\n")
            fresh = os.path.join(repo, "crates/automation/src/new.rs")
            os.makedirs(os.path.dirname(fresh))
            open(fresh, "w").write("const CI_GRACE_MS: u64 = 300;\n")  # untracked
            here = os.getcwd()
            os.chdir(repo)
            try:
                with mock.patch.dict(os.environ, {"BOUND_CHANGES_BASE": "HEAD"}), \
                        contextlib.redirect_stderr(io.StringIO()) as err:
                    self.assertEqual(bound_changes.main(), 1)
            finally:
                os.chdir(here)
        self.assertIn("CI_COLD_KEYSTROKE_DEADLINE", err.getvalue())
        self.assertIn("crates/automation/src/new.rs:1: adds the CI bound CI_GRACE_MS", err.getvalue())


class Values(unittest.TestCase):
    def test_durations_are_read_in_nanoseconds(self):
        self.assertEqual(bound_changes.nanos("Duration::from_millis(1_500) Duration::from_secs_f64(0.5)"),
                         [1_500_000_000, 500_000_000])


if __name__ == "__main__":
    unittest.main()
