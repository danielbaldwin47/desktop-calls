#!/usr/bin/env python3
"""Wait on one GitHub CI run, and say why it failed without anyone reading its log.

GitHub runs CI only on pushes to main and on `gh workflow run CI`; a branch's
CI is `just ci-local` (ticket #158), which prints its own failures. So this
is for the run after a merge, and for a dispatched run.

Usage (through just):
  just ci-wait               # the latest run for the current branch
  just ci-wait 41981986246   # that run

Waits with `gh run watch --exit-status`. If the run passed, prints one line
and exits 0. Otherwise it prints, for each failed test, its name, where it
panicked and the panic message (which, for a harness wait, holds the event
timeline), and where its scenario's logs are; for a failed step with no
failed test, such as `just check`, the end of that step's log. Then it
downloads the run's scenario-log artifacts into target/ci/<run-id>/ and
exits 1.

The latest run for the current branch is the one for the commit checked out
here, if CI has one; right after a push this waits up to a minute for it to
appear. Otherwise it is the newest run for the branch, and a line says so.

Exit status: 0 the run passed; 1 it failed or was cancelled; 2 no run, or gh
failed.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass, field

EXIT_PASSED, EXIT_FAILED, EXIT_NO_RUN = 0, 1, 2

# The workflow in .github/workflows/ci.yml.
WORKFLOW = "CI"
# Artifacts that hold scenario directories, from either CI job.
SCENARIO_LOGS = "*scenario-logs*"
# Where the scenario directories live on the runner (see Harness::start).
RUNNER_SCENARIO = re.compile(r"/tmp/(desktop-calls-test-\w+)")
# How long a just-pushed commit may take to get its run.
RUN_APPEARS_SECONDS = 60
POLL_SECONDS = 5
# How a run that didn't pass ended, as gh names it, in words.
VERDICTS = {"failure": "failed", "cancelled": "was cancelled", "timed_out": "timed out"}
# How much of a failed step's log to show when no test failed in it.
STEP_TAIL_LINES = 20

# A `gh run view --log-failed` line: job, step, then an ISO timestamp.
LOG_LINE = re.compile(r"^([^\t]*)\t([^\t]*)\t(?:﻿)?\S+Z ?(.*)$")
# Colour codes, raw or as the log viewer escapes them.
ANSI = re.compile(r"(?:\x1b|\^\[)\[[0-9;]*m")


class Failed(Exception):
    """gh or git failed, or there is no run to wait on. The message says what
    to do next."""


def run(*args: str) -> str:
    result = subprocess.run(list(args), capture_output=True, text=True)
    if result.returncode != 0:
        raise Failed(f"`{' '.join(args)}` failed: {result.stderr.strip()}")
    return result.stdout


def latest_run() -> str:
    """The run for the commit checked out here, or else the newest run for
    the current branch."""
    branch = run("git", "branch", "--show-current").strip()
    if not branch:
        raise Failed("No branch is checked out. Next: just ci-wait <run-id>")
    head = run("git", "rev-parse", "HEAD").strip()
    give_up = time.monotonic() + RUN_APPEARS_SECONDS
    told = False
    while True:
        runs = json.loads(
            run(
                "gh", "run", "list", "--branch", branch, "--workflow", WORKFLOW,
                "--limit", "10", "--json", "databaseId,headSha",
            )
        )
        for found in runs:
            if found["headSha"] == head:
                return str(found["databaseId"])
        if time.monotonic() >= give_up:
            break
        if not told:
            print(f"Waiting for CI to start on {head[:7]}...", file=sys.stderr)
            told = True
        time.sleep(POLL_SECONDS)
    if not runs:
        raise Failed(
            f"GitHub has no CI run for {branch}: it runs CI only on pushes to main and "
            f"on `gh workflow run CI --ref {branch}`.\n"
            "Next: just ci-local (a branch's CI), or `gh workflow run CI` for a run here"
        )
    newest = str(runs[0]["databaseId"])
    print(
        f"CI has no run for {head[:7]}, the commit checked out here; "
        f"waiting on run {newest}, the newest for {branch}.",
        file=sys.stderr,
    )
    return newest


@dataclass
class TestFailure:
    job: str
    name: str
    # How to rerun it: `just test-file contacts some_scenario --exact` for a
    # harness scenario, else what cargo names, such as `-p automation --test gallery`.
    target: str = ""
    panics: list[list[str]] = field(default_factory=list)
    kept: list[str] = field(default_factory=list)


@dataclass
class StepLog:
    job: str
    step: str
    lines: list[str] = field(default_factory=list)


def failed_steps(run_id: str) -> list[StepLog]:
    """The log of every failed step, in order, without timestamps or colour."""
    steps: list[StepLog] = []
    for raw in run("gh", "run", "view", run_id, "--log-failed").splitlines():
        match = LOG_LINE.match(raw)
        if not match:
            continue
        job, step, line = match.groups()
        if not steps or (steps[-1].job, steps[-1].step) != (job, step):
            steps.append(StepLog(job, step))
        steps[-1].lines.append(ANSI.sub("", line))
    return steps


def harness_rerun(name: str, target: str) -> str:
    """How to rerun the test `name`, which cargo reran with `target`.

    The harness's scenarios are one binary, `scenarios`, whose test names are
    `<file>::<scenario>`; `just test-file` reruns one of them.
    """
    file, _, scenario = name.partition("::")
    if target.split() == ["-p", "test-harness", "--test", "scenarios"] and scenario:
        return f"just test-file {file} {scenario} --exact"
    return target


def test_failures(step: StepLog) -> list[TestFailure]:
    """Every failed test in a step that ran cargo test, with its panics."""
    failures: list[TestFailure] = []
    current: TestFailure | None = None
    panic: list[str] | None = None
    for line in step.lines:
        if heading := re.match(r"^---- (\S+) stdout ----$", line):
            current = TestFailure(step.job, heading.group(1))
            failures.append(current)
            panic = None
        elif line == "failures:" or line.startswith("test result:"):
            current, panic = None, None
        elif rerun := re.match(r"^error: test failed, to rerun pass `(.*)`$", line):
            for failure in failures:
                failure.target = failure.target or harness_rerun(failure.name, rerun.group(1))
        elif current is None:
            continue
        elif re.match(r"^thread '.*' (\(\d+\) )?panicked at ", line):
            panic = [line.split(" panicked at ", 1)[1].rstrip(":")]
            current.panics.append(panic)
        elif line.startswith("Kept this scenario's logs in "):
            if kept := RUNNER_SCENARIO.search(line):
                current.kept.append(kept.group(1))
            panic = None
        elif line.startswith("note: run with `RUST_BACKTRACE") or line == "stack backtrace:":
            panic = None
        elif panic is not None:
            panic.append(line)
            # A scenario that failed to start names its directory here.
            for scenario in RUNNER_SCENARIO.findall(line):
                if scenario not in current.kept:
                    current.kept.append(scenario)
    return failures


# cargo-nextest's report of a failed test: a heading such as
# `FAIL [   3.034s] (  1/220) test-harness::scenarios contacts::some_test`,
# then the test's indented `stdout ───` and `stderr ───`. The summary at the
# end repeats each heading without the output.
NEXTEST_FAIL = re.compile(r"^\s*(?:FAIL|TIMEOUT|SIGSEGV|SIGABRT|ABORT) \[[^]]*\] (?:\(\s*\d+/\d+\) )?(\S+) (\S+)\s*$")
# Any other nextest status line, or its summary, ends a failure's output.
NEXTEST_STATUS = re.compile(r"^\s*(?:PASS|FAIL|SKIP|SLOW|TIMEOUT|LEAK|RETRY|TRY \d+ \w+|Summary|Cancelling|Starting)\b")


def nextest_failures(step: StepLog) -> list[TestFailure]:
    """Every failed test in a step that ran cargo nextest, with its panics."""
    failures: dict[str, TestFailure] = {}
    current: TestFailure | None = None
    panic: list[str] | None = None
    for raw in step.lines:
        line = raw.strip()
        if heading := NEXTEST_FAIL.match(raw):
            name = heading.group(2)
            if name in failures:
                current, panic = None, None
                continue
            current = TestFailure(step.job, name)
            binary = heading.group(1).split("::", 1)[-1]
            current.target = harness_rerun(name, f"-p test-harness --test {binary}")
            failures[name] = current
            panic = None
        elif NEXTEST_STATUS.match(raw):
            current, panic = None, None
        elif current is None:
            continue
        elif re.match(r"^thread '.*' (\(\d+\) )?panicked at ", line):
            panic = [line.split(" panicked at ", 1)[1].rstrip(":")]
            current.panics.append(panic)
        elif line.startswith("Kept this scenario's logs in "):
            if kept := RUNNER_SCENARIO.search(line):
                current.kept.append(kept.group(1))
            panic = None
        elif line.startswith("note: run with `RUST_BACKTRACE") or line == "stack backtrace:":
            panic = None
        elif panic is not None:
            panic.append(line)
            for scenario in RUNNER_SCENARIO.findall(line):
                if scenario not in current.kept:
                    current.kept.append(scenario)
    return list(failures.values())


def report(failures: list[TestFailure], steps: list[StepLog], directory: pathlib.Path) -> None:
    for failure in failures:
        target = f" ({failure.target})" if failure.target else ""
        print(f"\nFAILED {failure.name}{target}, in \"{failure.job}\"")
        for location, *message in failure.panics:
            while message and not message[-1].strip():
                message.pop()
            print(f"  panicked at {location}")
            for line in message:
                print(f"    {line}")
        for scenario in failure.kept:
            found = sorted(directory.glob(f"*/{scenario}"))
            where = found[0] if found else f"{scenario}, which CI did not keep"
            print(f"  logs: {where}/" if found else f"  logs: {where}")
    tested = {failure.job for failure in failures}
    for step in steps:
        if step.job in tested:
            continue
        print(f"\nFAILED step \"{step.step}\", in \"{step.job}\"; its last lines:")
        lines, preamble = [], False
        for line in step.lines:
            # GitHub's echo of the step's command and environment.
            preamble = (preamble or line.startswith("##[group]")) and line != "##[endgroup]"
            if line.strip() and not preamble and line != "##[endgroup]":
                lines.append(line)
        for line in lines[-STEP_TAIL_LINES:]:
            print(f"    {line}")


def download_logs(run_id: str, directory: pathlib.Path) -> str:
    """Downloads the run's scenario logs, and returns the line saying so."""
    if directory.exists():
        shutil.rmtree(directory)
    result = subprocess.run(
        ["gh", "run", "download", run_id, "--dir", str(directory), "--pattern", SCENARIO_LOGS],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        if "no artifact matches" in result.stderr or "no valid artifacts" in result.stderr:
            return "CI kept no scenario logs for this run."
        raise Failed(f"`gh run download {run_id}` failed: {result.stderr.strip()}")
    scenarios = sorted(directory.glob("*/desktop-calls-test-*"))
    noun = "directory" if len(scenarios) == 1 else "directories"
    return f"Downloaded {len(scenarios)} scenario {noun} into {directory}/"


def main() -> int:
    parser = argparse.ArgumentParser(
        prog="just ci-wait",
        description=__doc__.split("\n\n", 1)[0],
        epilog="See tools/ci_wait.py for the details.",
    )
    parser.add_argument("run_id", nargs="?", help="the run (default: the current branch's latest)")
    arguments = parser.parse_args()
    try:
        run_id = arguments.run_id or latest_run()
        # The live view goes nowhere: the verdict below is all anyone reads.
        subprocess.run(
            ["gh", "run", "watch", run_id, "--exit-status", "--interval", "10"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        summary = json.loads(
            run("gh", "run", "view", run_id, "--json", "conclusion,url,headBranch,jobs")
        )
        conclusion, url = summary["conclusion"], summary["url"]
        if conclusion == "success":
            print(f"CI passed: run {run_id} on {summary['headBranch']}, {url}")
            return EXIT_PASSED
        failed_jobs = [job["name"] for job in summary["jobs"] if job["conclusion"] == "failure"]
        verdict = VERDICTS.get(conclusion, conclusion or "did not finish")
        print(f"CI {verdict}: run {run_id} on {summary['headBranch']}, {url}")
        if failed_jobs:
            print("Failed jobs: " + "; ".join(failed_jobs))
        steps = failed_steps(run_id)
        failures = [
            failure
            for step in steps
            for failure in (*test_failures(step), *nextest_failures(step))
        ]
        # "CI passed" fails whenever another job did; it says nothing new.
        steps = [step for step in steps if step.job != "CI passed" or len(failed_jobs) == 1]
        directory = pathlib.Path("target/ci") / run_id
        downloaded = download_logs(run_id, directory)
        report(failures, steps, directory)
        print(f"\n{downloaded}")
        return EXIT_FAILED
    except Failed as failed:
        print(f"just ci-wait: {failed}", file=sys.stderr)
        return EXIT_NO_RUN


if __name__ == "__main__":
    sys.exit(main())
