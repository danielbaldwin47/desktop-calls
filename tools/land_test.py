#!/usr/bin/env python3
"""Tests of `just land` (tools/land.py, ticket #247): bring a PR up to date,
test the result and merge it, holding the CI lane. GitHub is never asked, and
nothing is run in the image. The git steps run against a throwaway repository.
`just lint` runs them."""

from __future__ import annotations

import contextlib
import fcntl
import io
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ci_local  # noqa: E402
import land  # noqa: E402
import merge  # noqa: E402

ENV = {
    "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.com",
    "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.com",
}
SHA, MERGED = "a" * 40, "b" * 40


class Decide(unittest.TestCase):
    """What to do with a head, from where it stands against the base."""

    def test_a_green_head_that_contains_main_merges_with_no_run(self):
        self.assertEqual(land.decide(behind=False, gap_has_code=False, status="success"), land.MERGE_NOW)

    def test_a_green_head_lacking_only_docs_is_let_through_with_no_run(self):
        self.assertEqual(land.decide(behind=True, gap_has_code=False, status="success"), land.MERGE_NOW)

    def test_a_green_head_lacking_code_is_updated_and_run_again(self):
        self.assertEqual(land.decide(behind=True, gap_has_code=True, status="success"), land.UPDATE_AND_RUN)

    def test_a_head_never_run_is_run_and_one_behind_main_is_updated_first(self):
        self.assertEqual(land.decide(behind=False, gap_has_code=False, status=None), land.RUN)
        self.assertEqual(land.decide(behind=False, gap_has_code=False, status="pending"), land.RUN)
        self.assertEqual(land.decide(behind=True, gap_has_code=False, status=None), land.UPDATE_AND_RUN)

    def test_a_red_head_with_nothing_new_to_test_is_refused_not_rerun(self):
        self.assertEqual(land.decide(behind=False, gap_has_code=False, status="failure"), land.RED)
        self.assertEqual(land.decide(behind=True, gap_has_code=False, status="error"), land.RED)

    def test_a_red_head_that_must_take_main_is_run_on_the_merge_as_a_new_commit(self):
        self.assertEqual(land.decide(behind=True, gap_has_code=True, status="failure"), land.UPDATE_AND_RUN)


