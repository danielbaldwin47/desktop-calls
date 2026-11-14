#!/usr/bin/env python3
"""Tests of ci_local's rules, and that it can't drift from CI. `just lint` runs them."""

from __future__ import annotations

import contextlib
import io
import json
import os
import pathlib
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

import ci_local

REPOSITORY = pathlib.Path(__file__).resolve().parent.parent
WORKFLOW = (REPOSITORY / ".github/workflows/ci.yml").read_text()
BUILD_ROOTFS = (REPOSITORY / "tools/local-ci/build-rootfs.sh").read_text()
JUSTFILE = (REPOSITORY / "justfile").read_text()


REPO_EXPORT = "export GITHUB_REPOSITORY=o/r"


def packages(text: str) -> set[str]:
    return set(text.split())


class Repository(unittest.TestCase):
    """The run is told which repository it is, as GitHub Actions tells CI."""

    def test_either_form_of_a_github_url_gives_owner_and_name(self):
        for url in ("https://github.com/o/r.git", "git@github.com:o/r.git", "https://github.com/o/r"):
            self.assertEqual(ci_local.owner_and_name(url), "o/r", url)

    def test_another_host_is_refused_with_what_to_do(self):
        with self.assertRaises(ci_local.Refused) as raised:
            ci_local.owner_and_name("https://example.com/o/r.git")
        self.assertIn("Next:", str(raised.exception))

    def test_the_checkout_in_the_image_belongs_to_another_user_as_on_github(self):
        script = (REPOSITORY / "tools/local-ci/enter.sh").read_text()
        self.assertRegex(script, r'chown -h 1:1 \{\}')


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

    def test_every_just_command_in_a_ci_job_is_a_local_job(self):
        # The other direction: a `just` step added to the lint, check or
        # scenarios job of ci.yml must be added to JOBS, or it runs on GitHub
        # only, and "CI passed" can be posted without it.
        local = {
            recipe.strip()
            for command in ci_local.JOBS.values()
            for recipe in command.split("&&")
        }
        found = set()
        for job in ("lint", "check", "scenarios"):
            block = re.search(rf"(?ms)^  {job}:\n(.*?)(?=^  [\w-]+:\n|\Z)", WORKFLOW)
            self.assertIsNotNone(block, job)
            for line in re.findall(r"(?m)^\s+run: (just [^\n]*)$", block.group(1)):
                found.add(line.split(" | ")[0].split(" --partition")[0].strip())
        self.assertEqual(found, local)

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


class Stopping(unittest.TestCase):
    def test_sigterm_stops_the_run_and_its_child(self):
        previous = signal.getsignal(signal.SIGTERM)
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(ci_local, "LOGS", pathlib.Path(directory)):
            ci_local.stop_on_sigterm()
            log = ci_local.Log("a" * 40)
            threading.Timer(0.5, os.kill, (os.getpid(), signal.SIGTERM)).start()
            try:
                with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(KeyboardInterrupt):
                    # Prints its process ID, then waits.
                    log.stream(["sh", "-c", "echo $$; exec sleep 30"])
            finally:
                signal.signal(signal.SIGTERM, previous)
                log._file.close()
            child = int(log.path.read_text().split()[0])
            with self.assertRaises(ProcessLookupError):
                os.kill(child, 0)


class Offline(unittest.TestCase):
    def test_a_run_with_the_ticket_check_skipped_cannot_pass(self):
        with mock.patch.dict(os.environ, {ci_local.OFFLINE: "1"}), \
                mock.patch.object(ci_local, "post_status") as post:
            with self.assertRaises(ci_local.Refused):
                ci_local.settle("a" * 40, [], 100, mock.Mock())
            post.assert_not_called()

    def test_the_image_never_gets_the_host_environment(self):
        enter = (REPOSITORY / "tools/local-ci/enter.sh").read_text()
        self.assertNotIn("DESKTOP_CALLS_OFFLINE=", enter)
        self.assertNotIn("GH_TOKEN=", enter)
        self.assertIn("env -i", enter)


