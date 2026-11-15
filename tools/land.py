#!/usr/bin/env python3
"""Land a pull request: bring it up to date, let GitHub's CI test the result,
merge it, and clear the worktree's build output (tickets #247, #286, #277).

Usage (through just), from the worktree that has the PR's branch checked out:
  just land 42                # do whatever is needed, then squash-merge
  just land 42 --plan         # say what it would do; change nothing, take no lock
  just land 42 --keep-target  # keep target/ after the merge, for more work here

GitHub runs CI on every push to a PR, and its "CI passed" check on the PR's
head is what branch protection requires (#286). Landings take turns on one
lock (tools/merge.py's), held from the first look at `main` to the squash, so
`main` can't move between the run that tested a head and its merge.

What it does, holding the lock:
  1. Brings the branch level with its copy on GitHub, fast-forward only. A
     branch that has diverged from it is refused: someone rebased, or pushed.
  2. If origin/main has commits the head lacks that touch code (or docs that
     code names, tools/changes.py), merges origin/main into the branch. On a
     conflict it aborts the merge, changes nothing, and names the command to
     run next. A gap of docs alone is let through, as `just merge` always has.
  3. Pushes the branch if GitHub lacks the head (a fast-forward; never a force).
  4. The checklist `lands_with` check (tools/checklist_tickets.py) runs on the
     head as it will be merged, after step 2 and before step 3, so a script
     that landed on main counts, and a PR it refuses waits for no CI run. A
     merge of main it made stays in the branch, unpushed.
  5. Squash-merges through the checks `just merge` makes (tools/merge.py),
     which wait for GitHub's run on the head and refuse a red or missing
     "CI passed", with the head pinned (`--match-head-commit`).
  6. After a merge, removes the worktree's target/ and says how much it freed
     (#277), unless --keep-target. Nothing else: not the shared cache under
     ~/.cache/desktop-calls-ci, not another worktree, not the worktree itself.
     A failure here is reported, and the merge still counts.

Exit status: 0 merged and branch deleted; 1 refused or CI failed, nothing
merged; 2 merged, but the branch could not be deleted.
"""

from __future__ import annotations

import argparse
import dataclasses
import os
import pathlib
import shutil
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
# How long to wait for GitHub's run on the head. A warm run takes about 5
# minutes; a cold one, which builds CI's image, longer.
CI_WAIT_MINUTES = 30


def git(*args: str, allowed: tuple[int, ...] = (0,)) -> str:
    return merge.git(*args, allowed=allowed)


def is_ancestor(older: str, newer: str) -> bool:
    """Whether commit `older` is `newer` or lies in its history."""
    result = subprocess.run(["git", "merge-base", "--is-ancestor", older, newer], capture_output=True, text=True)
    if result.returncode not in (0, 1):
        raise merge.Refused(f"`git merge-base --is-ancestor {older} {newer}` failed: {result.stderr.strip()}")
    return result.returncode == 0


def gap_changes_code(base: str) -> tuple[bool, str]:
    """Whether what `base` has that HEAD lacks touches code, or docs that code
    names: the rule `just merge` applies to a head behind its base. Says why."""
    try:
        return changes.code_changes(changes.run_git, "HEAD", base)
    except RuntimeError as error:
        return True, f"Couldn't compare HEAD with {base}: {error}"


def level_with_github(branch: str) -> None:
    """Fast-forwards HEAD to the branch's copy on GitHub if that is ahead. A
    HEAD ahead of it is left, to be pushed. A diverged one is refused, because
    a merge would make a commit nobody asked for."""
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
    try:
        result = subprocess.run(["git", "merge", "--no-edit", f"origin/{base}"], capture_output=True, text=True)
    except KeyboardInterrupt:
        # Ctrl-C or SIGTERM (stop_on_sigterm) mid-merge: don't leave the worktree merging.
        subprocess.run(["git", "merge", "--abort"], capture_output=True, text=True)
        raise
    if result.returncode == 0:
        return
    conflicted = git("diff", "--name-only", "--diff-filter=U", allowed=(0, 1)).split()
    subprocess.run(["git", "merge", "--abort"], capture_output=True, text=True)
    names = ", ".join(conflicted[:5]) + (f" and {len(conflicted) - 5} more" if len(conflicted) > 5 else "")
    raise merge.Refused(
        f"Merging origin/{base} into the branch conflicts"
        f"{f' in {names}' if conflicted else f' ({result.stderr.strip() or result.stdout.strip()})'}. "
        "The merge was aborted; nothing was pushed or merged.\n"
        f"Next: git merge --no-edit origin/{base}   (resolve the conflicts, commit, then `just land {number}`)"
    )


def push(branch: str, sha: str) -> None:
    """Pushes `sha` to the branch on GitHub: a fast-forward, never a force."""
    result = subprocess.run(["git", "push", "origin", f"{sha}:refs/heads/{branch}"], capture_output=True, text=True)
    if result.returncode != 0:
        raise merge.Refused(
            f"Pushing {sha[:7]} to {branch} failed: {(result.stderr or result.stdout).strip()}\n"
            "Next: git fetch origin, merge what GitHub has (never rebase a pushed branch), then `just land <n>`"
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
                f"PR #{number}'s head on GitHub is {shown[:7]}, not the {sha[:7]} pushed, "
                f"after {HEAD_SHOWS_SECONDS} s.\n"
                f"Next: git push origin HEAD, then `just land {number}`"
            )
        time.sleep(POLL_SECONDS)


