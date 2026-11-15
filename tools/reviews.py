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
history from which every later commit (but main's own) is a merge of
origin/main whose tree is exactly what `git merge-tree` makes of its two
parents: no conflict resolved, nothing slipped in. The merge is recomputed;
commit messages are not read. Of the reviews of one kind that count, the latest decides,
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
    """The paths the PR's diff touches: from where `head` left `base`. NUL-
    separated, since git quotes a path with other than ASCII in it."""
    return [path for path in git("diff", "--name-only", "-z", "--no-renames", f"{base}...{head}").split("\0") if path]


def unreviewed_commits(git: Git, reviewed: str, head: str, base: str) -> str | None:
    """None if every commit from `reviewed` to `head` that `base` lacks is a
    merge of `base` whose tree is exactly what git's own merge of its two
    parents makes: no conflict resolved, nothing slipped in. Else the first
    commit that isn't, and why. Recomputing the merge compares the change,
    not the commit messages."""
    listing = git("rev-list", "--parents", head, f"^{reviewed}", f"^{base}").splitlines()
    for line in listing:
        commit, *parents = line.split()
        if len(parents) != 2:
            return f"{commit[:7]} is a new commit"
        if not any(git("merge-base", parent, base, allowed=(0, 1)).strip() == parent for parent in parents):
            return f"{commit[:7]} merges something other than {base}"
        merged = git("merge-tree", "--write-tree", "--no-messages", *parents, allowed=(0, 1)).split()
        tree = git("rev-parse", f"{commit}^{{tree}}").strip()
        # On a conflict, merge-tree still prints a tree first, then a line per
        # conflicted path: a merge that kept git's conflicted tree (such as
        # an edit kept over main's delete) must not pass for a clean one.
        if len(merged) != 1 or merged[0] != tree:
            return f"{commit[:7]} is a merge that resolved a conflict or changed more"
    return None


def why_not_covered(git: Git, reviewed: str, head: str, base: str) -> str | None:
    """None if a review of `reviewed` counts for `head`; else why not."""
    if reviewed == head:
        return None
    if git("cat-file", "-t", reviewed, allowed=(0, 128)).strip() != "commit":
        return f"{reviewed[:7]} is not in this clone (was the branch rebased?)"
    if git("merge-base", reviewed, head, allowed=(0, 1)).strip() != reviewed:
        return f"{reviewed[:7]} is not in the history of {head[:7]} (was the branch rebased?)"
    why = unreviewed_commits(git, reviewed, head, base)
    return f"{why}, since {reviewed[:7]}" if why else None


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
