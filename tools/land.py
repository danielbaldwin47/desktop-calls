#!/usr/bin/env python3
"""Land a pull request: bring it up to date, test the result and merge it, all
while holding the CI lane, so `main` can't move in between (ticket #247).

Usage (through just), from the worktree that has the PR's branch checked out:
  just land 42            # do whatever is needed, then squash-merge
  just land 42 --plan     # say what it would do; change nothing, take no lock

Before this, `just merge` refused a head that lacked commits on origin/main,
and the agent merged origin/main, queued a full `just ci-local` run (204 to
262 s plus the queue) and tried again, while another PR landed and it was
refused again. Here the lane's lock (the one `just ci-local` takes) is held
from the first look at `main` to the squash, and every merge to `main` takes
it for its final recheck (tools/merge.py), so nothing lands in between.

What it does, holding the lock:
  1. Brings the branch level with its copy on GitHub, fast-forward only. A
     branch that has diverged from it is refused: someone rebased, or pushed.
  2. Looks at the head's "CI passed" status and at what origin/main has that
     the head lacks, and does the least that is safe:
       - "CI passed" on the head, and the head contains origin/main or lacks
         only docs-only commits that no code names (the rule `just merge`
         has always had, tools/changes.py): nothing to test, so straight to 4.
       - "CI passed" failed on the head, and nothing new to test: refused. A
         red run is fixed, committed and run again, never rerun until green.
       - the head lacks other commits on origin/main: merges origin/main into
         the branch. On a conflict it aborts the merge, changes nothing, and
         names the command to run next.
       - no "CI passed" on the head yet, or a merge was made: runs what `just
         ci-local` runs on the result (the full run, or lint alone for a
         change to docs that no code names), pushes the branch and posts
         "CI passed" on that exact commit. A red run merges nothing.
  3. The checklist `lands_with` check (tools/checklist_tickets.py) runs first,
     before any test run, so a PR it would refuse costs no minutes.
  4. Squash-merges through the same checks `just merge` makes (tools/merge.py)
     with the head pinned (`--match-head-commit`), which is also what refuses
     a red or missing run: this tool never merges on one.

Exit status: 0 merged and branch deleted; 1 refused or a run failed, nothing
merged; 2 merged, but the branch could not be deleted.
"""

from __future__ import annotations

import argparse
import dataclasses
import subprocess
import sys
import time

import changes
import ci_local
import merge

EXIT_MERGED, EXIT_REFUSED, EXIT_BRANCH_REMAINS = merge.EXIT_MERGED, merge.EXIT_REFUSED, merge.EXIT_BRANCH_REMAINS

# How long GitHub may take to show a pushed head on the PR.
HEAD_SHOWS_SECONDS = 60
POLL_SECONDS = 2

# What to do with a head, from `decide`.
MERGE_NOW, UPDATE_AND_RUN, RUN, RED = "merge", "update-and-run", "run", "red"


def git(*args: str, allowed: tuple[int, ...] = (0,)) -> str:
    return merge.git(*args, allowed=allowed)


def is_ancestor(older: str, newer: str) -> bool:
    """Whether commit `older` is `newer` or lies in its history."""
    result = subprocess.run(["git", "merge-base", "--is-ancestor", older, newer], capture_output=True, text=True)
    if result.returncode not in (0, 1):
        raise merge.Refused(f"`git merge-base --is-ancestor {older} {newer}` failed: {result.stderr.strip()}")
    return result.returncode == 0