class Load(unittest.TestCase):
    """Local CI shares the Desktop with agents' builds (ticket #167)."""

    def test_a_quiet_desktop_runs_a_test_per_cpu_and_a_busy_one_the_least(self):
        self.assertEqual(ci_local.threads_for(0.0, 20), 20)
        self.assertEqual(ci_local.threads_for(1.4, 20), 17)
        self.assertEqual(ci_local.threads_for(10.0, 20), ci_local.MIN_THREADS)
        self.assertEqual(ci_local.threads_for(20.0, 20), ci_local.MIN_THREADS)

    def test_a_machine_with_fewer_cpus_than_the_least_runs_one_per_cpu(self):
        self.assertEqual(ci_local.threads_for(0.0, 2), 2)

    def test_the_cpu_time_read_from_the_kernel_is_busy_within_all(self):
        busy, total = ci_local.cpu_seconds()
        self.assertTrue(0 < busy <= total)

    def test_others_used_what_the_machine_spent_less_what_the_jobs_did(self):
        # 100 CPU-seconds were spent in 10 s; the jobs spent 60 of them.
        with mock.patch.object(ci_local, "cpu_seconds", side_effect=[(1000.0, 5000.0), (1100.0, 5200.0)]), \
                mock.patch.object(ci_local, "children_cpu_seconds", side_effect=[7.0, 67.0]), \
                mock.patch.object(ci_local.time, "monotonic", side_effect=[50.0, 60.0]):
            usage = ci_local.Usage()
            self.assertAlmostEqual(usage.others_cpus(), 4.0)

    def test_the_jobs_using_more_than_the_machine_spent_leave_others_nothing(self):
        with mock.patch.object(ci_local, "cpu_seconds", side_effect=[(1000.0, 5000.0), (1010.0, 5200.0)]), \
                mock.patch.object(ci_local, "children_cpu_seconds", side_effect=[0.0, 30.0]), \
                mock.patch.object(ci_local.time, "monotonic", side_effect=[50.0, 60.0]):
            self.assertEqual(ci_local.Usage().others_cpus(), 0.0)

    def test_a_pass_says_how_much_others_used_and_nothing_more(self):
        said = ci_local.blame(14.2, 20, failed=False)
        self.assertEqual(said, "other programs used about 14 of 20 CPUs while it ran")

    def test_a_failure_under_load_names_the_load_and_forbids_rerunning_until_green(self):
        said = ci_local.blame(14.2, 20, failed=True)
        self.assertIn("about 14 of 20 CPUs", said)
        self.assertIn("may be the machine's", said)
        self.assertIn("rerunning until it passes", said)

    def test_a_failure_on_a_quiet_desktop_is_the_changes(self):
        said = ci_local.blame(1.0, 20, failed=True)
        self.assertIn("about 1 of 20 CPUs", said)
        self.assertIn("treat the failure as the change's", said)

    def test_the_timings_start_at_once_on_a_calm_desktop(self):
        log = mock.Mock()
        with mock.patch.object(ci_local, "watch_others", return_value=1.0) as watch:
            ci_local.wait_for_calm(log, 20)
        self.assertEqual(watch.call_count, ci_local.CALM_LOOKS)
        log.line.assert_not_called()

    def test_the_timings_wait_for_a_build_to_end_and_say_so(self):
        log = mock.Mock()
        # A quiet look between two bursts doesn't count: three in a row do.
        looks = [15.0, 3.0, 12.0, 3.0, 2.0, 1.0]
        with mock.patch.object(ci_local, "watch_others", side_effect=looks) as watch:
            ci_local.wait_for_calm(log, 20)
        self.assertEqual(watch.call_count, 6)
        said = [call.args[0] for call in log.line.call_args_list]
        self.assertIn("waiting for others to use at most 4 of 20 CPUs (now about 15)", said[0])
        self.assertTrue(said[1].startswith("   calm after"))

    def test_the_timings_run_anyway_when_the_desktop_never_calms(self):
        log = mock.Mock()
        with mock.patch.object(ci_local, "watch_others", return_value=15.0), \
                mock.patch.object(ci_local.time, "monotonic", side_effect=[0.0, 1.0, 130.0]):
            ci_local.wait_for_calm(log, 20, give_up_after=120)
        self.assertIn("not calm after 120 s", log.line.call_args.args[0])
        self.assertIn("running anyway", log.line.call_args.args[0])

    def test_only_the_parallel_jobs_are_held_to_the_spare_cpus_and_timings_are_not_polite(self):
        self.assertTrue(ci_local.PARALLEL_JOBS <= set(ci_local.JOBS))
        self.assertTrue(ci_local.UNHURRIED_JOBS <= set(ci_local.JOBS))
        self.assertEqual(ci_local.PARALLEL_JOBS & ci_local.UNHURRIED_JOBS, set())
        # The first-frame timings run one at a time: nothing of ours beside them.
        self.assertIn("ci-first-frame-timings", ci_local.JOBS["timings"])
        self.assertNotIn("ci-first-frame-timings", ci_local.JOBS["scenarios"])

    def run_in_image(self, **options):
        log = mock.Mock()
        log.stream.return_value = 0
        with tempfile.TemporaryDirectory() as cache, \
                mock.patch.object(ci_local, "CACHE", pathlib.Path(cache)), \
                mock.patch.object(ci_local, "git", return_value="git@github.com:o/r.git"):
            ci_local.run_in_image(
                log, pathlib.Path("/src"), pathlib.Path("/tmp"), "just something", network=False,
                **options,
            )
        return log.stream.call_args.args[0]

    def test_a_job_with_a_thread_count_exports_it_for_every_test_runner(self):
        command = self.run_in_image(threads=5)
        self.assertEqual(
            command[-1],
            f"export RUST_TEST_THREADS=5 NEXTEST_TEST_THREADS=5 && {REPO_EXPORT} && just something",
        )

    def test_a_job_without_one_runs_its_command_as_it_is_but_for_the_repository(self):
        self.assertEqual(self.run_in_image()[-1], f"{REPO_EXPORT} && just something")

    def test_a_job_runs_under_nice_unless_it_is_not_polite(self):
        self.assertEqual(self.run_in_image()[:3], ["nice", "-n", "19"])
        self.assertEqual(self.run_in_image(polite=False)[0], "unshare")


