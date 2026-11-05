#!/usr/bin/env python3
"""Tests of ci_local's rules, and that it can't drift from CI. `just lint` runs them."""

from __future__ import annotations

import os
import pathlib
import re
import tempfile
import time
import unittest
from unittest import mock

import ci_local

REPOSITORY = pathlib.Path(__file__).resolve().parent.parent
WORKFLOW = (REPOSITORY / ".github/workflows/ci.yml").read_text()
BUILD_ROOTFS = (REPOSITORY / "tools/local-ci/build-rootfs.sh").read_text()
JUSTFILE = (REPOSITORY / "justfile").read_text()


def packages(text: str) -> set[str]:
    return set(text.split())


class SameAsCI(unittest.TestCase):
    def test_full_ci_runs_only_on_pushes_to_main(self):
        trigger = WORKFLOW.split("\njobs:")[0]
        self.assertNotRegex(trigger, r"(?m)^  pull_request:")
        self.assertRegex(trigger, r"(?m)^  push:\n    branches: \[main\]")

    def test_every_job_is_a_command_ci_runs(self):
        # CI runs each `just` recipe a local job runs, so a recipe can't change
        # for one and not the other. ci.yml passes ci-scenarios its shard.
        commands = {
            recipe.strip()
            for command in ci_local.JOBS.values()
            for recipe in command.split("&&")
        }
        self.assertEqual(
            commands,
            {
                "just lint",
                "just test --exclude test-harness",
                "just scorecard",
                "just ci-scenarios",
                "just ci-first-frame-timings",
            },
        )
        for command in commands:
            self.assertIn(f"run: {command}", WORKFLOW.replace("run: >-\n          ", "run: "))
        for recipe in ("ci-scenarios", "ci-first-frame-timings"):
            self.assertRegex(JUSTFILE, rf"(?m)^{recipe}\b.*:$")

    def test_the_image_has_the_packages_ci_installs(self):
        # The workflow's one package list, anchored as `&install` and used by
        # every test job.
        installed = re.search(r"&install\n\s+name: [^\n]*\n\s+run: >-\n((?:\s+[^\n]+\n)+?)\n", WORKFLOW)
        self.assertIsNotNone(installed)
        ci = packages(installed.group(1).replace("pacman -Syu --noconfirm --needed", ""))
        local = re.search(r"pacman -Syu --noconfirm --needed \\\n((?:    [^\n]+\\?\n)+)", BUILD_ROOTFS)
        self.assertIsNotNone(local)
        self.assertEqual(packages(local.group(1).replace("\\", "")), ci)


class Folders(unittest.TestCase):
    def test_clears_only_old_test_folders_of_this_user(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = pathlib.Path(directory)
            now = time.time()
            old = parent / "desktop-calls-test-old"
            fresh = parent / "desktop-calls-test-fresh"
            other = parent / "something-else"
            for folder in (old, fresh, other):
                (folder / "inside").mkdir(parents=True)
            two_hours_ago = now - 2 * 3600
            os.utime(old, (two_hours_ago, two_hours_ago))
            os.utime(other, (two_hours_ago, two_hours_ago))
            removed = ci_local.clear_stale_test_folders(now, parent)
            self.assertEqual(removed, [old])
            self.assertFalse(old.exists())
            self.assertTrue(fresh.exists())
            self.assertTrue(other.exists())

    def test_leaves_a_symlink_alone(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = pathlib.Path(directory)
            target = parent / "target"
            target.mkdir()
            link = parent / "desktop-calls-test-link"
            link.symlink_to(target)
            two_hours_ago = time.time() - 2 * 3600
            os.utime(link, (two_hours_ago, two_hours_ago), follow_symlinks=False)
            self.assertEqual(ci_local.clear_stale_test_folders(parent=parent), [])
            self.assertTrue(target.exists())


class Posting(unittest.TestCase):
    SHA = "a" * 40

    def log(self):
        log = mock.Mock()
        log.display_path.return_value = "~/.cache/desktop-calls-ci/logs/x.log"
        return log

    def test_the_description_names_the_log_without_a_home_directory(self):
        text = ci_local.status_description("success", 104.4, [], self.log())
        self.assertIn("~/.cache/desktop-calls-ci/logs/x.log", text)
        self.assertNotIn("/home/", text)
        self.assertLessEqual(len(text), 140)

    def test_a_failure_names_the_jobs(self):
        text = ci_local.status_description("failure", 60, ["lint", "test"], self.log())
        self.assertIn("lint, test", text)

    def test_nothing_is_posted_if_head_moved(self):
        with mock.patch.object(ci_local, "head_and_cleanliness", return_value=("b" * 40, "")), \
                mock.patch.object(ci_local, "post_status") as post:
            with self.assertRaises(ci_local.Refused):
                ci_local.settle(self.SHA, [], 100, self.log())
            post.assert_not_called()

    def test_nothing_is_posted_if_the_tree_became_dirty(self):
        with mock.patch.object(ci_local, "head_and_cleanliness", return_value=(self.SHA, " M x")), \
                mock.patch.object(ci_local, "post_status") as post:
            with self.assertRaises(ci_local.Refused):
                ci_local.settle(self.SHA, [], 100, self.log())
            post.assert_not_called()

    def test_green_posts_success_on_the_tested_commit(self):
        with mock.patch.object(ci_local, "head_and_cleanliness", return_value=(self.SHA, "")), \
                mock.patch.object(ci_local, "github_has", return_value=True), \
                mock.patch.object(ci_local, "post_status") as post:
            self.assertEqual(ci_local.settle(self.SHA, [], 100, self.log()), ci_local.EXIT_PASSED)
            self.assertEqual(post.call_args.args[:2], (self.SHA, "success"))

    def test_green_pushes_first_if_github_lacks_the_commit(self):
        order = []
        with mock.patch.object(ci_local, "head_and_cleanliness", return_value=(self.SHA, "")), \
                mock.patch.object(ci_local, "github_has", return_value=False), \
                mock.patch.object(ci_local, "push_branch", side_effect=lambda *a: order.append("push")), \
                mock.patch.object(ci_local, "post_status", side_effect=lambda *a: order.append("post")):
            ci_local.settle(self.SHA, [], 100, self.log())
        self.assertEqual(order, ["push", "post"])

    def test_red_posts_failure_and_never_pushes(self):
        with mock.patch.object(ci_local, "head_and_cleanliness", return_value=(self.SHA, "")), \
                mock.patch.object(ci_local, "github_has", return_value=True), \
                mock.patch.object(ci_local, "push_branch") as push, \
                mock.patch.object(ci_local, "post_status") as post:
            self.assertEqual(ci_local.settle(self.SHA, ["lint"], 100, self.log()), ci_local.EXIT_FAILED)
            self.assertEqual(post.call_args.args[:2], (self.SHA, "failure"))
            push.assert_not_called()

    def test_red_on_a_commit_github_lacks_posts_nothing(self):
        with mock.patch.object(ci_local, "head_and_cleanliness", return_value=(self.SHA, "")), \
                mock.patch.object(ci_local, "github_has", return_value=False), \
                mock.patch.object(ci_local, "push_branch") as push, \
                mock.patch.object(ci_local, "post_status") as post:
            self.assertEqual(ci_local.settle(self.SHA, ["test"], 100, self.log()), ci_local.EXIT_FAILED)
            post.assert_not_called()
            push.assert_not_called()


if __name__ == "__main__":
    unittest.main()
