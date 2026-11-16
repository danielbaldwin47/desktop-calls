#!/usr/bin/env python3
"""Whether a PR's green CI run was rerun without a recorded cause (ticket
#396, from the retro of 2026-11-15, candidate 1). `just merge`, and so `just
land`, refuse one; `just ci-rerun` (tools/ci_rerun.py) is the one way to make
the record and rerun.

A rerun's pass proves nothing, and on 2026-11-15 one on main nearly hid a
lost Missed Call (#356). So a run at attempt 2 or later counts only beside a
PR comment whose first line names the run and the issue holding its cause:

  Rerun: <run id> cause: #<issue>

Only comments by those who can push count (the repo is public), and a hidden
comment counts for nothing, as with review records (tools/reviews.py).
"""

from __future__ import annotations

import dataclasses
import re

import reviews

FIRST_LINE = re.compile(r"Rerun: (?P<run>\d+) cause: #(?P<issue>\d+)")
# A check's link: https://github.com/<owner>/<repo>/actions/runs/<run>/job/<job>.
RUN_IN_LINK = re.compile(r"/actions/runs/(\d+)")


@dataclasses.dataclass(frozen=True)
class Record:
    """One rerun comment's first line."""

    run: str
    issue: int


def first_line(run: str, issue: int) -> str:
    return f"Rerun: {run} cause: #{issue}"


def parse(body: str) -> Record | None:
    """The rerun a comment records, or None if its first line records none."""
    lines = body.strip().splitlines()
    found = FIRST_LINE.fullmatch(lines[0].strip()) if lines else None
    return Record(found["run"], int(found["issue"])) if found else None


def records_in(comments: list[dict]) -> list[Record]:
    """The rerun records among a PR's comments (`gh pr view --json comments`)."""
    return [
        record
        for comment in comments
        if comment.get("authorAssociation") in reviews.RECORDERS
        and not comment.get("isMinimized")
        and (record := parse(comment["body"]))
    ]


def run_of(link: str) -> str | None:
    """The run a check's link points into."""
    found = RUN_IN_LINK.search(link or "")
    return found.group(1) if found else None


def problem(run: str, attempt: int, records: list[Record]) -> str | None:
    """None if `run`, at `attempt`, may land: a first attempt, or a rerun
    with a record. Else what is missing."""
    if attempt <= 1 or any(record.run == run for record in records):
        return None
    return f"run {run} passed on attempt {attempt}, and no `Rerun: {run} cause: #<issue>` record names its cause."