class InARepository(unittest.TestCase):
    """A repository with a main, a branch, and `origin/*` refs made by hand."""

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
        self.commit("shared.txt", "one\n", "Start")

    def git(self, *args: str) -> str:
        result = subprocess.run(["git", *args], capture_output=True, text=True, check=True)
        return result.stdout.strip()

    def commit(self, path: str, text: str, message: str) -> str:
        (self.root / path).parent.mkdir(parents=True, exist_ok=True)
        (self.root / path).write_text(text)
        self.git("add", "--all")
        self.git("commit", "--quiet", "-m", message)
        return self.git("rev-parse", "HEAD")

    def head(self) -> str:
        return self.git("rev-parse", "HEAD")

    def remote(self, name: str, sha: str) -> None:
        self.git("update-ref", f"refs/remotes/origin/{name}", sha)

    def branch_off_main(self) -> str:
        """Switches to `work`, one commit past main; returns main's sha."""
        main = self.head()
        self.git("switch", "--quiet", "-c", "work")
        self.commit("work.txt", "mine\n", "Work")
        return main

    def main_moves(self, path: str, text: str) -> None:
        """origin/main gains a commit that branches off where `work` did."""
        here = self.git("rev-parse", "--abbrev-ref", "HEAD")
        self.git("switch", "--quiet", "main")
        self.remote("main", self.commit(path, text, f"Main changes {path}"))
        self.git("switch", "--quiet", here)

    # -- what origin/main has that the head lacks

    def test_a_gap_of_docs_that_no_code_names_does_not_change_code(self):
        self.branch_off_main()
        self.main_moves("docs/fixture/page.md", "words\n")
        self.assertFalse(land.gap_changes_code("origin/main")[0])

    def test_a_gap_that_touches_code_changes_code(self):
        self.branch_off_main()
        self.main_moves("src/lib.rs", "fn main() {}\n")
        changed, why = land.gap_changes_code("origin/main")
        self.assertTrue(changed)
        self.assertIn("src/lib.rs", why)

    def test_a_gap_of_docs_that_the_head_names_changes_code(self):
        self.git("switch", "--quiet", "-c", "work")
        self.commit("src/reader.rs", '// reads docs/fixture/page.md\n', "Code reads the page")
        self.git("switch", "--quiet", "main")
        self.remote("main", self.commit("docs/fixture/page.md", "words\n", "Docs change"))
        self.git("switch", "--quiet", "work")
        self.assertTrue(land.gap_changes_code("origin/main")[0])

    # -- the branch against its copy on GitHub

    def test_a_branch_behind_its_copy_on_github_is_fast_forwarded(self):
        self.branch_off_main()
        ahead = self.commit("more.txt", "more\n", "More")
        self.git("reset", "--quiet", "--hard", "HEAD~1")
        self.remote("work", ahead)
        land.level_with_github("work")
        self.assertEqual(self.head(), ahead)

    def test_a_branch_ahead_of_its_copy_is_left_for_ci_local_to_push(self):
        self.branch_off_main()
        behind = self.head()
        self.remote("work", behind)
        mine = self.commit("more.txt", "more\n", "More")
        land.level_with_github("work")
        self.assertEqual(self.head(), mine)

    def test_a_diverged_branch_is_refused_and_left_alone(self):
        self.branch_off_main()
        mine = self.head()
        self.git("switch", "--quiet", "main")
        self.git("switch", "--quiet", "-c", "elsewhere")
        self.remote("work", self.commit("other.txt", "theirs\n", "Pushed from elsewhere"))
        self.git("switch", "--quiet", "work")
        with self.assertRaises(merge.Refused) as raised:
            land.level_with_github("work")
        self.assertIn("Next: git merge --no-edit origin/work", str(raised.exception))
        self.assertEqual(self.head(), mine)

    # -- merging main in

    def test_a_clean_merge_makes_one_commit_with_main_in_its_history(self):
        self.branch_off_main()
        self.main_moves("main-only.txt", "from main\n")
        land.update_from_base("main", 42)
        self.assertTrue(land.is_ancestor("origin/main", "HEAD"))
        self.assertTrue((self.root / "main-only.txt").exists())
        self.assertEqual(len(self.git("rev-list", "--merges", "HEAD").split()), 1)

    def test_a_conflict_aborts_the_merge_and_names_the_command_to_run_next(self):
        self.git("switch", "--quiet", "-c", "work")
        mine = self.commit("shared.txt", "mine\n", "Work changes the shared file")
        self.git("switch", "--quiet", "main")
        self.remote("main", self.commit("shared.txt", "theirs\n", "Main changes it too"))
        self.git("switch", "--quiet", "work")
        with self.assertRaises(merge.Refused) as raised:
            land.update_from_base("main", 42)
        message = str(raised.exception)
        self.assertIn("conflicts in shared.txt", message)
        self.assertIn("nothing was run and nothing was posted", message)
        self.assertIn("Next: git merge --no-edit origin/main   (resolve the conflicts, commit, "
                      "then `just land 42`)", message)
        self.assertEqual(self.head(), mine)
        self.assertEqual(self.git("status", "--porcelain"), "", "the aborted merge left the tree clean")


class Interrupted(unittest.TestCase):
    def test_an_interrupt_during_the_merge_aborts_it_and_goes_on_up(self):
        calls: list[list[str]] = []

        def run(command, **_):
            calls.append(command)
            if command[:2] == ["git", "merge"] and "--abort" not in command:
                raise KeyboardInterrupt
            return subprocess.CompletedProcess(command, 0, "", "")

        with mock.patch.object(land.subprocess, "run", run), self.assertRaises(KeyboardInterrupt):
            land.update_from_base("main", 42)
        self.assertEqual(calls[-1], ["git", "merge", "--abort"])


