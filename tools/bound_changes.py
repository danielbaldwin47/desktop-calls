#!/usr/bin/env python3
"""Fail a change that loosens a timing bound in the tests without naming the
measurement behind it (ticket #398, from the retro of 2026-11-15, candidate
10). `just lint`, and so `just check` and CI, run it.

A flake is fixed at its cause, "without loosening a bound" (2026-11-05).
PR #379 still added a CI-only 100 ms bound on an inferred cause, and both
its reviews passed it. So, in the harness and the automation crate's tests,
each of these needs a `bound-change:` comment on the line or within the
three lines above it, naming the measurement (and its PR or issue):

  - a raised Duration: a hunk whose largest `Duration::from_secs/millis/
    micros/nanos(...)`, `from_secs_f64(...)` or `Duration::new(s, ns)` is
    bigger than the largest it removed, where it adds no more Durations
    than it removed (arguments may be sums and products of numbers, and may
    run over several lines), which also catches a deadline renamed and
    raised in one go;
  - a bound chosen on whether this is CI (`var_os("CI")`, `var("CI")`,
    `GITHUB_ACTIONS`, outside comments), and a new `const` or `static` of
    any type but a string named for CI (`CI_SLACK_MS`).

A new scenario's own deadlines are not flagged: on the last 30 commits to
these crates that would have asked for 14 records, 12 of them for ordinary
new deadlines, and a record every change writes is a record nobody reads.
Over main's last 200 commits to these crates, as written, it flags three:
#379, the CI_SHOW_DEADLINE that #379 cited as precedent (1e5d39c), and a
100 ms settle replaced by a 10 s redraw wait (ebe59be, #87), where the
record would say it is a wait, not a bound.

  // bound-change: first keystroke measured at 12 ms warm, 31 ms cold on CI (#378)
  const CI_FIRST_KEYSTROKE: Duration = Duration::from_millis(100);

It compares the working tree with where the branch left origin/main; on
GitHub, with the commit's first parent (a PR's merge commit, or main's
previous commit). BOUND_CHANGES_BASE names another base. It reads only what
the diff adds, so a bound already in the tree is left alone.
"""

from __future__ import annotations

import ast
import dataclasses
import os
import re
import subprocess
import sys

PATHS = ("crates/test-harness", "crates/automation")
RECORD = "bound-change:"
RECORD_REACH = 3

