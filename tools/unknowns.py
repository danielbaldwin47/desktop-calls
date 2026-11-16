#!/usr/bin/env python3
"""Keep the Real-Phone unknowns ledger honest (retro of 2026-11-15,
candidate 3). `just lint`, and so `just check` and CI, run it; `just
real-phone-unknowns [GROUP]` lists the open unknowns on a Real-Phone
Checklist group's path.

The justfile passes the ledger's path (`--ledger`), and names no other: a
docs page a tool names would make every change to it count as code, and
this way the justfile's naming it does, so a change to the ledger alone
still runs this check (tools/changes.py).

It fails when:
  - code in the Simulated Phone or the checklist's steps says "inferred" or
    "unverified" in a comment with no ledger id (`RPU-7`) within two lines
    of the word, in the same comment, so a long header names each of its
    assumptions where it makes it;
  - code names an id the ledger lacks, or calls an observed entry inferred;
  - an entry lacks Assumed, Source, Observed by or Status, has a Status
    other than `open` or `observed ...`, or names a step the checklist
    doesn't have;
  - an id is used twice.
"""

from __future__ import annotations

import argparse
import dataclasses
import pathlib
import re
import sys

# The ledger's path, from --ledger, for messages.
LEDGER = "the ledger"
STEPS = "crates/automation/src/checklist/steps.rs"
# Where an assumption about the real stack is written down as code.
SCANNED = ("crates/simulated-phone/src", "crates/automation/src/checklist/steps")
SCANNED_FILES = (STEPS,)
MARK = re.compile(r"\b(inferred|unverified)\b", re.IGNORECASE)
ID = re.compile(r"\bRPU-(\d+)\b")
REACH = 2
HEADING = re.compile(r"^## RPU-(\d+): (.+)$")
FIELD = re.compile(r"^- ([A-Za-z ]+): (.*)$")
FIELDS = ("Assumed", "Source", "Observed by", "Status")
NO_STEP = "no step yet"
STEP_ID = re.compile(r'\bid:\s*"(\d+\.\d+)"')


@dataclasses.dataclass
class Entry:
    number: int
    title: str
    line: int
    fields: dict[str, str]

    @property
    def open(self) -> bool:
        return self.fields.get("Status", "") == "open"

    def steps(self) -> list[str]:
        observed_by = self.fields.get("Observed by", "")
        if observed_by == NO_STEP:
            return []
        return [step.strip() for step in observed_by.split(",") if step.strip()]


def ledger(text: str) -> tuple[list[Entry], list[str]]:
    """The ledger's entries, and what is wrong with their form."""
    entries: list[Entry] = []
    problems: list[str] = []
    for number, line in enumerate(text.splitlines(), 1):
        heading = HEADING.match(line)
        if heading:
            entries.append(Entry(int(heading.group(1)), heading.group(2), number, {}))
            continue
        field = FIELD.match(line)
        if field and entries:
            entries[-1].fields[field.group(1)] = field.group(2).strip()
    seen: set[int] = set()
    for entry in entries:
        where = f"{LEDGER}:{entry.line} RPU-{entry.number}"
        if entry.number in seen:
            problems.append(f"{where}: the id is used twice; ids are never reused.")
        seen.add(entry.number)
        missing = [name for name in FIELDS if not entry.fields.get(name)]
        if missing:
            problems.append(f"{where}: no {', '.join(missing)}.")
        status = entry.fields.get("Status", "")
        if status and status != "open" and not re.match(r"observed \d{4}-\d{2}-\d{2} \(.+\): .+", status):
            problems.append(f"{where}: Status is `open` or `observed <date> (<issue or note>): <what was seen>`, not {status!r}.")
    return entries, problems


def comment_blocks(text: str) -> list[tuple[int, str]]:
    """Each run of `//` comment lines, as (first line number, its text), and
    each `/* */`-free line's trailing comment on its own."""
    blocks: list[tuple[int, str]] = []
    current: list[str] = []
    start = 0
    for number, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if stripped.startswith("//"):
            if not current:
                start = number
            current.append(stripped)
            continue
        if current:
            blocks.append((start, "\n".join(current)))
            current = []
        if "//" in line:
            blocks.append((number, line[line.index("//") :]))
    if current:
        blocks.append((start, "\n".join(current)))
    return blocks