class Landing(unittest.TestCase):
    """The order of a landing, with every step stubbed and recorded."""

    def land(self, status: str | None, behind: bool, gap_has_code: bool, *, run_passes: bool = True,
             conflict: bool = False, ticket_refused: bool = False) -> list[str]:
        steps: list[str] = []

        def step(name, result=None):
            def call(*args, **_):
                steps.append(name)
                return result
            return call

        def update(base, number):
            steps.append("merge main in")
            if conflict:
                raise merge.Refused("Merging origin/main into the branch conflicts in x.\nNext: ...")

        def run(sha, jobs, every_job, full, rebuild, log):
            steps.append(f"run {'every job' if jobs == list(ci_local.JOBS) else jobs} on {sha[:7]}")
            return ci_local.EXIT_PASSED if run_passes else ci_local.EXIT_FAILED

        def ticket(pr):
            steps.append("ticket check")
            if ticket_refused:
                raise merge.Refused("Real-Phone Checklist steps wait on closed tickets")

        pr = {"number": 42, "state": "OPEN", "isDraft": False, "headRefName": "work",
              "baseRefName": "main", "headRefOid": SHA}
        look = land.Survey("work", "main", SHA, behind, gap_has_code, "a code change", status)
        with (
            mock.patch.object(merge, "pull_request", return_value=pr),
            mock.patch.object(land, "require_here"),
            mock.patch.object(ci_local, "require_clean_head", return_value=SHA),
            mock.patch.object(merge, "require_no_step_waits_on_a_closed_ticket", ticket),
            mock.patch.object(land, "survey", step("look", look)),
            mock.patch.object(land, "update_from_base", update),
            mock.patch.object(land, "git", return_value=MERGED + "\n"),
            mock.patch.object(ci_local, "Log"),
            mock.patch.object(ci_local, "clear_stale_test_folders"),
            mock.patch.object(ci_local, "prune_old_runs"),
            mock.patch.object(ci_local, "run_and_post", run),
            mock.patch.object(land, "wait_for_head_on_github", lambda n, sha: steps.append(f"wait for {sha[:7]}")),
            mock.patch.object(merge, "merge", lambda n, wait, lane_held: steps.append(
                f"merge {n} with wait {wait}, lane held: {lane_held}") or 0),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            try:
                land.holding_the_lane(42)
            except merge.Refused:
                steps.append("refused")
        return steps

    def test_a_head_behind_main_by_code_takes_main_runs_everything_and_merges_the_result(self):
        self.assertEqual(
            self.land("success", behind=True, gap_has_code=True),
            ["ticket check", "look", "merge main in", f"run every job on {MERGED[:7]}",
             f"wait for {MERGED[:7]}", "merge 42 with wait 0, lane held: True"],
        )

    def test_a_docs_only_gap_with_a_green_head_runs_nothing_and_merges(self):
        self.assertEqual(
            self.land("success", behind=True, gap_has_code=False),
            ["ticket check", "look", f"wait for {SHA[:7]}", "merge 42 with wait 0, lane held: True"],
        )

    def test_a_green_head_that_contains_main_runs_nothing_and_merges(self):
        self.assertEqual(
            self.land("success", behind=False, gap_has_code=False),
            ["ticket check", "look", f"wait for {SHA[:7]}", "merge 42 with wait 0, lane held: True"],
        )

    def test_a_head_with_no_run_yet_is_run_as_it_is_and_merged(self):
        self.assertEqual(
            self.land(None, behind=False, gap_has_code=False),
            ["ticket check", "look", f"run every job on {SHA[:7]}", f"wait for {SHA[:7]}",
             "merge 42 with wait 0, lane held: True"],
        )

    def test_a_red_head_is_refused_with_nothing_run_or_merged(self):
        self.assertEqual(self.land("failure", behind=False, gap_has_code=False),
                         ["ticket check", "look", "refused"])

    def test_a_red_run_on_the_merge_merges_nothing(self):
        steps = self.land("success", behind=True, gap_has_code=True, run_passes=False)
        self.assertEqual(steps[-2:], [f"run every job on {MERGED[:7]}", "refused"])
        self.assertFalse([step for step in steps if step.startswith("merge 42")])

    def test_a_conflict_runs_and_merges_nothing(self):
        self.assertEqual(self.land("success", behind=True, gap_has_code=True, conflict=True),
                         ["ticket check", "look", "merge main in", "refused"])

    def test_a_ticket_the_checklist_waits_on_is_refused_before_any_run(self):
        self.assertEqual(self.land("success", behind=True, gap_has_code=True, ticket_refused=True),
                         ["ticket check", "refused"])

    def test_it_merges_the_commit_the_run_tested_not_the_one_it_started_with(self):
        steps = self.land(None, behind=True, gap_has_code=False)
        self.assertIn(f"wait for {MERGED[:7]}", steps)
        self.assertNotIn(f"wait for {SHA[:7]}", steps)


class TakingTheLane(unittest.TestCase):
    """`land` holds the lock before it looks at main and gives the claim up
    whatever happens."""

    def land(self, inside) -> tuple[list[str], int | None, Exception | None]:
        steps: list[str] = []
        pr = {"number": 42, "state": "OPEN", "isDraft": False, "headRefName": "work", "baseRefName": "main"}
        claim = mock.Mock()
        claim.take.side_effect = lambda sha: steps.append("claim")
        claim.release.side_effect = lambda: steps.append("release")
        with (
            mock.patch.object(merge, "pull_request", return_value=pr),
            mock.patch.object(land, "require_here"),
            mock.patch.object(ci_local, "require_online"),
            mock.patch.object(ci_local, "require_clean_head", return_value=SHA),
            mock.patch.object(ci_local, "stop_on_sigterm"),
            mock.patch.object(ci_local, "worktree", return_value="/work/here"),
            mock.patch.object(ci_local, "Claim", return_value=claim),
            mock.patch.object(ci_local, "take_turn", lambda: steps.append("lock")),
            mock.patch.object(land, "holding_the_lane", inside(steps)),
        ):
            try:
                return steps, land.land(42), None
            except Exception as error:  # noqa: BLE001 - the test reports what escaped
                return steps, None, error

    def test_it_claims_the_worktree_and_takes_the_lock_before_it_looks_at_main(self):
        steps, code, _ = self.land(lambda steps: lambda number: steps.append("look at main") or 0)
        self.assertEqual((steps, code), (["claim", "lock", "look at main", "release"], 0))

    def test_a_refusal_inside_still_gives_the_claim_up(self):
        def refuses(steps):
            def inside(number):
                steps.append("look at main")
                raise merge.Refused("no")
            return inside

        steps, _, error = self.land(refuses)
        self.assertIsInstance(error, merge.Refused)
        self.assertEqual(steps[-1], "release")


class HoldingTheLock(unittest.TestCase):
    """The lock is really held, with the real `take_turn`, from before the
    first look at main to the end of the landing, and let go after it."""

    def test_another_run_cannot_take_the_lock_while_a_landing_is_inside(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            seen: list[str] = []

            def inside(number):
                other = open(root / "lock", "w")
                try:
                    fcntl.flock(other, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    seen.append("free")
                except BlockingIOError:
                    seen.append("held")
                finally:
                    other.close()
                return 0

            pr = {"number": 42, "state": "OPEN", "isDraft": False, "headRefName": "work", "baseRefName": "main"}
            with (
                mock.patch.object(ci_local, "ROOT", root),
                mock.patch.object(ci_local, "LOCK", root / "lock"),
                mock.patch.object(ci_local, "CLAIMS", root / "claims"),
                mock.patch.object(merge, "pull_request", return_value=pr),
                mock.patch.object(land, "require_here"),
                mock.patch.object(ci_local, "require_online"),
                mock.patch.object(ci_local, "require_clean_head", return_value=SHA),
                mock.patch.object(ci_local, "stop_on_sigterm"),
                mock.patch.object(ci_local, "worktree", return_value="/work/here"),
                mock.patch.object(land, "holding_the_lane", inside),
            ):
                land.land(42)
            after = open(root / "lock", "w")
            self.addCleanup(after.close)
            fcntl.flock(after, fcntl.LOCK_EX | fcntl.LOCK_NB)  # raises if still held
        self.assertEqual(seen, ["held"])


class Placing(unittest.TestCase):
    """Where `land` may run."""

    PR = {"number": 42, "headRefName": "work", "baseRefName": "main", "isDraft": False}

    def refusal(self, pr: dict, current: str) -> str:
        with (
            mock.patch.object(land, "git", return_value=current + "\n"),
            mock.patch.object(merge, "checkout_of", return_value="/work/agent"),
            self.assertRaises(merge.Refused) as raised,
        ):
            land.require_here(pr)
        return str(raised.exception)

    def test_another_branch_checked_out_is_sent_to_the_worktree_that_has_the_branch(self):
        self.assertIn("Next: cd /work/agent && just land 42", self.refusal(self.PR, "other"))

    def test_a_pr_not_on_main_is_sent_to_merge(self):
        message = self.refusal({**self.PR, "baseRefName": "release/1.0"}, "work")
        self.assertIn("Next: just merge 42", message)

    def test_a_draft_is_refused(self):
        self.assertIn("Next: gh pr ready 42", self.refusal({**self.PR, "isDraft": True}, "work"))

    def test_the_branch_checked_out_on_a_ready_pr_to_main_is_fine(self):
        with mock.patch.object(land, "git", return_value="work\n"):
            land.require_here(self.PR)


class Waiting(unittest.TestCase):
    def test_it_waits_for_github_to_show_the_pushed_head(self):
        shown = iter([{"headRefOid": "c" * 40}, {"headRefOid": SHA}])
        with (
            mock.patch.object(merge, "pull_request", side_effect=lambda n: next(shown)),
            mock.patch.object(land.time, "sleep") as sleep,
        ):
            land.wait_for_head_on_github(42, SHA)
        sleep.assert_called_once()

    def test_it_gives_up_with_the_command_to_push(self):
        with (
            mock.patch.object(merge, "pull_request", return_value={"headRefOid": "c" * 40}),
            mock.patch.object(land.time, "sleep"),
            mock.patch.object(land.time, "monotonic", side_effect=[0, 0, land.HEAD_SHOWS_SECONDS + 1]),
            self.assertRaises(merge.Refused) as raised,
        ):
            land.wait_for_head_on_github(42, SHA)
        self.assertIn("Next: git push origin HEAD, then `just land 42`", str(raised.exception))


class TheStatus(unittest.TestCase):
    def ci_state(self, returncode: int, stdout: str = "", stderr: str = "") -> str | None:
        done = subprocess.CompletedProcess([], returncode, stdout, stderr)
        with mock.patch.object(land.subprocess, "run", return_value=done):
            return land.ci_state(SHA)

    def test_the_newest_status_is_read(self):
        self.assertEqual(self.ci_state(0, "success\n"), "success")

    def test_no_status_is_none(self):
        self.assertIsNone(self.ci_state(0, "null\n"))
        self.assertIsNone(self.ci_state(0, ""))

    def test_a_commit_github_lacks_is_none(self):
        self.assertIsNone(self.ci_state(1, stderr="gh: Not Found (HTTP 404)"))

    def test_any_other_failure_is_refused_not_taken_for_no_status(self):
        with self.assertRaises(merge.Refused):
            self.ci_state(1, stderr="gh: authentication required")


if __name__ == "__main__":
    unittest.main()