class Image(unittest.TestCase):
    def test_an_old_image_is_built_again(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "rootfs").mkdir()
            built = root / "image-built"
            built.touch()
            old = time.time() - (ci_local.IMAGE_STALE_DAYS + 1) * 86400
            os.utime(built, (old, old))
            with mock.patch.object(ci_local, "ROOTFS", root / "rootfs"), \
                    mock.patch.object(ci_local, "BUILT", built), \
                    mock.patch.object(ci_local, "ROOT", root), \
                    mock.patch.object(
                        ci_local, "remove_tree", side_effect=lambda p: shutil.rmtree(p, ignore_errors=True)
                    ) as remove, \
                    mock.patch.object(ci_local, "pull_base_layers", side_effect=RuntimeError("pulled")):
                with self.assertRaisesRegex(RuntimeError, "pulled"):
                    ci_local.build_image(mock.Mock(), rebuild=False)
            remove.assert_any_call(root / "rootfs")

    def test_a_fresh_image_is_kept(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "rootfs").mkdir()
            built = root / "image-built"
            built.touch()
            with mock.patch.object(ci_local, "ROOTFS", root / "rootfs"), \
                    mock.patch.object(ci_local, "BUILT", built), \
                    mock.patch.object(ci_local, "pull_base_layers") as pull:
                ci_local.build_image(mock.Mock(), rebuild=False)
            pull.assert_not_called()


class Pushing(unittest.TestCase):
    SHA = "c" * 40

    def push(self, branch):
        calls = []

        def run(command, **kwargs):
            calls.append(command)
            return subprocess.CompletedProcess(command, 0, "", "")

        with mock.patch.object(ci_local, "git", return_value=branch), \
                mock.patch.object(ci_local.subprocess, "run", side_effect=run):
            ci_local.push_branch(self.SHA, mock.Mock())
        return calls

    def test_never_pushes_main(self):
        with mock.patch.object(ci_local, "git", return_value="main"), \
                mock.patch.object(ci_local.subprocess, "run") as run:
            with self.assertRaisesRegex(ci_local.Refused, "main"):
                ci_local.push_branch(self.SHA, mock.Mock())
            run.assert_not_called()

    def test_never_pushes_a_detached_head(self):
        with mock.patch.object(ci_local, "git", return_value=""), \
                mock.patch.object(ci_local.subprocess, "run") as run:
            with self.assertRaisesRegex(ci_local.Refused, "detached"):
                ci_local.push_branch(self.SHA, mock.Mock())
            run.assert_not_called()

    def test_pushes_the_tested_commit_never_the_branch_ref(self):
        calls = self.push("ticket/7-x")
        pushes = [command for command in calls if command[:2] == ["git", "push"]]
        self.assertEqual(pushes, [["git", "push", "origin", f"{self.SHA}:refs/heads/ticket/7-x"]])
        self.assertNotIn("--force", " ".join(pushes[0]))

    def test_a_failed_push_posts_nothing(self):
        def fail(command, **kwargs):
            return subprocess.CompletedProcess(command, 1, "", "rejected")

        with mock.patch.object(ci_local, "git", return_value="ticket/7-x"), \
                mock.patch.object(ci_local.subprocess, "run", side_effect=fail):
            with self.assertRaisesRegex(ci_local.Refused, "pushing ticket/7-x failed"):
                ci_local.push_branch(self.SHA, mock.Mock())


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


