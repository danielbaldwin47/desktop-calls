#!/usr/bin/env python3
"""Whether a change touches code, or only docs that no code reads.

CI runs the Rust jobs only when a change touches code, and `just merge` lets
a PR through when the only commits it lacks change docs alone. Both decide
with this one rule:

  A path is docs if it is under docs/ or ends in .md. A docs path still
  counts as code if any file outside docs/ and *.md names it, or names a
  directory holding it: the scorecard test reads docs/wiki/polish-scorecard.md,
  and the conformance test reads the D-Bus snapshot under docs/research/.
  Comments count too, which can only run CI that wasn't needed.

Usage (CI's `changes` job):
  python3 tools/changes.py BASE HEAD

Prints `code=true` or `code=false` on standard output, for $GITHUB_OUTPUT,
and the reason on standard error. Anything it can't work out counts as code.
"""

from __future__ import annotations

import subprocess
import sys
from collections.abc import Callable

DOCS_DIRECTORY = "docs/"
DOCS_SUFFIX = ".md"
# How a file names a docs path it may read: docs/... or ....md.
DOCS_REFERENCE = (r"docs/[A-Za-z0-9_.-][A-Za-z0-9_./-]*", r"[A-Za-z0-9_./-]+\.md")
# Files that name docs paths only to describe this rule.
DESCRIBING_THE_RULE = (":!tools/changes.py", ":!tools/merge.py")

# Runs git with these arguments, allowing these exit statuses, and returns its
# standard output.
Git = Callable[..., str]


def is_docs(path: str) -> bool:
    return path.startswith(DOCS_DIRECTORY) or path.endswith(DOCS_SUFFIX)


def docs_named_by_code(git: Git, trees: list[str], paths: list[str]) -> dict[str, str]:
    """Which of `paths` a file outside docs/ and *.md names in any of
    `trees`: maps each to the first file naming it."""
    patterns = [argument for pattern in DOCS_REFERENCE for argument in ("-e", pattern)]
    names: dict[str, str] = {}
    for tree in trees:
        # git grep exits 1 when nothing matches.
        output = git(
            "grep", "-o", "-I", "-E", *patterns, tree, "--",
            ".", ":!docs", ":!*.md", *DESCRIBING_THE_RULE,
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


def code_changes(git: Git, base: str, head: str) -> tuple[bool, str]:
    """Whether the change from `base` to `head` touches code, and why."""
    files = git("diff", "--name-only", "--no-renames", f"{base}...{head}").split()
    code = [path for path in files if not is_docs(path)]
    if code:
        return True, f"{len(code)} of {len(files)} changed files are code, such as {code[0]}."
    named = docs_named_by_code(git, [base, head], files)
    if named:
        path, file = next(iter(named.items()))
        return True, f"{path} is docs, but {file} names it."
    return False, f"All {len(files)} changed files are docs that no code names."


def run_git(*args: str, allowed: tuple[int, ...] = (0,)) -> str:
    result = subprocess.run(["git", *args], capture_output=True, text=True)
    if result.returncode not in allowed:
        raise RuntimeError(f"`git {' '.join(args)}` failed: {result.stderr.strip()}")
    return result.stdout


def main() -> int:
    base, head = (sys.argv[1:] + ["", ""])[:2]
    if not base.strip("0") or not head:
        code, reason = True, "No base to compare with (a new branch, or a manual run)."
    else:
        try:
            code, reason = code_changes(run_git, base, head)
        except RuntimeError as error:
            code, reason = True, f"Couldn't compare {base[:7]}...{head[:7]}: {error}"
    print(reason, file=sys.stderr)
    print(f"code={'true' if code else 'false'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