def code_problems(files: dict[str, str], entries: list[Entry]) -> list[str]:
    """What is wrong with how `files` (path to text) cite the ledger."""
    known = {entry.number: entry for entry in entries}
    problems = []
    for path, text in sorted(files.items()):
        for start, block in comment_blocks(text):
            lines = block.split("\n")
            for offset, comment in enumerate(lines):
                line = start + offset
                for number in ID.findall(comment):
                    if int(number) not in known:
                        problems.append(f"{path}:{line}: RPU-{number} isn't in {LEDGER}.")
                mark = MARK.search(comment)
                if not mark:
                    continue
                near = "\n".join(lines[max(0, offset - REACH) : offset + REACH + 1])
                cited = [int(number) for number in ID.findall(near)]
                if not cited:
                    problems.append(
                        f"{path}:{line}: says {mark.group(1)!r} with no ledger id within {REACH} lines: add an entry to {LEDGER} and name it here, as `(inferred, RPU-7)`."
                    )
                for number in cited:
                    entry = known.get(number)
                    if entry and not entry.open:
                        problems.append(
                            f"{path}:{line}: calls RPU-{number} {mark.group(1)}, but {LEDGER} says it was {entry.fields['Status']}. Correct the code and drop the word."
                        )
    return problems


def step_problems(entries: list[Entry], steps: set[str]) -> list[str]:
    problems = []
    for entry in entries:
        observed_by = entry.fields.get("Observed by", "")
        if not observed_by:
            continue
        for step in entry.steps():
            if step not in steps:
                problems.append(
                    f"{LEDGER}:{entry.line} RPU-{entry.number}: Observed by names step {step!r}, which the checklist doesn't have; name a step from {STEPS}, or `{NO_STEP}`."
                )
    return problems


def check(ledger_text: str, steps_text: str, files: dict[str, str]) -> list[str]:
    entries, problems = ledger(ledger_text)
    if not entries:
        return [f"{LEDGER} has no entries this check can read (`## RPU-<n>: <title>`)."]
    steps = set(STEP_ID.findall(steps_text))
    return problems + step_problems(entries, steps) + code_problems(files, entries)


def open_on_path(ledger_text: str, group: int | None) -> str:
    """The open unknowns a group's steps observe (every group's if `group`
    is None), and those no step observes, for a message before a sitting."""
    entries, _ = ledger(ledger_text)
    open_entries = [entry for entry in entries if entry.open]
    prefix = f"{group}." if group is not None else ""
    observed = [entry for entry in open_entries if any(step.startswith(prefix) for step in entry.steps())]
    lines = []
    scope = f"group {group}" if group is not None else "the Real-Phone Checklist"
    lines.append(f"Open unknowns on {scope}'s path: {len(observed)} (of {len(open_entries)} open in {LEDGER}).")
    for entry in observed:
        steps = ", ".join(step for step in entry.steps() if step.startswith(prefix))
        lines.append(f"  RPU-{entry.number} (step {steps}): {entry.title}")
    unobserved = [entry for entry in open_entries if not entry.steps()]
    if unobserved:
        lines.append(f"Open with no step to observe them: {', '.join(f'RPU-{entry.number}' for entry in unobserved)}.")
    lines.append("A step resting on one of these observes it and never fails on it alone; tell the owner which before the sitting.")
    return "\n".join(lines)


def scanned_files(root: pathlib.Path) -> dict[str, str]:
    files = {}
    for directory in SCANNED:
        for path in sorted((root / directory).rglob("*.rs")):
            files[str(path.relative_to(root))] = path.read_text()
    for name in SCANNED_FILES:
        files[name] = (root / name).read_text()
    return files


def main(arguments: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("group", nargs="?", type=int, help="list the open unknowns on this group's path")
    parser.add_argument("--list", action="store_true", help="list the open unknowns on every group's path")
    parser.add_argument("--ledger", required=True, help="the ledger, relative to the repository")
    options = parser.parse_args(arguments)
    global LEDGER
    LEDGER = options.ledger
    root = pathlib.Path(__file__).resolve().parent.parent
    ledger_text = (root / LEDGER).read_text()
    if options.group is not None or options.list:
        print(open_on_path(ledger_text, options.group))
        return 0
    problems = check(ledger_text, (root / STEPS).read_text(), scanned_files(root))
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