def fetch(base: str, branch: str) -> None:
    git("fetch", "--quiet", "origin", base, branch)


@dataclasses.dataclass
class Survey:
    """What a landing faces: HEAD, whether it lacks commits on the base, and
    whether those touch code, and why."""

    branch: str
    base: str
    sha: str
    behind: bool
    gap_has_code: bool
    why: str

    @property
    def must_update(self) -> bool:
        return self.behind and self.gap_has_code


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
    return Survey(branch, base, sha, behind, gap_has_code, why)


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


def target_directory() -> pathlib.Path:
    """This worktree's build output."""
    return pathlib.Path(ci_local.worktree()) / "target"


def size_of(path: pathlib.Path) -> int:
    """The bytes the files under `path` take, not following links."""
    total = 0
    for directory, _, files in os.walk(path):
        for name in files:
            try:
                total += os.lstat(os.path.join(directory, name)).st_size
            except OSError:
                continue
    return total


def clear_target(target: pathlib.Path) -> None:
    """Removes `target` and says how much it freed. A failure is said and
    goes no further: the merge it follows has happened (#277)."""
    if not target.is_dir() or target.is_symlink():
        print(f"No {target.name}/ to remove in {target.parent}.")
        return
    freed = size_of(target)
    try:
        shutil.rmtree(target)
    except OSError as error:
        print(f"Merged, but removing {target} failed: {error}\nNext: cargo clean", file=sys.stderr)
        return
    amount = f"{freed / 1e9:.1f} GB" if freed >= 1e9 else f"{freed / 1e6:.0f} MB"
    print(f"Removed {target.name}/ ({amount}).")


def plan(number: int, keep_target: bool) -> int:
    """Says what landing would do, from what is true now. It fetches, and
    changes no branch and takes no lock."""
    pr = merge.pull_request(number)
    merge.require_open(pr)
    require_here(pr)
    ci_local.require_clean_head()
    look = survey(pr, level=False)
    gap = f"origin/{look.base} has commits the head lacks ({look.why})" if look.behind else f"it contains origin/{look.base}"
    print(f"PR #{number}: {look.branch} at {look.sha[:7]}; {gap}.")
    first = (
        f"Would merge origin/{look.base} into the branch and push it, then wait for GitHub's \"CI passed\" on "
        "that merge"
        if look.must_update
        else "Would wait for GitHub's \"CI passed\" on the head"
    )
    print(f"{first}, then squash-merge.")
    print(
        "Would keep target/ (--keep-target)."
        if keep_target
        else f"Would then remove {target_directory()} (--keep-target keeps it)."
    )
    return EXIT_MERGED


def holding_the_lane(number: int) -> int:
    """Everything from the first look at `main` to the squash. The caller holds
    the landing lock, and nothing else lands until it lets go."""
    pr = merge.pull_request(number)
    merge.require_open(pr)
    require_here(pr)
    ci_local.require_clean_head()  # HEAD may have moved while this waited for its turn
    look = survey(pr, level=True)
    sha, base = look.sha, look.base
    if look.must_update:
        print(f"{sha[:7]} lacks commits on origin/{base} ({look.why}): merging origin/{base} into "
              f"{look.branch}.", flush=True)
        update_from_base(base, number)
        sha = git("rev-parse", "HEAD").strip()
    elif look.behind:
        print(f"Letting through the commits on origin/{base} it lacks: {look.why}", flush=True)
    # On the head as it will be merged: a step whose script is on main only
    # must not refuse a branch behind main.
    merge.require_no_step_waits_on_a_closed_ticket(pr, head=sha)
    if not is_ancestor(sha, f"origin/{look.branch}"):
        print(f"Pushing {sha[:7]} to {look.branch}; GitHub's CI runs on it.", flush=True)
        push(look.branch, sha)
    wait_for_head_on_github(number, sha)
    return merge.merge(number, CI_WAIT_MINUTES, lane_held=True)


def land(number: int, keep_target: bool) -> int:
    pr = merge.pull_request(number)
    merge.require_open(pr)
    require_here(pr)
    ci_local.require_clean_head()
    ci_local.stop_on_sigterm()
    lane = merge.take_landing_turn()
    try:
        result = holding_the_lane(number)
    finally:
        lane.close()
    if result in (EXIT_MERGED, EXIT_BRANCH_REMAINS) and not keep_target:
        clear_target(target_directory())
    return result


def main() -> int:
    parser = argparse.ArgumentParser(
        prog="just land",
        description="Bring a PR up to date with main, wait for GitHub's CI on it and squash-merge it, "
        "taking turns with other landings so nothing lands in between; then remove target/.",
    )
    parser.add_argument("pr", type=int, help="the pull request number")
    parser.add_argument("--plan", action="store_true", help="say what it would do and change nothing")
    parser.add_argument("--keep-target", action="store_true",
                        help="keep this worktree's target/ after the merge, for more work here")
    arguments = parser.parse_args()
    try:
        if arguments.plan:
            return plan(arguments.pr, arguments.keep_target)
        return land(arguments.pr, arguments.keep_target)
    except (merge.Refused, ci_local.Refused) as refusal:
        print(f"Not merged: {refusal}", file=sys.stderr)
        return EXIT_REFUSED
    except KeyboardInterrupt:
        print("Interrupted; nothing was merged.", file=sys.stderr)
        return EXIT_REFUSED


if __name__ == "__main__":
    sys.exit(main())