class Stopped(unittest.TestCase):
    """A failed lint ends the run: head 8b3c277 failed lint in 4 s and the
    other jobs still ran for 205 s."""

    def run_jobs(self, statuses: dict[str, int], jobs: list[str]) -> tuple[list[str], list[str]]:
        ran = []

        def run_in_image(log, scratch, tmp, command, **options):
            job = next(name for name, text in ci_local.JOBS.items() if text == command) \
                if command != ci_local.PREPARE else "prepare"
            ran.append(job)
            return statuses.get(job, 0)

        with tempfile.TemporaryDirectory() as runs, contextlib.redirect_stdout(io.StringIO()), \
                mock.patch.object(ci_local, "RUNS", pathlib.Path(runs)), \
                mock.patch.object(ci_local, "checkout"), \
                mock.patch.object(ci_local, "remove_tree"), \
                mock.patch.object(ci_local, "watch_others", return_value=0.0), \
                mock.patch.object(ci_local, "wait_for_calm"), \
                mock.patch.object(ci_local, "run_in_image", side_effect=run_in_image):
            log = mock.Mock()
            log.name = "run"
            failed, _ = ci_local.run_jobs(log, "a" * 40, jobs)
        return failed, ran[1:]

    def test_a_failed_lint_runs_no_other_job(self):
        failed, ran = self.run_jobs({"lint": 1}, list(ci_local.JOBS))
        self.assertEqual((failed, ran), (["lint"], ["lint"]))

    def test_a_failed_test_job_still_lets_the_scenarios_run(self):
        failed, ran = self.run_jobs({"test": 1}, list(ci_local.JOBS))
        self.assertEqual(failed, ["test"])
        self.assertEqual(ran, list(ci_local.JOBS))


class DocsOnly(unittest.TestCase):
    """`just ci-local` tests only lint when the change from origin/main is
    docs that no code names (tools/changes.py, the rule GitHub CI uses)."""

    ENV = {
        "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.com",
        "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.com",
    }

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = pathlib.Path(directory.name)
        patcher = mock.patch.dict(os.environ, self.ENV)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.origin, self.work = root / "origin.git", root / "work"
        self.git("init", "--quiet", "--bare", "--initial-branch=main", str(self.origin), cwd=root)
        self.git("clone", "--quiet", str(self.origin), str(self.work), cwd=root)
        self.commit("justfile", "lint:\n", "Start")
        self.git("push", "--quiet", "origin", "HEAD:main")
        self.git("checkout", "--quiet", "-b", "change")
        previous = os.getcwd()
        os.chdir(self.work)
        self.addCleanup(os.chdir, previous)

    def git(self, *args: str, cwd: pathlib.Path | None = None) -> str:
        return subprocess.run(
            ["git", *args], cwd=cwd or self.work, check=True, capture_output=True, text=True
        ).stdout.strip()

    def commit(self, name: str, text: str, subject: str) -> str:
        path = self.work / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        self.git("add", name)
        self.git("commit", "--quiet", "-m", subject)
        return self.git("rev-parse", "HEAD")

    def docs_only(self, sha: str) -> tuple[bool, str]:
        log = mock.Mock()
        return ci_local.is_docs_only(sha, log), " ".join(call.args[0] for call in log.line.call_args_list)

    def test_a_change_to_docs_alone_is_docs_only(self):
        sha = self.commit("docs/fixture/page.md", "words\n", "Docs")
        self.assertEqual(self.docs_only(sha)[0], True)

    def test_a_change_that_touches_code_is_not(self):
        self.commit("docs/fixture/page.md", "words\n", "Docs")
        sha = self.commit("justfile", "lint:\n\techo\n", "Code")
        self.assertEqual(self.docs_only(sha)[0], False)

    def test_docs_that_code_names_are_not_docs_only(self):
        self.commit("tools/reader.py", "open('docs/fixture/page.md')\n", "Code that reads a page")
        self.git("push", "--quiet", "origin", "HEAD:main")
        sha = self.commit("docs/fixture/page.md", "words\n", "Docs")
        self.assertEqual(self.docs_only(sha)[0], False)

    def test_a_head_that_lacks_a_newer_origin_main_is_not_docs_only(self):
        sha = self.commit("docs/fixture/page.md", "words\n", "Docs")
        self.git("checkout", "--quiet", "main")
        self.commit("crates/x.rs", "fn main() {}\n", "Code lands on main")
        self.git("push", "--quiet", "origin", "HEAD:main")
        self.git("checkout", "--quiet", "change")
        answer, said = self.docs_only(sha)
        self.assertEqual(answer, False)
        self.assertIn("doesn't contain origin/main", said)

    def test_a_head_that_merged_the_newer_origin_main_is_judged_from_it(self):
        self.commit("docs/fixture/page.md", "words\n", "Docs")
        self.git("checkout", "--quiet", "main")
        self.commit("crates/x.rs", "fn main() {}\n", "Code lands on main")
        self.git("push", "--quiet", "origin", "HEAD:main")
        self.git("checkout", "--quiet", "change")
        self.git("merge", "--quiet", "--no-edit", "main")
        self.assertEqual(self.docs_only(self.git("rev-parse", "HEAD"))[0], True)

    def test_a_failed_fetch_is_not_docs_only(self):
        sha = self.commit("docs/fixture/page.md", "words\n", "Docs")
        self.git("remote", "set-url", "origin", str(self.origin) + "-gone")
        self.assertEqual(self.docs_only(sha)[0], False)


