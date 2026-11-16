#!/usr/bin/env python3
"""Say whether PRs, open or merged, carry passed review records for their
heads; or post that verdict as the "Reviews recorded" check (retro of
2026-11-15, candidate 5).

Usage:
  just review-audit 382 393 399        # one line per PR; exit 1 if any lacks one
  python3 tools/review_audit.py --post-check 400   # CI's Reviews workflow

The rule is tools/reviews.py's, read as `just merge` reads it
(merge.review_status): the kinds a head needs by the paths it touches, the
records by those who can push, a clean merge of origin/main keeping them.
For a merged PR it judges the head that merged.

`--post-check` posts the verdict on the PR's head as a completed check run
named "Reviews recorded", replacing the one there if any, so the head carries
exactly one, and it is never left pending: the workflow (.github/workflows/
reviews.yml) runs on every push to a PR and on every review record, from
main's copy of this file, so a PR cannot change the rule it is judged by.
`just merge` ignores this check and reads the records itself, so a verdict
GitHub has not caught up with never holds a landing back.

Exit status: 0 every PR reviewed (or, with --post-check, the check posted);
1 a PR lacks a record; 2 gh or git failed.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Callable

import merge

CHECK = "Reviews recorded"

Gh = Callable[..., str]


def verdict(number: int) -> tuple[str, str, tuple[str, ...], list[str]]:
    """PR `number`'s state, the head judged, the kinds it needs and what is missing."""
    pr = merge.pull_request(number)
    head = pr["headRefOid"]
    needed, missing = merge.review_status(pr, head)
    return pr["state"].lower(), head, needed, missing


def line(number: int, state: str, head: str, needed: tuple[str, ...], missing: list[str]) -> str:
    if not missing:
        return f"#{number} ({state}, {head[:7]}): reviewed, {' and '.join(needed)} passed."
    return f"#{number} ({state}, {head[:7]}): NOT reviewed. " + " ".join(missing)


def post_check(gh: Gh, head: str, needed: tuple[str, ...], missing: list[str]) -> None:
    """Posts the verdict on `head` as the one "Reviews recorded" check run."""
    conclusion = "failure" if missing else "success"
    title = f"Missing: {len(missing)} of {len(needed)} kinds" if missing else f"Passed: {' and '.join(needed)}"
    summary = "\n".join(f"- {why}" for why in missing) if missing else f"Passed reviews of {head[:7]} are recorded."
    if missing:
        summary += "\n\nNext: have the head reviewed, then `just review-record <pr> <kind> <model> PASS <findings>`."
    found = gh("api", f"repos/{{owner}}/{{repo}}/commits/{head}/check-runs?check_name={CHECK.replace(' ', '%20')}",
               "--jq", '.check_runs[] | select(.app.slug == "github-actions") | .id').split()
    fields = ["-f", f"name={CHECK}", "-f", "status=completed", "-f", f"conclusion={conclusion}",
              "-f", f"output[title]={title}", "-f", f"output[summary]={summary}"]
    if found:
        gh("api", "--method", "PATCH", f"repos/{{owner}}/{{repo}}/check-runs/{found[0]}", *fields)
    else:
        gh("api", "--method", "POST", "repos/{owner}/{repo}/check-runs", "-f", f"head_sha={head}", *fields)
    print(f"{CHECK} on {head[:7]}: {conclusion}. {title}.")


def main() -> int:
    parser = argparse.ArgumentParser(prog="just review-audit",
                                     description="Say whether PRs carry passed review records for their heads.")
    parser.add_argument("prs", type=int, nargs="+", help="pull request numbers, open or merged")
    parser.add_argument("--post-check", action="store_true", help=f'post the verdict as the "{CHECK}" check (CI)')
    arguments = parser.parse_args()
    unreviewed = 0
    try:
        for number in arguments.prs:
            state, head, needed, missing = verdict(number)
            print(line(number, state, head, needed, missing), flush=True)
            unreviewed += bool(missing)
            if arguments.post_check:
                post_check(merge.gh, head, needed, missing)
    except merge.Refused as failed:
        print(f"just review-audit: {failed}", file=sys.stderr)
        return 2
    if arguments.post_check:
        return 0
    return 1 if unreviewed else 0


if __name__ == "__main__":
    sys.exit(main())
