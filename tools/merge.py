#!/usr/bin/env python3
"""Squash-merge a pull request, but only once it is safe for main.

Usage (through just):
  just merge 42              # wait for checks (up to 30 minutes), then merge
  just merge 42 --wait 0     # don't wait: refuse if a check is still pending
  just merge 42 --wait 60    # wait up to 60 minutes

GitHub cannot enforce green checks on this private Free repository (branch
protection and rulesets return 403), so this is the only way to merge. It
refuses unless all of these hold, in this order:

  1. The PR is open.
  2. The PR head already contains the tip of origin/<base>, so the CI that
     passed tested exactly what <base> will become. One exception: commits
     the head lacks may change docs/ and *.md files (the wiki and glossary
     move often), unless code, tests or CI name one of those files, as the
     scorecard test names docs/wiki/polish-scorecard.md. It prints one line
     naming the commits it let through.
  3. Every check on the PR head has passed. Pending checks are waited for
     with `gh pr checks --watch`; a check that is still pending when the
     wait ends counts as not passed.
  4. The head did not move while waiting, and still contains origin/<base>
     (with the same exception).
  5. GitHub reports the PR as mergeable, and it is not a draft.

Then it squash-merges the head it checked (`--match-head-commit`) and
deletes the PR's branch on GitHub. Local branches and worktrees are left
alone. Each refusal names the condition that failed and the next command.

Exit status: 0 merged and branch deleted; 1 refused, nothing merged;
2 merged, but the branch could not be deleted.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time

EXIT_MERGED, EXIT_REFUSED, EXIT_BRANCH_REMAINS = 0, 1, 2

# How long CI may take to report its first check after a push.
CHECKS_APPEAR_SECONDS = 180
# How often to ask GitHub again while waiting.
POLL_SECONDS = 10
# How long GitHub may take to work out whether a PR is mergeable.
MERGEABLE_SECONDS = 60


class Refused(Exception):
    """A condition for merging does not hold. The message says which, and
    what to run next."""


# Missing commits the head may lack: those changing only files here.
DOCS_DIRECTORY = "docs/"
DOCS_SUFFIX = ".md"
# How a file names a docs path it may read: docs/... or ....md.
DOCS_REFERENCE = (r"docs/[A-Za-z0-9_.-][A-Za-z0-9_./-]*", r"[A-Za-z0-9_./-]+\.md")
# The compare API lists at most this many files and commits.
COMPARE_FILES_LIMIT, COMPARE_COMMITS_LIMIT = 300, 250


def gh(*args: str) -> str:
    """Runs gh and returns its standard output. Raises Refused if it fails."""
    result = subprocess.run(["gh", *args], capture_output=True, text=True)
    if result.returncode != 0:
        raise Refused(
            f"`gh {' '.join(args)}` failed: {result.stderr.strip()}\n"
            "Next: check `gh auth status`, then run this again."
        )
    return result.stdout


def git(*args: str, allowed: tuple[int, ...] = (0,)) -> str:
    """Runs git and returns its standard output. Raises Refused if it fails."""
    result = subprocess.run(["git", *args], capture_output=True, text=True)
    if result.returncode not in allowed:
        raise Refused(
            f"`git {' '.join(args)}` failed: {result.stderr.strip()}\n"
            "Next: run `just merge` from a clone of this repository, then run this again."
        )
    return result.stdout


def pull_request(number: int) -> dict:
    fields = "number,url,state,isDraft,headRefName,headRefOid,baseRefName,mergeable"
    return json.loads(gh("pr", "view", str(number), "--json", fields))


def require_open(pr: dict) -> None:
    if pr["state"] != "OPEN":
        raise Refused(f"PR #{pr['number']} is {pr['state'].lower()}, so there is nothing to merge.")


def is_docs(path: str) -> bool:
    return path.startswith(DOCS_DIRECTORY) or path.endswith(DOCS_SUFFIX)


def docs_named_by_code(pr: dict, paths: list[str]) -> dict[str, str]:
    """Which of `paths` a file outside docs/ and *.md names, in the base
    branch or the PR head: maps each to the first file naming it. A path
    counts as named if it, or a directory holding it, appears. Comments count
    too, which can only refuse a merge that was safe."""
    base, number = pr["baseRefName"], pr["number"]
    git("fetch", "--quiet", "origin", base, f"refs/pull/{number}/head")
    trees = [git("rev-parse", f"origin/{base}").strip(), pr["headRefOid"]]
    patterns = [argument for pattern in DOCS_REFERENCE for argument in ("-e", pattern)]
    names: dict[str, str] = {}
    for tree in trees:
        # git grep exits 1 when nothing matches.
        output = git(
            "grep", "-o", "-I", "-E", *patterns, tree, "--",
            ".", ":!docs", ":!*.md", ":!tools/merge.py",
            allowed=(0, 1),
        )
        for line in output.splitlines():
            _, file, name = line.split(":", 2)
            start = name.find(DOCS_DIRECTORY)
            name = name[start:] if start >= 0 else name.lstrip("./")
            names.setdefault(name.rstrip("./"), file)
    return {
        path: file
        for path in paths
        for name, file in names.items()
        if path == name or path.startswith(name + "/")
    }


def require_contains_base(pr: dict) -> str | None:
    """The head must contain the tip of the base branch on GitHub, but for
    commits that change only docs no code, test or CI names. Returns the line
    naming any such commits it let through."""
    number, base, head = pr["number"], pr["baseRefName"], pr["headRefOid"]
    missing = json.loads(
        gh(
            "api",
            f"repos/{{owner}}/{{repo}}/compare/{head}...{base}",
            "--jq",
            "{behind: .ahead_by,"
            " commits: [.commits[] | {sha, subject: (.commit.message | split(\"\\n\")[0])}],"
            " files: [.files[] | .filename, (.previous_filename // empty)]}",
        )
    )
    behind, commits, files = missing["behind"], missing["commits"], missing["files"]
    if not behind:
        return None
    noun, change = ("commit", "changes") if behind == 1 else ("commits", "change")
    next_step = (
        f"Next: gh pr update-branch {number}   (then `just merge {number}` "
        "waits for the new checks)"
    )
    listed = len(commits) == behind and len(commits) < COMPARE_COMMITS_LIMIT
    listed = listed and len(files) < COMPARE_FILES_LIMIT
    code = [path for path in files if not is_docs(path)]
    if not listed:
        raise Refused(
            f"PR #{number} is {behind} {noun} behind origin/{base}, so its CI did not test "
            f"what {base} will become.\n{next_step}"
        )
    if code:
        raise Refused(
            f"PR #{number} lacks {behind} {noun} on origin/{base} that {change} "
            f"{examples(code)}, so its CI did not test what {base} will become.\n{next_step}"
        )
    named = docs_named_by_code(pr, files)
    if named:
        path, file = next(iter(named.items()))
        others = f" (and {len(named) - 1} more)" if len(named) > 1 else ""
        raise Refused(
            f"PR #{number} lacks {behind} {noun} on origin/{base} that {change} {path}"
            f"{others}, which {file} names, so its CI did not test what {base} will "
            f"become.\n{next_step}"
        )
    names = "; ".join(f"{commit['sha'][:7]} {commit['subject']}" for commit in commits)
    return (
        f"Letting through {behind} docs-only {noun} on origin/{base} that PR #{number} lacks: "
        f"{names}"
    )


def examples(paths: list[str]) -> str:
    """The first path, and how many more."""
    more = len(paths) - 1
    return paths[0] + (f" and {more} other file{'s' if more > 1 else ''}" if more else "")


def checks(pr: dict) -> list[dict]:
    """The checks on the PR head. A workflow run that is still queued has no
    check yet, so it appears as a pending check marked `queued`. Empty if CI
    has reported nothing for the head."""
    number = str(pr["number"])
    # gh exits 8 while checks are pending, 1 if one failed or none exist, so
    # read its output whatever the exit status.
    result = subprocess.run(
        ["gh", "pr", "checks", number, "--json", "name,event,bucket,state,link"],
        capture_output=True,
        text=True,
    )
    if result.stdout.strip():
        return json.loads(result.stdout)
    if "no checks reported" not in result.stderr:
        raise Refused(
            f"`gh pr checks {number}` failed: {result.stderr.strip()}\n"
            "Next: check `gh auth status`, then run this again."
        )
    runs = json.loads(
        gh("run", "list", "--commit", pr["headRefOid"], "--json", "name,event,status,url")
    )
    return [
        {
            "name": run["name"],
            "event": run["event"],
            "bucket": "pending",
            "state": run["status"],
            "link": run["url"],
            "queued": True,
        }
        for run in runs
        if run["status"] != "completed"
    ]


def label(check: dict) -> str:
    return f"{check['name']} ({check['event']})" if check.get("event") else check["name"]


def wait_for_checks(pr: dict, wait_seconds: float) -> None:
    """Waits until every check has finished, or the wait ends, or CI has
    reported nothing for CHECKS_APPEAR_SECONDS."""
    number = str(pr["number"])
    started = time.monotonic()
    deadline = started + wait_seconds
    announced = False
    while time.monotonic() < deadline:
        current = checks(pr)
        if current and all(check["bucket"] != "pending" for check in current):
            return
        if current and not any(check.get("queued") for check in current):
            remaining = deadline - time.monotonic()
            print(f"Waiting up to {remaining / 60:.0f} min for checks on PR #{number}:", flush=True)
            try:
                subprocess.run(
                    ["gh", "pr", "checks", number, "--watch", "--interval", str(POLL_SECONDS)],
                    timeout=remaining,
                    # On a terminal, gh redraws the table in place. Anywhere
                    # else it would print the whole table every interval.
                    stdout=sys.stderr if sys.stderr.isatty() else subprocess.DEVNULL,
                )
            except subprocess.TimeoutExpired:
                pass
            return
        if not current and time.monotonic() - started > CHECKS_APPEAR_SECONDS:
            return
        if not announced:
            print(f"Waiting for CI to start on {pr['headRefOid'][:7]}...", flush=True)
            announced = True
        time.sleep(POLL_SECONDS)


def require_checks_passed(pr: dict, wait_minutes: float) -> None:
    number = pr["number"]
    found = checks(pr)
    if not found:
        raise Refused(
            f"No checks are reported on PR #{number}'s head ({pr['headRefOid'][:7]}), so "
            "nothing shows it is green. CI may not have started.\n"
            f"Next: gh run list --branch {pr['headRefName']}   (then `just merge {number}`)"
        )
    failed = [check for check in found if check["bucket"] in ("fail", "cancel")]
    pending = [check for check in found if check["bucket"] == "pending"]
    other = [check for check in found if check["bucket"] not in ("pass", "fail", "cancel", "pending")]
    if failed:
        lines = "\n".join(f"  {label(c)}: {c['state'].lower()}  {c['link']}" for c in failed)
        runs = [m.group(1) for c in failed if (m := re.search(r"/runs/(\d+)", c["link"] or ""))]
        next_step = (
            f"gh run view {runs[0]} --log-failed" if runs else f"gh pr checks {number}"
        )
        raise Refused(
            f"{len(failed)} of {len(found)} checks on PR #{number} did not pass:\n{lines}\n"
            f"Next: {next_step}"
        )
    if pending:
        waited = "without waiting" if wait_minutes == 0 else f"after waiting {wait_minutes:g} min"
        lines = "\n".join(f"  {label(c)}: {c['state'].lower()}" for c in pending)
        raise Refused(
            f"{len(pending)} of {len(found)} checks on PR #{number} {'is' if len(pending) == 1 else 'are'} still pending {waited}:\n"
            f"{lines}\n"
            f"Next: just merge {number}   (it waits for them)"
        )
    if other:
        lines = "\n".join(f"  {label(c)}: {c['state'].lower()}" for c in other)
        raise Refused(
            f"{len(other)} of {len(found)} checks on PR #{number} did not pass:\n{lines}\n"
            f"Next: gh pr checks {number}"
        )


def require_mergeable(pr: dict) -> dict:
    """Returns the PR as GitHub reports it once it knows whether it can merge."""
    number = pr["number"]
    give_up = time.monotonic() + MERGEABLE_SECONDS
    while pr["mergeable"] == "UNKNOWN" and time.monotonic() < give_up:
        time.sleep(POLL_SECONDS / 2)
        pr = pull_request(number)
    if pr["mergeable"] == "CONFLICTING":
        raise Refused(
            f"GitHub reports PR #{number} conflicts with origin/{pr['baseRefName']}.\n"
            f"Next: git switch {pr['headRefName']} && git merge origin/{pr['baseRefName']}   "
            "(resolve, push, then `just merge` again)"
        )
    if pr["mergeable"] != "MERGEABLE":
        raise Refused(
            f"GitHub has not worked out whether PR #{number} can merge "
            f"(mergeable: {pr['mergeable'].lower()}).\n"
            f"Next: just merge {number}   (in a minute)"
        )
    if pr["isDraft"]:
        raise Refused(
            f"PR #{number} is a draft.\nNext: gh pr ready {number}   (then `just merge {number}`)"
        )
    return pr


def merge(number: int, wait_minutes: float) -> int:
    pr = pull_request(number)
    require_open(pr)
    allowed = require_contains_base(pr)
    if allowed:
        print(allowed, flush=True)
    try:
        wait_for_checks(pr, wait_minutes * 60)
    except KeyboardInterrupt:
        raise Refused(
            f"Stopped waiting for PR #{number}'s checks.\nNext: just merge {number}"
        ) from None
    require_checks_passed(pr, wait_minutes)

    now = pull_request(number)
    require_open(now)
    if now["headRefOid"] != pr["headRefOid"]:
        raise Refused(
            f"PR #{number}'s head moved from {pr['headRefOid'][:7]} to {now['headRefOid'][:7]} "
            "while its checks ran, so they did not test the new head.\n"
            f"Next: just merge {number}"
        )
    allowed_now = require_contains_base(now)
    if allowed_now and allowed_now != allowed:
        print(allowed_now, flush=True)
    now = require_mergeable(now)

    head, branch = now["headRefOid"], now["headRefName"]
    gh("pr", "merge", str(number), "--squash", "--match-head-commit", head)
    merged = json.loads(gh("pr", "view", str(number), "--json", "mergeCommit"))["mergeCommit"]
    merged_as = merged["oid"][:7] if merged else "a squash commit"
    try:
        gh("api", "--method", "DELETE", f"repos/{{owner}}/{{repo}}/git/refs/heads/{branch}")
    except Refused as error:
        print(f"Merged PR #{number} as {merged_as}, but its branch {branch} remains:", file=sys.stderr)
        print(str(error).splitlines()[0], file=sys.stderr)
        print(f"Next: git push origin --delete {branch}", file=sys.stderr)
        return EXIT_BRANCH_REMAINS
    print(f"Merged PR #{number} into {now['baseRefName']} as {merged_as}, and deleted {branch}.")
    print(f"Next: git switch {now['baseRefName']} && git pull --ff-only")
    return EXIT_MERGED


def main() -> int:
    parser = argparse.ArgumentParser(
        prog="just merge",
        description="Squash-merge a PR only if its checks passed on a head that contains main"
        " (or lacks only docs-only commits).",
    )
    parser.add_argument("pr", type=int, help="the pull request number")
    parser.add_argument(
        "--wait",
        type=float,
        default=30,
        metavar="MINUTES",
        help="how long to wait for pending checks (default 30; 0 refuses at once)",
    )
    arguments = parser.parse_args()
    try:
        return merge(arguments.pr, arguments.wait)
    except Refused as refusal:
        print(f"Not merged: {refusal}", file=sys.stderr)
        return EXIT_REFUSED


if __name__ == "__main__":
    sys.exit(main())