class Choosing(unittest.TestCase):
    """What main() runs and what it posts."""

    SHA = "a" * 40

    def main(self, *arguments: str, docs_only: bool) -> tuple[mock.Mock, mock.Mock]:
        run_jobs = mock.Mock(return_value=([], pathlib.Path("/tmp/x")))
        settle = mock.Mock(return_value=ci_local.EXIT_PASSED)
        with contextlib.redirect_stdout(io.StringIO()), \
                mock.patch("sys.argv", ["ci_local.py", *arguments]), \
                mock.patch.object(ci_local, "require_online"), \
                mock.patch.object(ci_local, "require_clean_head", return_value=self.SHA), \
                mock.patch.object(ci_local, "worktree", return_value="/work/here"), \
                mock.patch.object(ci_local, "Claim"), \
                mock.patch.object(ci_local, "take_turn"), \
                mock.patch.object(ci_local, "Log"), \
                mock.patch.object(ci_local, "clear_stale_test_folders", return_value=[]), \
                mock.patch.object(ci_local, "prune_old_runs"), \
                mock.patch.object(ci_local, "build_image"), \
                mock.patch.object(ci_local, "remove_tree"), \
                mock.patch.object(ci_local, "is_docs_only", return_value=docs_only), \
                mock.patch.object(ci_local, "run_jobs", run_jobs), \
                mock.patch.object(ci_local, "settle", settle):
            ci_local.main()
        return run_jobs, settle

    def test_a_docs_only_change_runs_lint_alone_and_posts_the_status(self):
        run_jobs, settle = self.main(docs_only=True)
        self.assertEqual(run_jobs.call_args.args[2], ["lint"])
        self.assertEqual(settle.call_args.args[4], True)

    def test_a_change_with_code_runs_every_job(self):
        run_jobs, settle = self.main(docs_only=False)
        self.assertEqual(run_jobs.call_args.args[2], list(ci_local.JOBS))
        self.assertEqual(settle.call_args.args[4], False)

    def test_full_runs_every_job_for_docs(self):
        run_jobs, _ = self.main("--full", docs_only=True)
        self.assertEqual(run_jobs.call_args.args[2], list(ci_local.JOBS))

    def test_named_jobs_run_as_named_for_docs(self):
        run_jobs, settle = self.main("lint", "test", docs_only=True)
        self.assertEqual(run_jobs.call_args.args[2], ["lint", "test"])
        settle.assert_not_called()

    def test_the_status_says_the_run_was_docs_only(self):
        log = mock.Mock()
        log.display_path.return_value = "~/.cache/desktop-calls-ci/logs/x.log"
        text = ci_local.status_description("success", 40, [], log, docs_only=True)
        self.assertIn("docs only, lint alone", text)
        self.assertLessEqual(len(text), 140)


