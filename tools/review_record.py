#!/usr/bin/env python3
"""Post a review's record on a PR, for its head on GitHub (ticket #327).

Usage (through just):
  just review-record 42 spec opus PASS findings.md
  just review-record 42 tooling sonnet FAIL findings.md

The comment's first line is the one `just merge` and `just land` read
(tools/reviews.py): `Review: <kind> by <model> on <full sha>: <verdict>`.
The findings file, if given, follows it: each finding and how it was
resolved. Kinds: standards-spec and spec for a PR touching product or
harness code, tooling for one touching docs or tooling only.

It records the head GitHub has, and refuses if this worktree has the PR's
branch at another commit: push first, so the commit reviewed is the one
recorded. Afterwards it says which kinds the PR still lacks.

This records that a review was claimed, not that it happened.

Exit status: 0 posted; 1 refused, nothing posted.
"""

from __future__ import annotations

import argparse
import pathlib
import subprocess
import sys

import merge
import reviews

NO_FINDINGS = "No findings were recorded with this review."


def refuse_bad_arguments(kind: str, model: str, verdict: str) -> None:
    if kind not in reviews.KINDS:
        raise merge.Refused(f"Unknown kind {kind!r}; the kinds are {', '.join(reviews.KINDS)}.")
    if model == "fable":
        raise merge.Refused("Fable never reviews here (the user's rule). Review with sonnet or opus.")
    if model not in reviews.MODELS:
        raise merge.Refused(f"Unknown model {model!r}; the models are {', '.join(reviews.MODELS)}.")
    if verdict not in reviews.VERDICTS:
        raise merge.Refused(f"Unknown verdict {verdict!r}; it is PASS or FAIL.")


def local_head_of(branch: str) -> str | None:
    """This worktree's HEAD if it has `branch` checked out, else None."""
    current = subprocess.run(["git", "branch", "--show-current"], capture_output=True, text=True)
    if current.returncode != 0 or current.stdout.strip() != branch:
        return None
    return merge.git("rev-parse", "HEAD").strip()


def body(kind: str, model: str, head: str, verdict: str, findings: str) -> str:
    return f"{reviews.first_line(kind, model, head, verdict)}\n\n{findings.strip() or NO_FINDINGS}\n"


def record(number: int, kind: str, model: str, verdict: str, findings_file: str | None) -> int:
    model = model.lower()
    refuse_bad_arguments(kind, model, verdict)
    findings = pathlib.Path(findings_file).read_text() if findings_file else ""
    pr = merge.pull_request(number)
    merge.require_open(pr)
    head = pr["headRefOid"]
    here = local_head_of(pr["headRefName"])
    if here is not None and here != head:
        raise merge.Refused(
            f"This worktree has {pr['headRefName']} at {here[:7]}, and PR #{number}'s head on GitHub is "
            f"{head[:7]}: a record names the commit GitHub has.\n"
            "Next: git push, review what you pushed, then record it"
        )
    merge.gh("pr", "comment", str(number), "--body", body(kind, model, head, verdict, findings))
    print(f"Recorded on PR #{number}: {reviews.first_line(kind, model, head, verdict)}")
    needed, missing = merge.review_status(pr, head)
    if kind not in needed:
        print(f"PR #{number} needs {' and '.join(needed)}; a {kind} review doesn't count toward it.")
    if missing:
        print(f"Still missing for {head[:7]}:")
        for line in missing:
            print(f"  {line}")
    else:
        print(f"PR #{number} has every review it needs on {head[:7]}.\nNext: just land {number}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(prog="just review-record", description="Post a review's record on a PR's head.")
    parser.add_argument("pr", type=int, help="the pull request number")
    parser.add_argument("kind", help=", ".join(reviews.KINDS))
    parser.add_argument("model", help=", ".join(reviews.MODELS))
    parser.add_argument("verdict", help="PASS or FAIL")
    parser.add_argument("findings", nargs="?", help="a file of findings and how each was resolved")
    arguments = parser.parse_args()
    try:
        return record(arguments.pr, arguments.kind, arguments.model, arguments.verdict, arguments.findings)
    except merge.Refused as refusal:
        print(f"Not recorded: {refusal}", file=sys.stderr)
        return 1
    except OSError as error:
        print(f"Not recorded: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
