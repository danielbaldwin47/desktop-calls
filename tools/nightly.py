#!/usr/bin/env python3
"""The nightly run's three decisions (.github/workflows/nightly.yml): whether
tonight runs, whose scenario results the scorecard scores, and what a red
night files.

Since 2026-11-16 the Real-Phone Checklist's group 6 runs and the polish
scorecard run nightly against main's head, not on every push to main (the
owner: "Sure change those to nightly"). Nothing waits on them, so a red
night opens an issue rather than going unseen.

Usage (the workflow's steps; each prints `key=value` lines for
$GITHUB_OUTPUT on standard output, and what it decided on standard error):

  python3 tools/nightly.py decide         # EVENT, HEAD
  python3 tools/nightly.py results        # HEAD
  python3 tools/nightly.py report [--dry-run]   # RUN_ID, HEAD, SINCE, SINCE_RUN

decide: a scheduled night runs only if main has moved since the last green
nightly; a dispatched one always runs. Either way it names that green run,
for the report's list of commits.

results: waits for CI's run on main at HEAD to finish (it builds CI's image
when that changes, and its scenario shards keep their results), then names
the newest run on main, at HEAD or before it, that kept them. A docs-only
push keeps none, and changed nothing the scenarios run.

report: opens the issue "Nightly CI is red", labelled needs-triage, naming
the run, its failed jobs and the commits since the last green nightly; or,
while one is open, comments on it. A run is posted once, so a rerun of the
report posts nothing new. --dry-run posts nothing and prints what it would.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from collections.abc import Callable

WORKFLOW = "nightly.yml"
CI_WORKFLOW = "ci.yml"
TITLE = "Nightly CI is red"
LABEL = "needs-triage"
# The artifact each scenario shard of CI's run on main keeps; shard 1's
# stands for the set.
RESULTS_ARTIFACT = "scenario-results-1"
FAILED = {"failure", "timed_out", "cancelled", "startup_failure"}
# How long CI's run for a just-pushed head may take to be listed.
RUN_APPEARS_SECONDS = 120
POLL_SECONDS = 10


class Failed(Exception):
    pass


def gh(*args: str) -> str:
    result = subprocess.run(["gh", *args], capture_output=True, text=True)
    if result.returncode != 0:
        raise Failed(f"`gh {' '.join(args)}` failed: {result.stderr.strip()}")
    return result.stdout


def git(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], capture_output=True, text=True)


def say(line: str) -> None:
    print(line, file=sys.stderr)


# decide


def should_run(event: str, head: str, last_green: str | None) -> bool:
    """A dispatched night always runs; a scheduled one only when main has
    moved since the last green nightly, or there has been none."""
    return event != "schedule" or last_green != head


def last_green() -> dict | None:
    runs = json.loads(gh(
        "run", "list", "--workflow", WORKFLOW, "--branch", "main", "--status", "success",
        "--limit", "1", "--json", "databaseId,headSha,url",
    ))
    return runs[0] if runs else None


def decide(event: str, head: str) -> list[str]:
    green = last_green()
    sha = green["headSha"] if green else None
    run = should_run(event, head, sha)
    if not green:
        say("No green nightly yet: tonight runs.")
    elif run and sha == head:
        say(f"main is still at {head[:7]}, as at the last green nightly ({green['url']}); dispatched, so it runs.")
    elif run:
        say(f"main moved from {sha[:7]} (the last green nightly, {green['url']}) to {head[:7]}: tonight runs.")
    else:
        say(f"main is still at {head[:7]}, as at the last green nightly ({green['url']}): skipping tonight.")
    return [
        f"run={'true' if run else 'false'}",
        f"since={sha or ''}",
        f"since_run={green['url'] if green else ''}",
    ]


# results


def results_run(artifacts: list[dict], is_ancestor: Callable[[str], bool]) -> dict | None:
    """The newest unexpired run on main, at HEAD or before it, among the
    `scenario-results-1` artifacts (`GET /actions/artifacts`, whose order
    GitHub doesn't promise)."""
    for artifact in sorted(artifacts, key=lambda artifact: artifact.get("created_at", ""), reverse=True):
        run = artifact.get("workflow_run") or {}
        if artifact.get("expired") or run.get("head_branch") != "main":
            continue
        if is_ancestor(run["head_sha"]):
            return run
    return None


def wait_for_ci(head: str) -> None:
    """Waits for CI's run on main at `head` to finish, giving a push made
    moments ago time to get its run listed."""
    give_up = time.monotonic() + RUN_APPEARS_SECONDS
    while True:
        runs = json.loads(gh(
            "run", "list", "--workflow", CI_WORKFLOW, "--branch", "main", "--event", "push",
            "--commit", head, "--json", "databaseId,status",
        ))
        if runs or time.monotonic() >= give_up:
            break
        time.sleep(POLL_SECONDS)
    if not runs:
        say(f"CI has no run on main at {head[:7]}; scoring the newest results kept before it.")
    for found in runs:
        if found["status"] != "completed":
            say(f"Waiting for CI's run {found['databaseId']} on main at {head[:7]} to finish.")
            # Red or green, its results are kept; the scorecard scores both.
            subprocess.run(["gh", "run", "watch", str(found["databaseId"]), "--interval", "30"],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def results(head: str) -> list[str]:
    wait_for_ci(head)
    artifacts = json.loads(gh(
        "api", f"repos/{{owner}}/{{repo}}/actions/artifacts?name={RESULTS_ARTIFACT}&per_page=100",
    ))["artifacts"]
    run = results_run(
        artifacts, lambda sha: git("merge-base", "--is-ancestor", sha, head).returncode == 0
    )
    if run is None:
        raise Failed(
            f"No CI run on main at or before {head[:7]} kept its scenario results.\n"
            f"Next: gh workflow run CI --ref main, then run this nightly again"
        )
    say(f"Scoring the scenario results of CI's run {run['id']}, at {run['head_sha'][:7]}.")
    return [f"run={run['id']}", f"sha={run['head_sha']}"]


# report


def failed_jobs(jobs: list[dict]) -> list[dict]:
    return [job for job in jobs if job.get("conclusion") in FAILED]


def body(run_url: str, head: str, jobs: list[dict], since: str, since_run: str, commits: list[str]) -> str:
    """What the issue, or a comment on it, says about one red night."""
    lines = [f"Nightly run {run_url} failed on main at {head[:7]}.", "", "Failed jobs:"]
    lines += [f"- {job['name']}: {job['url']}" for job in jobs] or ["- none listed: read the run"]
    lines.append("")
    if not since:
        lines.append("No nightly has been green yet, so there is no list of commits since one.")
    elif not commits:
        lines.append(f"No commits since the last green nightly ({since_run}, at {since[:7]}): "
                     "the same commit failed now, so suspect the runner or a flaky test first.")
    else:
        lines.append(f"Commits since the last green nightly ({since_run}, at {since[:7]}):")
        lines += [f"- {commit}" for commit in commits]
    lines += ["", f"Next: just ci-wait {run_url.rsplit('/', 1)[-1]}"]
    return "\n".join(lines)


def open_issue_of(issues: list[dict]) -> dict | None:
    """The open issue titled TITLE, whatever its labels now (triage may have
    changed them), among `gh issue list --json number,title,body`."""
    return next((issue for issue in issues if issue["title"] == TITLE), None)


def plan(open_issue: dict | None, comments: list[dict], run_url: str) -> tuple[str, int | None]:
    """("open", None) with no open issue of the title; ("comment", n) on the
    open one; ("none", n) if it already names this run."""
    if open_issue is None:
        return "open", None
    if any(run_url in comment.get("body", "") for comment in comments) or run_url in open_issue.get("body", ""):
        return "none", open_issue["number"]
    return "comment", open_issue["number"]


def commits_since(since: str, head: str) -> list[str]:
    if not since:
        return []
    result = git("log", "--format=%h %s", f"{since}..{head}")
    if result.returncode != 0:
        return [f"(git log {since[:7]}..{head[:7]} failed: {result.stderr.strip()})"]
    return result.stdout.splitlines()


def report(run_id: str, head: str, since: str, since_run: str, dry_run: bool) -> list[str]:
    run = json.loads(gh("run", "view", run_id, "--json", "url,jobs"))
    text = body(run["url"], head, failed_jobs(run["jobs"]), since, since_run, commits_since(since, head))
    issues = json.loads(gh("issue", "list", "--state", "open", "--limit", "500", "--json", "number,title,body"))
    open_issue = open_issue_of(issues)
    comments = (
        json.loads(gh("issue", "view", str(open_issue["number"]), "--json", "comments"))["comments"]
        if open_issue else []
    )
    action, number = plan(open_issue, comments, run["url"])
    if action == "none":
        say(f"Issue #{number} already names {run['url']}; nothing posted.")
    elif dry_run:
        target = f"open the issue \"{TITLE}\", labelled {LABEL}" if action == "open" else f"comment on #{number}"
        say(f"Dry run: would {target}, saying:\n\n{text}")
    elif action == "open":
        url = gh("issue", "create", "--title", TITLE, "--label", LABEL, "--body", text).strip()
        say(f"Opened {url}.")
    else:
        gh("issue", "comment", str(number), "--body", text)
        say(f"Commented on #{number}.")
    return [f"action={action}"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("command", choices=["decide", "results", "report"])
    parser.add_argument("--dry-run", action="store_true", help="report: post nothing, print what it would")
    arguments = parser.parse_args()
    env = os.environ
    try:
        if arguments.command == "decide":
            lines = decide(env["EVENT"], env["HEAD"])
        elif arguments.command == "results":
            lines = results(env["HEAD"])
        else:
            lines = report(env["RUN_ID"], env["HEAD"], env.get("SINCE", ""), env.get("SINCE_RUN", ""),
                           arguments.dry_run)
    except Failed as failed:
        print(f"tools/nightly.py: {failed}", file=sys.stderr)
        return 1
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
