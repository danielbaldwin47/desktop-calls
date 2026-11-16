#!/usr/bin/env python3
"""Fail a change that loosens a timing bound in the tests without naming the
measurement behind it (ticket #398, from the retro of 2026-11-15, candidate
10). `just lint`, and so `just check` and CI, run it.

A flake is fixed at its cause, "without loosening a bound" (2026-11-05).
PR #379 still added a CI-only 100 ms bound on an inferred cause, and both
its reviews passed it. So, in the harness and the automation crate's tests,
each of these needs a `bound-change:` comment on the line or within the
three lines above it, naming the measurement (and its PR or issue):

  - a raised Duration: a line changed only by a bigger number of
    `Duration::from_secs/millis/micros/nanos(...)` or `from_secs_f64(...)`;
  - a bound chosen on whether this is CI (`var_os("CI")`, `var("CI")`,
    `GITHUB_ACTIONS`), and a new `const` or `static` named for CI.

A new scenario's own deadlines are not flagged: on the last 30 commits to
these crates that would have asked for 14 records, 12 of them for ordinary
new deadlines, and a record every change writes is a record nobody reads.
Over main's last 200 commits to these crates, as written, it flags only
#379 and the CI_SHOW_DEADLINE that #379 cited as precedent (1e5d39c).

  // bound-change: first keystroke measured at 12 ms warm, 31 ms cold on CI (#378)
  const CI_FIRST_KEYSTROKE: Duration = Duration::from_millis(100);

It compares the working tree with where the branch left origin/main; on
GitHub, with the commit's first parent (a PR's merge commit, or main's
previous commit). BOUND_CHANGES_BASE names another base. It reads only what
the diff adds, so a bound already in the tree is left alone.
"""

from __future__ import annotations

import dataclasses
import os
import re
import subprocess
import sys

PATHS = ("crates/test-harness", "crates/automation")
RECORD = "bound-change:"
RECORD_REACH = 3

UNIT_NANOS = {"nanos": 1, "micros": 1_000, "millis": 1_000_000, "secs": 1_000_000_000}
DURATION = re.compile(r"Duration::from_(nanos|micros|millis|secs)\(\s*([\d_]+)\s*\)"
                      r"|Duration::from_secs_f(?:32|64)\(\s*([\d_.]+)\s*\)")
DECLARATION = re.compile(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?(?:const|static)\s+(\w+)\s*:")
CI_TEST = re.compile(r"""var(?:_os)?\(\s*"(?:CI|GITHUB_ACTIONS)"\s*\)""")
CI_NAME = re.compile(r"(?:^|_)CI(?:_|$)")
NUMBERS = re.compile(r"[\d_.]+")
HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@")


@dataclasses.dataclass
class Hunk:
    path: str
    start: int
    removed: list[str] = dataclasses.field(default_factory=list)
    added: list[tuple[int, str]] = dataclasses.field(default_factory=list)


def nanos(line: str) -> list[int]:
    """Every Duration literal on `line`, in nanoseconds."""
    found = []
    for unit, whole, seconds in DURATION.findall(line):
        found.append(int(whole.replace("_", "")) * UNIT_NANOS[unit] if unit
                     else int(float(seconds.replace("_", "")) * UNIT_NANOS["secs"]))
    return found


def shape(line: str) -> str:
    """`line` with every Duration and number blanked, so two lines that differ
    only in a bound compare equal, whatever its unit."""
    return NUMBERS.sub("#", DURATION.sub("Duration", line))


def hunks(diff: str) -> list[Hunk]:
    """The hunks of a `git diff -U0`, with each added line's number in the new file."""
    found: list[Hunk] = []
    path = ""
    for line in diff.splitlines():
        if line.startswith("+++ "):
            path = line[6:] if line.startswith("+++ b/") else ""
        elif line.startswith("--- "):
            continue
        elif heading := HUNK.match(line):
            found.append(Hunk(path, int(heading.group(1))))
        elif found and path and line.startswith("-"):
            found[-1].removed.append(line[1:])
        elif found and path and line.startswith("+"):
            hunk = found[-1]
            hunk.added.append((hunk.start + len(hunk.added), line[1:]))
    return found


def loosened(hunk: Hunk) -> list[tuple[int, str]]:
    """The added lines of `hunk` that need a record, and why."""
    found = []
    removed_names = {match.group(1) for line in hunk.removed if (match := DECLARATION.match(line))}
    raised = set()
    if len(hunk.removed) == len(hunk.added):
        for old, (number, new) in zip(hunk.removed, hunk.added):
            if shape(old) == shape(new) and nanos(old) and nanos(new):
                if any(after > before for before, after in zip(nanos(old), nanos(new))):
                    raised.add(number)
    for number, line in hunk.added:
        declared = DECLARATION.match(line)
        if number in raised:
            found.append((number, "raises a Duration"))
        elif (declared and declared.group(1) not in removed_names and CI_NAME.search(declared.group(1))
              and (nanos(line) or "Duration" in line)):
            found.append((number, f"adds the CI bound {declared.group(1)}"))
        elif CI_TEST.search(line):
            found.append((number, "chooses a bound on whether this is CI"))
    return found


def recorded(lines: list[str], number: int) -> bool:
    """Whether line `number` (from 1) of a file, or one of the lines just above it, holds a record."""
    return any(RECORD in line for line in lines[max(number - 1 - RECORD_REACH, 0):number])


def problems(diff: str, read=lambda path: open(path, encoding="utf-8").read()) -> list[str]:
    found = []
    for hunk in hunks(diff):
        needing = loosened(hunk)
        if not needing:
            continue
        lines = read(hunk.path).splitlines()
        found += [f"  {hunk.path}:{number}: {why}" for number, why in needing if not recorded(lines, number)]
    return found


def base() -> str | None:
    if chosen := os.environ.get("BOUND_CHANGES_BASE"):
        return chosen
    if os.environ.get("GITHUB_ACTIONS") == "true":
        return "HEAD^1"
    result = subprocess.run(["git", "-c", "safe.directory=*", "merge-base", "HEAD", "origin/main"],
                            capture_output=True, text=True)
    return result.stdout.strip() or None


def main() -> int:
    start = base()
    if not start:
        print("bound changes: no base to compare with (no origin/main); skipped.", file=sys.stderr)
        return 0
    result = subprocess.run(["git", "-c", "safe.directory=*", "diff", "-U0", "--no-renames", "--no-ext-diff",
                             "--src-prefix=a/", "--dst-prefix=b/", start, "--", *PATHS],
                            capture_output=True, text=True)
    if result.returncode != 0:
        print(f"bound changes: `git diff {start}` failed: {result.stderr.strip()}", file=sys.stderr)
        return 1
    found = problems(result.stdout)
    if found:
        print("Timing bounds loosened without a recorded measurement (#398). Fix the cause, or put\n"
              f"`// {RECORD} <the measurement, with its PR or issue>` on or just above each line:", file=sys.stderr)
        print("\n".join(found), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
