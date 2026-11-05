#!/usr/bin/env python3
"""No Real-Phone Checklist step may say it lands with a closed ticket.

A step without a script names, in `lands_with`, the open tickets whose
features it waits on, or runs together with another step (`RUNS_WITH`).
Once a ticket closes, the step must have its script, or name the open
ticket it still waits on; otherwise the runner tells the user "Not yet
runnable (lands with #11)" for a ticket long closed (ticket #120). This
check asks GitHub about every ticket named.

Usage:
  python3 tools/checklist_tickets.py             # `just check` runs this
  python3 tools/checklist_tickets.py --ref SHA --closing 22,16
      # the steps at SHA, and the tickets a PR closes

`just merge` runs the same rules in-process (tools/merge.py).

It fails when it can't reach GitHub, so a skipped check never looks like a
passed one. Offline, set DESKTOP_CALLS_OFFLINE=1 to skip it, knowingly: it
then says "Skipped". GitHub's token comes from GH_TOKEN or GITHUB_TOKEN,
else from `gh auth token`, else none: anonymous calls work on a public
repository, within GitHub's limit of 60 an hour.
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
OFFLINE = "DESKTOP_CALLS_OFFLINE"
SKIPPED = f"Skipped the Real-Phone Checklist ticket check: {OFFLINE}=1."


class Unreadable(ValueError):
    """steps.rs has a step this check can't read, so it would pass blind."""


class Unreachable(OSError):
    """GitHub couldn't be asked."""


@dataclass
class Step:
    id: str
    scripted: bool
    lands_with: list[int]


def steps(text: str) -> list[Step]:
    """Every `Step { ... }` in steps.rs, in order. Raises Unreadable for any
    step block without a readable id, script and lands_with."""
    found = []
    # A step literal, not the struct's own definition.
    for match in re.finditer(r"(?<!struct )\bStep \{", text):
        block = text[match.end() :].split("},", 1)[0]
        id_ = re.search(r'\bid:\s*"([^"]+)"', block)
        script = re.search(r"\bscript:\s*(None|Some\()", block)
        lands = re.search(r"\blands_with:\s*&\[([^\]]*)\]", block)
        if not (id_ and script and lands):
            line = text.count("\n", 0, match.start()) + 1
            raise Unreadable(f"{STEPS}:{line} has a step this check can't read; update tools/checklist_tickets.py to fit it.")
        tickets = [int(n) for n in re.findall(r"\d+", lands.group(1))]
        found.append(Step(id_.group(1), script.group(1) != "None", tickets))
    if not found:
        raise Unreadable(f"{STEPS} has no steps this check can read; update tools/checklist_tickets.py to fit it.")
    return found


def runs_with(text: str) -> dict[str, str]:
    """The RUNS_WITH table: step id to the step it runs together with."""
    table = re.search(r"RUNS_WITH: &\[\(&str, &str\)\] = &\[(.*?)\];", text, re.S)
    if not table:
        raise Unreadable(f"{STEPS} has no RUNS_WITH table this check can read.")
    return dict(re.findall(r'\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*\)', table.group(1)))


def problems(text: str, closing: set[int], is_open: Callable[[int], bool]) -> list[str]:
    """What is wrong with steps.rs's `text`, one sentence each."""
    found = steps(text)
    partners = runs_with(text)
    by_id = {step.id: step for step in found}
    wrong = []
    for step in found:
        partner = partners.get(step.id)
        if step.scripted and (step.lands_with or partner):
            wrong.append(f"Step {step.id} has a script, so it must neither land with a ticket nor run with another step.")
        if not step.scripted and not step.lands_with and not partner:
            wrong.append(f"Step {step.id} has no script, so it must name the open ticket it waits on, or the step it runs with.")
        if partner and not step.scripted:
            other = by_id.get(partner)
            if other is None:
                wrong.append(f"Step {step.id} runs with step {partner}, which doesn't exist.")
            elif other.scripted:
                wrong.append(f"Step {step.id} runs with step {partner}, which has its script, so {step.id} needs one too.")
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
    """Asks GitHub's REST API whether a ticket is open. Raises Unreachable
    if it can't."""
    known: dict[int, bool] = {}

    def is_open(ticket: int) -> bool:
        if ticket not in known:
            request = urllib.request.Request(
                f"https://api.github.com/repos/{repo}/issues/{ticket}",
                headers={"Accept": "application/vnd.github+json"},
            )
            if auth:
                request.add_header("Authorization", f"Bearer {auth}")
            try:
                with urllib.request.urlopen(request, timeout=20) as response:
                    known[ticket] = json.load(response)["state"] == "open"
            except (OSError, KeyError, ValueError) as error:
                raise Unreachable(f"Couldn't ask GitHub whether #{ticket} is open: {error}") from error
        return known[ticket]

    return is_open


def check(text: str, closing: set[int], is_open: Callable[[int], bool] | None = None) -> list[str]:
    """The problems in steps.rs's `text`, asking GitHub unless `is_open` is
    given. Raises Unreadable or Unreachable."""
    if is_open is None:
        try:
            is_open = github_is_open(repository(), token())
        except (OSError, subprocess.CalledProcessError) as error:
            raise Unreachable(f"Couldn't tell which GitHub repository to ask: {error}") from error
    return problems(text, closing, is_open)


def main(arguments: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--ref", help="read steps.rs at this commit instead of the working tree")
    parser.add_argument("--closing", default="", help="tickets the PR closes, comma-separated")
    options = parser.parse_args(arguments)
    if os.environ.get(OFFLINE) == "1":
        print(SKIPPED)
        return 0
    if options.ref:
        text = subprocess.run(
            ["git", "show", f"{options.ref}:{STEPS}"], capture_output=True, text=True, check=True
        ).stdout
    else:
        with open(STEPS, encoding="utf-8") as file:
            text = file.read()
    closing = {int(n) for n in re.findall(r"\d+", options.closing)}
    try:
        wrong = check(text, closing)
    except Unreadable as error:
        print(error, file=sys.stderr)
        return 1
    except Unreachable as error:
        print(f"{error}\nOffline, set {OFFLINE}=1 to skip this check.", file=sys.stderr)
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