def ci_state(sha: str) -> str | None:
    """The newest "CI passed" status on `sha`, or None if there is none (or
    GitHub doesn't have the commit)."""
    result = subprocess.run(
        [
            "gh", "api", f"repos/{ci_local.REPO}/commits/{sha}/statuses",
            "--jq", f'[.[] | select(.context == "{ci_local.CONTEXT}")][0].state',
        ],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        if "Not Found" in result.stderr or "No commit found" in result.stderr:
            return None
        raise merge.Refused(
            f"Reading the status of {sha[:7]} failed: {result.stderr.strip()}\n"
            "Next: check `gh auth status`, then run this again."
        )
    state = result.stdout.strip()
    return None if state in ("", "null") else state


def gap_changes_code(base: str) -> tuple[bool, str]:
    """Whether what `base` has that HEAD lacks touches code, or docs that code
    names: the rule `just merge` applies to a head behind its base. Says why."""
    try:
        return changes.code_changes(changes.run_git, "HEAD", base)
    except RuntimeError as error:
        return True, f"Couldn't compare HEAD with {base}: {error}"


def decide(behind: bool, gap_has_code: bool, status: str | None) -> str:
    """What to do with a head: `behind` is whether it lacks commits on the
    base, `gap_has_code` whether those commits touch code (or docs that code
    names), `status` the "CI passed" state on it."""
    must_update = behind and gap_has_code
    if status in ("failure", "error") and not must_update:
        return RED
    if status == "success" and not must_update:
        return MERGE_NOW
    return UPDATE_AND_RUN if behind else RUN


def level_with_github(branch: str) -> None:
    """Fast-forwards HEAD to the branch's copy on GitHub if that is ahead. A
    HEAD ahead of it is left: `just ci-local` pushes it. A diverged one is
    refused, because a merge would make a commit nobody tested."""
    remote = f"origin/{branch}"
    if is_ancestor("HEAD", remote):
        git("merge", "--ff-only", remote)
    elif not is_ancestor(remote, "HEAD"):
        raise merge.Refused(
            f"{branch} here and {remote} have each commits the other lacks (one was rebased, or "
            "pushed from elsewhere), so landing it would test a merge nobody asked for.\n"
            f"Next: git merge --no-edit {remote}   (resolve and commit, then `just land <n>`; "
            "merge, never rebase a pushed branch)"
        )


def update_from_base(base: str, number: int) -> None:
    """Merges origin/<base> into HEAD, or aborts and says what to run."""
    result = subprocess.run(["git", "merge", "--no-edit", f"origin/{base}"], capture_output=True, text=True)
    if result.returncode == 0:
        return
    conflicted = git("diff", "--name-only", "--diff-filter=U", allowed=(0, 1)).split()
    subprocess.run(["git", "merge", "--abort"], capture_output=True, text=True)
    names = ", ".join(conflicted[:5]) + (f" and {len(conflicted) - 5} more" if len(conflicted) > 5 else "")
    raise merge.Refused(
        f"Merging origin/{base} into the branch conflicts"
        f"{f' in {names}' if conflicted else f' ({result.stderr.strip() or result.stdout.strip()})'}. "
        "The merge was aborted; nothing was run and nothing was posted.\n"
        f"Next: git merge --no-edit origin/{base}   (resolve the conflicts, commit, then `just land {number}`)"
    )


def wait_for_head_on_github(number: int, sha: str) -> None:
    """The PR shows the head a push made after a moment: merging before then
    would pin the old head."""
    give_up = time.monotonic() + HEAD_SHOWS_SECONDS
    while True:
        shown = merge.pull_request(number)["headRefOid"]
        if shown == sha:
            return
        if time.monotonic() >= give_up:
            raise merge.Refused(
                f"PR #{number}'s head on GitHub is {shown[:7]}, not the {sha[:7]} that was tested, "
                f"after {HEAD_SHOWS_SECONDS} s.\n"
                f"Next: git push origin HEAD, then `just land {number}`"
            )
        time.sleep(POLL_SECONDS)


def fetch(base: str, branch: str) -> None:
    git("fetch", "--quiet", "origin", base, branch)


@dataclasses.dataclass
class Survey:
    """What a landing faces: HEAD, whether it lacks commits on the base, why
    the gap matters, and the "CI passed" state on it."""

    branch: str
    base: str
    sha: str
    behind: bool
    gap_has_code: bool
    why: str
    status: str | None

    @property
    def action(self) -> str:
        return decide(self.behind, self.gap_has_code, self.status)


def survey(pr: dict, level: bool) -> Survey:
    """Fetches, then looks. `level` first fast-forwards HEAD to the branch on
    GitHub, which is the one change a look may make."""
    branch, base = pr["headRefName"], pr["baseRefName"]
    fetch(base, branch)
    if level:
        level_with_github(branch)
    sha = git("rev-parse", "HEAD").strip()
    behind = not is_ancestor(f"origin/{base}", "HEAD")
    gap_has_code, why = gap_changes_code(f"origin/{base}") if behind else (False, "")
    return Survey(branch, base, sha, behind, gap_has_code, why, ci_state(sha))


def require_here(pr: dict) -> None:
    """Landing happens in the worktree that has the branch, clean."""
    branch, number = pr["headRefName"], pr["number"]
    current = git("branch", "--show-current").strip()
    if current != branch:
        where = merge.checkout_of(branch)
        start = f"cd {where}" if where else f"git fetch origin && git switch {branch}"
        raise merge.Refused(
            f"PR #{number}'s branch is {branch}, and this worktree has {current or 'a detached HEAD'}.\n"
            f"Next: {start} && just land {number}"
        )
    if pr["baseRefName"] != "main":
        raise merge.Refused(
            f"PR #{number} targets {pr['baseRefName']}; `just land` lands PRs on main only.\n"
            f"Next: just merge {number}"
        )
    if pr["isDraft"]:
        raise merge.Refused(f"PR #{number} is a draft.\nNext: gh pr ready {number}   (then `just land {number}`)")


def plan(number: int) -> int:
    """Says what landing would do, from what is true now. It fetches, and
    changes no branch and takes no lock."""
    pr = merge.pull_request(number)
    merge.require_open(pr)
    require_here(pr)
    ci_local.require_clean_head()
    look = survey(pr, level=False)
    gap = f"origin/{look.base} has commits the head lacks ({look.why})" if look.behind else f"it contains origin/{look.base}"
    print(f"PR #{number}: {look.branch} at {look.sha[:7]}, \"CI passed\" {look.status or 'absent'}; {gap}.")
    print({
        MERGE_NOW: "Would merge now: nothing to test.",
        UPDATE_AND_RUN: f"Would merge origin/{look.base} into the branch, run CI on the result, post the status, then merge.",
        RUN: "Would run CI on the head, post the status, then merge.",
        RED: "Would refuse: the head's run failed. Fix it, commit, then `just ci-local`.",
    }[look.action])
    return EXIT_MERGED


def holding_the_lane(number: int) -> int:
    """Everything from the first look at `main` to the squash. The caller holds
    the lock, and nothing else lands until it lets go."""
    pr = merge.pull_request(number)
    merge.require_open(pr)
    require_here(pr)
    ci_local.require_clean_head()  # HEAD may have moved while this waited for its turn
    merge.require_no_step_waits_on_a_closed_ticket(pr)
    look = survey(pr, level=True)
    sha, base, action = look.sha, look.base, look.action
    if action == RED:
        raise merge.Refused(
            f"PR #{number}'s head {sha[:7]} has a failed \"CI passed\", and there is nothing new to test.\n"
            f"Next: fix it, commit, then `just ci-local`   (its log is named in the status: "
            f"gh api repos/{ci_local.REPO}/commits/{sha}/statuses), then `just land {number}`"
        )
    if action == MERGE_NOW:
        print(f"{sha[:7]} has \"CI passed\" and needs nothing from origin/{base}: merging.", flush=True)
        if look.behind:
            print(f"Letting through the commits on origin/{base} it lacks: {look.why}", flush=True)
    if action == UPDATE_AND_RUN:
        print(f"{sha[:7]} lacks commits on origin/{base} ({look.why}): merging origin/{base} into "
              f"{look.branch}.", flush=True)
        update_from_base(base, number)
        sha = git("rev-parse", "HEAD").strip()
    if action != MERGE_NOW:
        log = ci_local.Log(sha)
        merged = f"merged origin/{base}, " if action == UPDATE_AND_RUN else ""
        log.line(f"just land {number} on {sha[:7]}: {merged}running CI while holding the lane; "
                 f"log {log.display_path()}")
        ci_local.clear_stale_test_folders()
        ci_local.prune_old_runs()
        if ci_local.run_and_post(sha, list(ci_local.JOBS), True, False, False, log) != ci_local.EXIT_PASSED:
            raise merge.Refused(
                f"CI did not pass on {sha[:7]} (the failure is above, and in the log named there).\n"
                f"Next: fix it, commit, then `just land {number}`"
            )
    wait_for_head_on_github(number, sha)
    return merge.merge(number, 0, lane_held=True)


def land(number: int) -> int:
    pr = merge.pull_request(number)
    merge.require_open(pr)
    require_here(pr)
    ci_local.require_online()
    sha = ci_local.require_clean_head()
    ci_local.stop_on_sigterm()
    claim = ci_local.Claim(ci_local.worktree())
    try:
        claim.take(sha)
        lane = ci_local.take_turn()  # held until this function returns
        claim.write("running", sha)
        return holding_the_lane(number)
    finally:
        claim.release()


def main() -> int:
    parser = argparse.ArgumentParser(
        prog="just land",
        description="Bring a PR up to date with main, test the result and squash-merge it, "
        "holding the CI lane so nothing else lands in between.",
    )
    parser.add_argument("pr", type=int, help="the pull request number")
    parser.add_argument("--plan", action="store_true", help="say what it would do and change nothing")
    arguments = parser.parse_args()
    try:
        return plan(arguments.pr) if arguments.plan else land(arguments.pr)
    except (merge.Refused, ci_local.Refused) as refusal:
        print(f"Not merged: {refusal}", file=sys.stderr)
        return EXIT_REFUSED
    except KeyboardInterrupt:
        print("Interrupted; nothing was merged.", file=sys.stderr)
        return EXIT_REFUSED


if __name__ == "__main__":
    sys.exit(main())