class Queued(unittest.TestCase):
    """A run waits its turn; `--cancel` and the refusal of a second run touch
    only the run started from the same worktree. On 2026-11-13 an agent ran
    `pkill -f 'tools/ci_local.py'` to replace its own queued run and killed
    two other agents' runs, one at 346 of 348 scenarios."""

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = pathlib.Path(directory.name)
        for name, value in (("ROOT", root), ("CLAIMS", root / "claims"), ("LOCK", root / "lock")):
            patcher = mock.patch.object(ci_local, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        self.children: list[subprocess.Popen] = []
        self.addCleanup(self.stop_children)

    def stop_children(self):
        for child in self.children:
            child.kill()
            child.wait()

    def run_in(self, directory: str) -> tuple[ci_local.Claim, subprocess.Popen]:
        """A stand-in for a ci-local run, which `is_ci_local` takes for one
        by its command line, with its claim on `directory`."""
        child = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(60)", "ci_local.py"],
        )
        self.children.append(child)
        claim = ci_local.Claim(directory)
        ci_local.CLAIMS.mkdir(parents=True, exist_ok=True)
        claim.path.write_text(
            json.dumps({"pid": child.pid, "worktree": directory, "sha": "a" * 40,
                        "state": "waiting", "since": "20:41:00 UTC"})
        )
        return claim, child

    def test_a_second_run_from_a_worktree_is_refused_naming_the_first(self):
        _, child = self.run_in("/work/here")
        with self.assertRaises(ci_local.Refused) as raised:
            ci_local.Claim("/work/here").take("b" * 40)
        message = str(raised.exception)
        self.assertIn(f"process {child.pid}", message)
        self.assertIn("waiting since 20:41:00 UTC", message)
        self.assertIn("just ci-local --cancel", message)

    def test_a_run_from_another_worktree_is_not_refused(self):
        self.run_in("/work/elsewhere")
        ci_local.Claim("/work/here").take("b" * 40)

    def test_a_claim_whose_run_is_gone_is_replaced(self):
        claim, child = self.run_in("/work/here")
        child.kill()
        child.wait()
        claim.take("b" * 40)
        self.assertEqual(json.loads(claim.path.read_text())["pid"], os.getpid())

    def test_a_process_id_that_is_not_a_ci_local_run_is_not_a_claim(self):
        claim = ci_local.Claim("/work/here")
        ci_local.CLAIMS.mkdir(parents=True)
        claim.path.write_text(json.dumps({"pid": os.getpid(), "worktree": "/work/here",
                                          "sha": "a" * 40, "state": "running", "since": "now"}))
        self.assertIsNone(claim.read())

    def test_a_finished_run_gives_its_claim_up(self):
        claim = ci_local.Claim("/work/here")
        with mock.patch.object(ci_local, "is_ci_local", return_value=True):
            claim.take("a" * 40)
            claim.release()
        self.assertFalse(claim.path.exists())

    def test_cancel_stops_this_worktrees_run_and_no_other(self):
        _, mine = self.run_in("/work/here")
        _, theirs = self.run_in("/work/elsewhere")
        with contextlib.redirect_stdout(io.StringIO()) as printed:
            status = ci_local.cancel("/work/here")
        self.assertEqual(status, ci_local.EXIT_PASSED)
        self.assertIsNotNone(mine.poll())
        self.assertIsNone(theirs.poll())
        self.assertIn(f"process {mine.pid}", printed.getvalue())

    def test_cancel_with_nothing_queued_stops_nothing(self):
        _, theirs = self.run_in("/work/elsewhere")
        with contextlib.redirect_stdout(io.StringIO()) as printed:
            status = ci_local.cancel("/work/here")
        self.assertEqual(status, ci_local.EXIT_PASSED)
        self.assertIsNone(theirs.poll())
        self.assertIn("No ci-local run is queued or running", printed.getvalue())

    def test_a_run_that_must_wait_says_it_tests_head_as_it_is_when_its_turn_comes(self):
        holder = ci_local.take_turn()
        threading.Timer(0.5, holder.close).start()
        with contextlib.redirect_stdout(io.StringIO()) as printed:
            ci_local.take_turn().close()
        text = printed.getvalue()
        self.assertIn("tests HEAD as it is when its turn comes", text)
        self.assertIn("committing while it waits is fine", text)
        self.assertIn("just ci-local --cancel", text)

    def test_the_wait_names_the_run_that_has_the_lock(self):
        claim, child = self.run_in("/work/elsewhere")
        claim.path.write_text(
            json.dumps({"pid": child.pid, "worktree": "/work/elsewhere", "sha": "a" * 40,
                        "state": "running", "since": "20:41:00 UTC"})
        )
        self.assertIn(f"/work/elsewhere, process {child.pid}", ci_local.running_elsewhere())


if __name__ == "__main__":
    unittest.main()
