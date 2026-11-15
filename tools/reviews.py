#!/usr/bin/env python3
"""Whether a PR's comments record passed reviews for the head that merges
(ticket #327). `just merge`, and so `just land`, refuse a PR without them;
`just review-record` (tools/review_record.py) posts one.

A review comment's first line says what was reviewed and the verdict:

  Review: <kind> by <model> on <full sha>: PASS      (or FAIL)

The kinds a PR needs depend on the paths its diff touches:

  Product or harness code: anything under crates/ or data/, and Cargo.toml,
  Cargo.lock and rust-toolchain.toml. A PR touching any of them needs a
  `standards-spec` and a `spec` review that passed.
  Anything else is docs or tooling (docs/, *.md, tools/, the justfile,
  .github/, .config/, the lint configs): one `tooling` review that passed.

A review counts for a head if it names the head, or a commit in the head's
history whose PR diff is the head's: commits since then only merged
origin/main, with no conflict resolution. The diff is compared, not the
commit messages. Of the reviews of one kind that count, the latest decides,
so a FAIL after a PASS on the same head refuses.

This records that a review was claimed, not that it happened: the agent
that lands is the one that posts the comment.
"""

from __future__ import annotations

import dataclasses
import re
from collections.abc import Callable

KINDS = ("standards-spec", "spec", "tooling")
PRODUCT_KINDS = ("standards-spec", "spec")
TOOLING_KINDS = ("tooling",)
# Reviewers may be these models. Never Fable (the user's rule, 2026-11-13).
MODELS = ("sonnet", "opus", "haiku")
VERDICTS = ("PASS", "FAIL")

PRODUCT_DIRECTORIES = ("crates/", "data/")
PRODUCT_FILES = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")

FIRST_LINE = re.compile(
    r"Review: (?P<kind>[a-z-]+) by (?P<model>[a-z]+) on (?P<sha>[0-9a-f]{40}): (?P<verdict>PASS|FAIL)"
)

# Runs git with these arguments, allowing these exit statuses, and returns its
# standard output (tools/changes.py's Git).
Git = Callable[..., str]


@dataclasses.dataclass(frozen=True)
class Record:
    """One review comment's first line."""

    kind: str
    model: str
    sha: str
    verdict: str

    def describe(self) -> str:
        return f"{self.verdict} by {self.model} on {self.sha[:7]}"


def first_line(kind: str, model: str, sha: str, verdict: str) -> str:
    return f"Review: {kind} by {model} on {sha}: {verdict}"


def parse(body: str) -> Record | None:
    """The review a comment records, or None if its first line records none."""
    lines = body.strip().splitlines()
    found = FIRST_LINE.fullmatch(lines[0].strip()) if lines else None
    if not found or found["kind"] not in KINDS or found["model"] not in MODELS:
        return None
    return Record(found["kind"], found["model"], found["sha"], found["verdict"])


def is_product(path: str) -> bool:
    return path.startswith(PRODUCT_DIRECTORIES) or path in PRODUCT_FILES


def kinds_needed(paths: list[str]) -> tuple[str, ...]:
    return PRODUCT_KINDS if any(is_product(path) for path in paths) else TOOLING_KINDS


def changed_paths(git: Git, base: str, head: str) -> list[str]:
    """The paths the PR's diff touches: from where `head` left `base`."""
    return git("diff", "--name-only", "--no-renames", f"{base}...{head}").split()


def pr_diff(git: Git, base: str, commit: str) -> list[str]:
    """What `commit` changes since it last took in `base`, without line
    numbers or blob ids, which a clean merge of `base` shifts. Two commits
    with the same PR diff make the same change."""
    since = git("merge-base", base, commit).strip()
    text = git(
        "diff", "-U0", "--no-renames", "--no-color", "--no-ext-diff", "--binary",
        "--src-prefix=a/", "--dst-prefix=b/", since, commit,
    )
    return [line for line in text.splitlines() if not line.startswith(("@@", "index "))]


def why_not_covered(git: Git, reviewed: str, head: str, base: str) -> str | None:
    """None if a review of `reviewed` counts for `head`; else why not."""
    if reviewed == head:
        return None
    if git("cat-file", "-t", reviewed, allowed=(0, 128)).strip() != "commit":
        return f"{reviewed[:7]} is not in this clone (was the branch rebased?)"
    if git("merge-base", reviewed, head, allowed=(0, 1)).strip() != reviewed:
        return f"{reviewed[:7]} is not in the history of {head[:7]} (was the branch rebased?)"
    if pr_diff(git, base, reviewed) == pr_diff(git, base, head):
        return None
    new = git("rev-list", "--no-merges", head, f"^{reviewed}", f"^{base}").split()
    named = ", ".join(sha[:7] for sha in new[:3]) + (f" and {len(new) - 3} more" if len(new) > 3 else "")
    since = f"commits {named}" if new else "a merge that resolved a conflict or changed more"
    return f"{since} changed the PR's diff since {reviewed[:7]}"


def problems(git: Git, records: list[Record], head: str, base: str) -> tuple[tuple[str, ...], list[str]]:
    """The kinds `head` needs, and what stops `records` (oldest first) from
    showing each passed on it. No problems: reviewed."""
    needed = kinds_needed(changed_paths(git, base, head))
    reasons: dict[str, str | None] = {}
    found = []
    for kind in needed:
        of_kind = [record for record in records if record.kind == kind]
        counting = []
        for record in of_kind:
            if record.sha not in reasons:
                reasons[record.sha] = why_not_covered(git, record.sha, head, base)
            if reasons[record.sha] is None:
                counting.append(record)
        if counting and counting[-1].verdict == "PASS":
            continue
        if counting:
            found.append(f"{kind}: the latest review of this head is {counting[-1].describe()}.")
        elif of_kind:
            last = of_kind[-1]
            found.append(f"{kind}: the latest, {last.describe()}, doesn't count: {reasons[last.sha]}.")
        else:
            found.append(f"{kind}: none recorded.")
    return needed, found
