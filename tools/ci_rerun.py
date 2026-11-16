#!/usr/bin/env python3
"""Rerun a red CI run's failed jobs, with the issue that holds its cause on
record (ticket #396, from the retro of 2026-11-15, candidate 1).

Usage (through just):
  just ci-rerun 43768035550 378

A rerun's pass proves nothing, so the cause comes first: read the run (`just
ci-wait <run>`), find the cause by event order, and file an issue with the
evidence. A cause inside your diff is fixed by a push, not a rerun.

It refuses unless the issue exists (and is an issue, not a PR) and the run
has finished. Then it posts `Rerun: <run> cause: #<issue>`: on the PR, if the
run is a PR's (`just merge` and `just land` refuse a rerun PR without it,
tools/reruns.py), or on the issue, for a run on main. Last, it reruns the
run's failed jobs. For a run that already passed on a later attempt, it
posts the record and reruns nothing.

Exit status: 0 recorded (and rerun, if it was red); 1 refused, nothing posted.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Callable

import merge
import reruns

# Runs gh with these arguments and returns its standard output, or raises
# merge.Refused (merge.gh).
Gh = Callable[..., str]


def require_issue(gh: Gh, issue: int) -> None:
    try:
        found = json.loads(gh("api", f"repos/{{owner}}/{{repo}}/issues/{issue}"))
    except merge.Refused:
        raise merge.Refused(
            f"#{issue} is not an issue here.\n"
            "Next: gh issue create   (with the failure's cause by event order), then rerun with its number"
        ) from None
    if "pull_request" in found:
        raise merge.Refused(
            f"#{issue} is a pull request; the record names the issue that holds the cause.\n"
            "Next: gh issue create   (with the failure's cause by event order), then rerun with its number"
        )


def pull_request_of(gh: Gh, run: dict) -> int:
    """The open PR whose head is the run's commit."""
    prs = json.loads(gh("pr", "list", "--head", run["headBranch"], "--state", "open",
                        "--json", "number,headRefOid"))
    if not prs:
        raise merge.Refused(f"No open PR has the branch {run['headBranch']}, so the record has nowhere to go.")
    pr = prs[0]
    if pr["headRefOid"] != run["headSha"]:
        raise merge.Refused(
            f"Run {run['id']} tested {run['headSha'][:7]}, and PR #{pr['number']}'s head is now "
            f"{pr['headRefOid'][:7]}: the newer head has its own run.\nNext: just ci-wait"
        )
    return pr["number"]


def rerun(gh: Gh, run_id: str, issue: int) -> int:
    require_issue(gh, issue)
    run = json.loads(gh("run", "view", run_id, "--json", "attempt,headBranch,headSha,event,status,conclusion"))
    run["id"] = run_id
    if run["status"] != "completed":
        raise merge.Refused(f"Run {run_id} is {run['status'].replace('_', ' ')}.\nNext: just ci-wait {run_id}")
    passed = run["conclusion"] == "success"
    if passed and run["attempt"] <= 1:
        raise merge.Refused(f"Run {run_id} passed on its first attempt: there is nothing to rerun or record.")
    record = reruns.first_line(run_id, issue)
    if run["event"] == "pull_request":
        number = pull_request_of(gh, run)
        gh("pr", "comment", str(number), "--body", f"{record}\n\nRecorded with `just ci-rerun`.\n")
        print(f"Recorded on PR #{number}: {record}")
    else:
        gh("issue", "comment", str(issue), "--body",
           f"{record}\n\nRun {run_id} on {run['headBranch']} at {run['headSha'][:7]}, recorded with `just ci-rerun`.\n")
        print(f"Recorded on issue #{issue}: {record}")
    if passed:
        print(f"Run {run_id} already passed on attempt {run['attempt']}; nothing rerun.")
        return 0
    gh("run", "rerun", run_id, "--failed")
    print(f"Rerunning run {run_id}'s failed jobs.\nNext: just ci-wait {run_id}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(prog="just ci-rerun",
                                     description="Rerun a red CI run's failed jobs, with its cause's issue on record.")
    parser.add_argument("run", type=lambda text: str(int(text)), help="the run id (digits, not a URL)")
    parser.add_argument("issue", type=int, help="the issue holding the failure's cause, found by event order")
    arguments = parser.parse_args()
    try:
        return rerun(merge.gh, arguments.run, arguments.issue)
    except merge.Refused as refusal:
        print(f"Not rerun: {refusal}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