UNIT_NANOS = {"nanos": 1, "micros": 1_000, "millis": 1_000_000, "secs": 1_000_000_000}
UNIT_NANOS.update(secs_f32=UNIT_NANOS["secs"], secs_f64=UNIT_NANOS["secs"])
DURATION = re.compile(r"Duration::(?:from_(nanos|micros|millis|secs_f32|secs_f64|secs)|(new))\s*\(")
DECLARATION = re.compile(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?(?:const|static)\s+(\w+)\s*:")
CI_TEST = re.compile(r"""var(?:_os)?\(\s*"(?:CI|GITHUB_ACTIONS)"\s*\)""")
CI_NAME = re.compile(r"(?:^|_)CI(?:_|$)")
HUNK = re.compile(r"^@@ -\d+(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")


@dataclasses.dataclass
class Hunk:
    path: str
    start: int
    removed: list[str] = dataclasses.field(default_factory=list)
    added: list[tuple[int, str]] = dataclasses.field(default_factory=list)


def arithmetic(text: str) -> float | None:
    """The value of `text` if it is numbers joined by + - * / and brackets,
    else None (a name, a call)."""
    try:
        tree = ast.parse(text.replace("_", "").strip(), mode="eval")
    except SyntaxError:
        return None
    def value(node):
        if isinstance(node, ast.Constant) and isinstance(node.value, (int, float)):
            return node.value
        if isinstance(node, ast.BinOp) and isinstance(node.op, (ast.Add, ast.Sub, ast.Mult, ast.Div)):
            left, right = value(node.left), value(node.right)
            if left is None or right is None or (isinstance(node.op, ast.Div) and right == 0):
                return None
            return {ast.Add: left + right, ast.Sub: left - right,
                    ast.Mult: left * right, ast.Div: left / right}[type(node.op)]
        return None
    return value(tree.body)


def arguments(text: str, start: int) -> str:
    """The text between the bracket before `start` and its match."""
    depth = 1
    for index in range(start, len(text)):
        depth += {"(": 1, ")": -1}.get(text[index], 0)
        if depth == 0:
            return text[start:index]
    return text[start:]


def nanos(text: str) -> list[int]:
    """Every Duration on `text` whose length is written out in numbers, in
    nanoseconds."""
    found = []
    for match in DURATION.finditer(text):
        inside = arguments(text, match.end())
        if match.group(2):  # Duration::new(seconds, nanoseconds)
            parts = [arithmetic(part) for part in inside.split(",")]
            if len(parts) == 2 and None not in parts:
                found.append(int(parts[0] * UNIT_NANOS["secs"] + parts[1]))
        elif (number := arithmetic(inside)) is not None:
            found.append(int(number * UNIT_NANOS[match.group(1)]))
    return found


def code(line: str) -> str:
    """`line` without a `//` comment."""
    return line.split("//", 1)[0]


def hunks(diff: str) -> list[Hunk]:
    """The hunks of a `git diff -U0`, with each added line's number in the new file."""
    found: list[Hunk] = []
    path = ""
    removing = adding = 0  # lines the current hunk still holds
    for line in diff.splitlines():
        if removing and line.startswith("-"):
            removing -= 1
            if path:
                found[-1].removed.append(line[1:])
        elif adding and line.startswith("+"):
            adding -= 1
            if path:
                hunk = found[-1]
                hunk.added.append((hunk.start + len(hunk.added), line[1:]))
        elif line.startswith("+++ "):
            path = line[6:] if line.startswith("+++ b/") else ""
        elif heading := HUNK.match(line):
            removing, adding = int(heading.group(1) or 1), int(heading.group(3) or 1)
            found.append(Hunk(path, int(heading.group(2))))
    return found


def loosened(hunk: Hunk) -> list[tuple[int, str]]:
    """The added lines of `hunk` that need a record, and why."""
    found = []
    removed_names = {match.group(1) for line in hunk.removed if (match := DECLARATION.match(line))}
    before = nanos("\n".join(code(line) for line in hunk.removed))
    after = nanos("\n".join(code(line) for _, line in hunk.added))
    # As many Durations or fewer: a bound changed, not new deadlines added.
    if before and after and len(after) <= len(before) and max(after) > max(before):
        number = next(number for number, line in hunk.added if "Duration" in code(line))
        found.append((number, f"raises a Duration from {max(before) / 1e6:g} ms to {max(after) / 1e6:g} ms"))
    for number, line in hunk.added:
        declared = DECLARATION.match(line)
        if (declared and declared.group(1) not in removed_names and CI_NAME.search(declared.group(1))
                and not re.search(r":\s*&(?:'static\s+)?str\b", line)):  # an env var's name, not a bound
            found.append((number, f"adds the CI bound {declared.group(1)}"))
        elif CI_TEST.search(code(line)):
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


def untracked() -> str:
    """New files git doesn't track yet, as diffs, so a bound in one is read
    before it is added."""
    listed = subprocess.run(["git", "-c", "safe.directory=*", "ls-files", "--others", "--exclude-standard",
                             "-z", "--", *PATHS], capture_output=True, text=True)
    diffs = []
    for path in filter(None, listed.stdout.split("\0")):
        diffs.append(subprocess.run(["git", "diff", "--no-index", "-U0", "--no-ext-diff", "--src-prefix=a/",
                                     "--dst-prefix=b/", "/dev/null", path], capture_output=True, text=True).stdout)
    return "".join(diffs)


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
    found = problems(result.stdout + untracked())
    if found:
        print("Timing bounds loosened without a recorded measurement (#398). Fix the cause, or put\n"
              f"`// {RECORD} <the measurement, with its PR or issue>` on or just above each line:", file=sys.stderr)
        print("\n".join(found), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
