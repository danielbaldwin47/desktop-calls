#!/usr/bin/env python3
"""No Real-Phone Checklist step may say it lands with a closed ticket.

A step without a script names, in `lands_with`, the open tickets whose
features it waits on. Once one of them closes, the step must have its
script, or name the open ticket it still waits on; otherwise the runner
tells the user "Not yet runnable (lands with #11)" for a ticket long
closed (ticket #120). This check asks GitHub about every ticket named.

Usage:
  python3 tools/checklist_tickets.py             # `just check` runs this
  python3 tools/checklist_tickets.py --ref SHA --closing 22,16
      # `just merge`: the steps at SHA, and the tickets the PR closes

It fails when it can't reach GitHub, so a skipped check never looks like a
passed one. Offline, set DESKTOP_CALLS_OFFLINE=1 to skip it, knowingly.
GitHub's token comes from GH_TOKEN or GITHUB_TOKEN, else from `gh auth
token`, else none: anonymous calls work on a public repository, within
GitHub's limit of 60 an hour.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Callable

STEPS = "crates/automation/src/checklist/steps.rs"
# The checklist has 27 steps; far fewer means the parser no longer fits the
# file, and the check would pass by seeing nothing.
FEWEST_STEPS = 20


@dataclass
class Step:
    id: str
    scripted: bool
    lands_with: list[int]


def steps(text: str) -> list[Step]:
    """Every `Step { ... }` in steps.rs, in order."""
    found = []
    for block in text.split("Step {")[1:]:
        block = block.split("},", 1)[0]
        id_ = re.search(r'\bid:\s*"([^"]+)"', block)
        script = re.search(r"\bscript:\s*(None|Some\()", block)
        lands = re.search(r"\blands_with:\s*&\[([^\]]*)\]", block)
        if not (id_ and script and lands):
            continue
        tickets = [int(n) for n in re.findall(r"\d+", lands.group(1))]
        found.append(Step(id_.group(1), script.group(1) != "None", tickets))
    return found


def problems(found: list[Step], closing: set[int], is_open: Callable[[int], bool]) -> list[str]:
    """What is wrong with `found`, one sentence each."""
    wrong = []
    for step in found:
        if step.scripted and step.lands_with:
            wrong.append(f"Step {step.id} has a script, so its lands_with must be empty.")
        if not step.scripted and not step.lands_with:
            wrong.append(f"Step {step.id} has no script, so it must name the open ticket it waits on.")
        for ticket in step.lands_with:
            if ticket in closing:
                wrong.append(f"Step {step.id} has no script and lands with #{ticket}, which this PR closes.")
            elif not is_open(ticket):
                wrong.append(f"Step {step.id} has no script and lands with #{ticket}, which is closed.")
    return wrong


def repository() -> str:
    """owner/name: GitHub Actions says, or else origin's URL."""
    if os.environ.get("GITHUB_REPOSITORY"):
        return os.environ["GITHUB_REPOSITORY"]
    url = subprocess.run(
        ["git", "remote", "get-url", "origin"], capture_output=True, text=True, check=True
    ).stdout.strip()
    match = re.search(r"github\.com[:/]([^/]+/[^/]+?)(?:\.git)?$", url)
    if not match:
        raise OSError(f"origin ({url}) isn't a GitHub repository")
    return match.group(1)


def token() -> str | None:
    for name in ("GH_TOKEN", "GITHUB_TOKEN"):
        if os.environ.get(name):
            return os.environ[name]
    if shutil.which("gh"):
        result = subprocess.run(["gh", "auth", "token"], capture_output=True, text=True)
        if result.returncode == 0 and result.stdout.strip():
            return result.stdout.strip()
    return None


def github_is_open(repo: str, auth: str | None) -> Callable[[int], bool]:
    """Asks GitHub's REST API whether a ticket is open. Raises OSError if it
    can't."""
    known: dict[int, bool] = {}

    def is_open(ticket: int) -> bool:
        if ticket not in known:
            request = urllib.request.Request(
                f"https://api.github.com/repos/{repo}/issues/{ticket}",
                headers={"Accept": "application/vnd.github+json"},
            )
            if auth:
                request.add_header("Authorization", f"Bearer {auth}")
            with urllib.request.urlopen(request, timeout=20) as response:
                known[ticket] = json.load(response)["state"] == "open"
        return known[ticket]

    return is_open


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--ref", help="read steps.rs at this commit instead of the working tree")
    parser.add_argument("--closing", default="", help="tickets the PR closes, comma-separated")
    arguments = parser.parse_args()
    if os.environ.get("DESKTOP_CALLS_OFFLINE") == "1":
        print("Skipped the Real-Phone Checklist ticket check: DESKTOP_CALLS_OFFLINE=1.")
        return 0
    if arguments.ref:
        text = subprocess.run(
            ["git", "show", f"{arguments.ref}:{STEPS}"], capture_output=True, text=True, check=True
        ).stdout
    else:
        with open(STEPS, encoding="utf-8") as file:
            text = file.read()
    found = steps(text)
    if len(found) < FEWEST_STEPS:
        print(f"Read only {len(found)} steps from {STEPS}; update tools/checklist_tickets.py to fit it.", file=sys.stderr)
        return 1
    closing = {int(n) for n in re.findall(r"\d+", arguments.closing)}
    try:
        wrong = problems(found, closing, github_is_open(repository(), token()))
    except (OSError, urllib.error.URLError, subprocess.CalledProcessError, KeyError, ValueError) as error:
        print(
            f"Couldn't ask GitHub whether the tickets in {STEPS}'s lands_with are open: {error}\n"
            "Offline, set DESKTOP_CALLS_OFFLINE=1 to skip this check.",
            file=sys.stderr,
        )
        return 1
    if wrong:
        print("Real-Phone Checklist steps wait on closed tickets (ticket #120):", file=sys.stderr)
        for line in wrong:
            print(f"  {line}", file=sys.stderr)
        print(
            f"Write the step's script in {STEPS} (its feature has landed), or name the open ticket it still waits on.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
