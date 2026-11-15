#!/usr/bin/env python3
"""Tests of `just land` (tools/land.py, ticket #247): bring a PR up to date,
wait for GitHub's CI on the result and merge it, holding the landing lock;
then clear target/ (#286, #277). GitHub is never asked. The git steps run against a throwaway repository.
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

    def test_a_branch_ahead_of_its_copy_is_left_to_be_pushed(self):
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
        self.assertIn("nothing was pushed or merged", message)
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


MERGE = f"merge 42 waiting {land.CI_WAIT_MINUTES} min, lock held: True"


class Landing(unittest.TestCase):
    """The order of a landing, with every step stubbed and recorded."""

    def land(self, behind: bool, gap_has_code: bool, *, on_github: bool = True, conflict: bool = False,
             ticket_refused: bool = False, unreviewed: bool = False) -> list[str]:
        steps: list[str] = []
        self.refusal = ""

        def reviewed(pr, head=None):
            steps.append(f"reviews {(head or pr['headRefOid'])[:7]}")
            if unreviewed:
                raise merge.Refused("PR #42 has no passed review recorded")

        def update(base, number):
            steps.append("merge main in")
            if conflict:
                raise merge.Refused("Merging origin/main into the branch conflicts in x.\nNext: ...")

        def ticket(pr, head=None):
            steps.append(f"ticket check {(head or pr['headRefOid'])[:7]}")
            if ticket_refused:
                raise merge.Refused("Real-Phone Checklist steps wait on closed tickets")

        def ancestor(older, newer):
            return on_github and older == SHA

        pr = {"number": 42, "state": "OPEN", "isDraft": False, "headRefName": "work",
              "baseRefName": "main", "headRefOid": SHA}
        look = land.Survey("work", "main", SHA, behind, gap_has_code, "a code change")
        with (
            mock.patch.object(merge, "pull_request", return_value=pr),
            mock.patch.object(land, "require_here"),
            mock.patch.object(ci_local, "require_clean_head", return_value=SHA),
            mock.patch.object(merge, "require_no_step_waits_on_a_closed_ticket", ticket),
            mock.patch.object(merge, "require_reviewed", reviewed),
            mock.patch.object(land, "survey", lambda pr, level: steps.append("look") or look),
            mock.patch.object(land, "update_from_base", update),
            mock.patch.object(land, "git", return_value=MERGED + "\n"),
            mock.patch.object(land, "is_ancestor", ancestor),
            mock.patch.object(land, "push", lambda branch, sha: steps.append(f"push {sha[:7]}")),
            mock.patch.object(land, "wait_for_head_on_github", lambda n, sha: steps.append(f"wait for {sha[:7]}")),
            mock.patch.object(merge, "merge", lambda n, wait, lane_held: steps.append(
                f"merge {n} waiting {wait} min, lock held: {lane_held}") or 0),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            try:
                land.holding_the_lane(42)
            except merge.Refused as refusal:
                self.refusal = str(refusal)
                steps.append("refused")
        return steps

    def test_a_head_behind_main_by_code_takes_main_pushes_and_merges_once_github_passes_it(self):
        self.assertEqual(
            self.land(behind=True, gap_has_code=True),
            ["look", f"reviews {SHA[:7]}", "merge main in", f"ticket check {MERGED[:7]}", f"push {MERGED[:7]}",
             f"wait for {MERGED[:7]}", MERGE],
        )

    def test_a_docs_only_gap_pushes_nothing_and_merges_once_github_passes_the_head(self):
        self.assertEqual(self.land(behind=True, gap_has_code=False),
                         ["look", f"reviews {SHA[:7]}", f"ticket check {SHA[:7]}", f"wait for {SHA[:7]}", MERGE])

    def test_a_head_that_contains_main_pushes_nothing_and_merges_once_github_passes_it(self):
        self.assertEqual(self.land(behind=False, gap_has_code=False),
                         ["look", f"reviews {SHA[:7]}", f"ticket check {SHA[:7]}", f"wait for {SHA[:7]}", MERGE])

    def test_a_head_github_lacks_is_pushed_first(self):
        self.assertEqual(self.land(behind=False, gap_has_code=False, on_github=False),
                         ["look", f"reviews {SHA[:7]}", f"ticket check {SHA[:7]}", f"push {SHA[:7]}", f"wait for {SHA[:7]}", MERGE])

    def test_a_conflict_pushes_and_merges_nothing(self):
        self.assertEqual(self.land(behind=True, gap_has_code=True, conflict=True),
                         ["look", f"reviews {SHA[:7]}", "merge main in", "refused"])

    def test_the_checklist_is_judged_on_the_head_with_main_merged_in_before_any_push(self):
        # A step whose script landed on main must not refuse a branch behind
        # main: the check reads the head as it will be merged.
        self.assertEqual(self.land(behind=True, gap_has_code=True, ticket_refused=True),
                         ["look", f"reviews {SHA[:7]}", "merge main in", f"ticket check {MERGED[:7]}", "refused"])

    def test_a_head_without_recorded_reviews_is_refused_before_main_is_merged_in(self):
        self.assertEqual(self.land(behind=True, gap_has_code=True, unreviewed=True),
                         ["look", f"reviews {SHA[:7]}", "refused"])

    def test_a_checklist_refusal_after_merging_main_says_the_merge_stays_unpushed(self):
        self.land(behind=True, gap_has_code=True, ticket_refused=True)
        self.assertIn("The merge of origin/main made above stays in work, unpushed", self.refusal)
        self.land(behind=False, gap_has_code=False, ticket_refused=True)
        self.assertNotIn("stays in", self.refusal)

    def test_it_merges_the_commit_it_pushed_not_the_one_it_started_with(self):
        steps = self.land(behind=True, gap_has_code=True)
        self.assertIn(f"wait for {MERGED[:7]}", steps)
        self.assertNotIn(f"wait for {SHA[:7]}", steps)


class TakingTheLock(unittest.TestCase):
    """`land` takes the landing lock before it looks at main, lets it go
    whatever happens, and clears target/ only after a merge."""

    def land(self, inside, keep_target: bool = False) -> tuple[list[str], int | None, Exception | None]:
        steps: list[str] = []
        pr = {"number": 42, "state": "OPEN", "isDraft": False, "headRefName": "work", "baseRefName": "main"}
        lock = mock.Mock()
        lock.close.side_effect = lambda: steps.append("let go")
        with (
            mock.patch.object(merge, "pull_request", return_value=pr),
            mock.patch.object(land, "require_here"),
            mock.patch.object(ci_local, "require_clean_head", return_value=SHA),
            mock.patch.object(ci_local, "stop_on_sigterm"),
            mock.patch.object(merge, "require_reviewed", lambda pr: steps.append("reviews")),
            mock.patch.object(merge, "take_landing_turn", lambda: steps.append("lock") or lock),
            mock.patch.object(land, "holding_the_lane", inside(steps)),
            mock.patch.object(land, "target_directory", return_value=Path("/work/here/target")),
            mock.patch.object(land, "clear_target", lambda target: steps.append(f"clear {target}")),
        ):
            try:
                return steps, land.land(42, keep_target), None
            except Exception as error:  # noqa: BLE001 - the test reports what escaped
                return steps, None, error

    def test_it_takes_the_lock_before_it_looks_at_main_and_clears_target_after_the_merge(self):
        steps, code, _ = self.land(lambda steps: lambda number: steps.append("look at main") or 0)
        self.assertEqual((steps, code), (["reviews", "lock", "look at main", "let go", "clear /work/here/target"], 0))

    def test_a_merge_whose_branch_remains_still_clears_target(self):
        steps, code, _ = self.land(lambda steps: lambda number: land.EXIT_BRANCH_REMAINS)
        self.assertEqual((steps[-1], code), ("clear /work/here/target", land.EXIT_BRANCH_REMAINS))

    def test_keep_target_keeps_it(self):
        steps, code, _ = self.land(lambda steps: lambda number: 0, keep_target=True)
        self.assertEqual((steps, code), (["reviews", "lock", "let go"], 0))

    def test_a_refusal_lets_the_lock_go_and_clears_nothing(self):
        def refuses(steps):
            def inside(number):
                steps.append("look at main")
                raise merge.Refused("no")
            return inside

        steps, _, error = self.land(refuses)
        self.assertIsInstance(error, merge.Refused)
        self.assertEqual(steps, ["reviews", "lock", "look at main", "let go"])


class HoldingTheLock(unittest.TestCase):
    """The lock is really held, with the real `take_landing_turn`, from before
    the first look at main to the end of the landing, and let go after it."""

    def test_another_landing_cannot_take_the_lock_while_one_is_inside(self):
        with tempfile.TemporaryDirectory() as directory:
            lock = Path(directory) / "landing-lock"
            seen: list[str] = []

            def inside(number):
                other = open(lock, "w")
                try:
                    fcntl.flock(other, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    seen.append("free")
                except BlockingIOError:
                    seen.append("held")
                finally:
                    other.close()
                return 1

            pr = {"number": 42, "state": "OPEN", "isDraft": False, "headRefName": "work", "baseRefName": "main"}
            with (
                mock.patch.object(merge, "LANDING_LOCK", lock),
                mock.patch.object(merge, "pull_request", return_value=pr),
                mock.patch.object(land, "require_here"),
                mock.patch.object(ci_local, "require_clean_head", return_value=SHA),
                mock.patch.object(ci_local, "stop_on_sigterm"),
                mock.patch.object(merge, "require_reviewed"),
                mock.patch.object(land, "holding_the_lane", inside),
            ):
                land.land(42, keep_target=False)
            after = open(lock, "w")
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


class ClearingTarget(unittest.TestCase):
    """After a merge, the worktree's target/ goes, and only it (#277)."""

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.worktree = Path(directory.name) / "worktree"
        self.target = self.worktree / "target"
        (self.target / "debug").mkdir(parents=True)
        (self.target / "debug" / "app").write_bytes(b"x" * 2_000_000)
        (self.worktree / "src").mkdir()
        (self.worktree / "src" / "main.rs").write_text("fn main() {}\n")

    def clear(self) -> tuple[str, str]:
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            land.clear_target(self.target)
        return out.getvalue(), err.getvalue()

    def test_it_removes_target_says_what_it_freed_and_leaves_the_rest(self):
        out, _ = self.clear()
        self.assertFalse(self.target.exists())
        self.assertTrue((self.worktree / "src" / "main.rs").exists())
        self.assertEqual(out, "Removed target/ (2 MB).\n")

    def test_a_target_reached_through_a_link_is_not_followed(self):
        elsewhere = self.worktree.parent / "shared-target"
        self.target.rename(elsewhere)
        self.target.symlink_to(elsewhere)
        self.clear()
        self.assertTrue((elsewhere / "debug" / "app").exists())

    def test_a_failure_is_said_and_raises_nothing(self):
        with mock.patch.object(land.shutil, "rmtree", side_effect=PermissionError("denied")):
            _, err = self.clear()
        self.assertIn("Merged, but removing", err)
        self.assertIn("Next: cargo clean", err)

    def test_no_target_is_said_and_is_fine(self):
        land.shutil.rmtree(self.target)
        out, _ = self.clear()
        self.assertIn("No target/ to remove", out)

    def test_the_target_is_the_worktrees_own(self):
        with mock.patch.object(ci_local, "worktree", return_value="/work/here"):
            self.assertEqual(land.target_directory(), Path("/work/here/target"))


if __name__ == "__main__":
    unittest.main()
