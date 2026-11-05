#!/usr/bin/env python3
"""Run one of the harness's test files, or some of its scenarios, without
knowing how the harness's tests are compiled.

Usage (through just):
  just test-file contacts                     # every scenario in tests/contacts.rs
  just test-file contacts with_5000           # those whose names contain with_5000
  just test-file contacts some_scenario --exact   # only the one named some_scenario
  just test-file dialer_surface --ignored --test-threads=1
  just test-file incoming_call_surfaces the_first_banner --ignored --ci-like

NAME is a file in crates/test-harness/tests/, without `.rs`. FILTER, if
given, picks the scenarios whose names contain it, or with `--exact`, the
one named it. `--ignored` picks from the ignored scenarios only, and
`--include-ignored` from all of them. Other arguments go to the test binary
(`--test-threads=1`, `--nocapture`, ...). `--ci-like` runs them under CI's
limits, through `just test-ci-like`.

The harness's test files are modules of one test binary, `scenarios`
(crates/test-harness/build.rs, #118), so a scenario's full name is
`<file>::<scenario>`. This picks the matching names and runs exactly those.

Exit status: that of the tests; 2 if NAME or FILTER matches no scenario
that would run, so a filter gone stale fails rather than runs nothing.
"""

from __future__ import annotations

import pathlib
import subprocess
import sys

BINARY = ["-p", "test-harness", "--test", "scenarios"]
TESTS = pathlib.Path(__file__).resolve().parent.parent / "crates/test-harness/tests"
# Each picks which scenarios the test binary would run, so which to list.
WHICH = ("--ignored", "--include-ignored")


def scenario_names(which: list[str]) -> list[str]:
    """The full name of every scenario the binary would run given `which`."""
    listed = subprocess.run(
        ["cargo", "test", "--locked", "--quiet", *BINARY, "--", "--list", "--format", "terse", *which],
        capture_output=True,
        text=True,
    )
    if listed.returncode != 0:
        sys.stderr.write(listed.stderr)
        sys.exit(listed.returncode)
    return [line.removesuffix(": test") for line in listed.stdout.splitlines() if line.endswith(": test")]


def main() -> int:
    arguments = sys.argv[1:]
    if not arguments or arguments[0] in ("-h", "--help"):
        print(__doc__)
        return 0 if arguments else 2
    name, rest = arguments[0], arguments[1:]
    test_filter = rest.pop(0) if rest and not rest[0].startswith("-") else ""
    ci_like = "--ci-like" in rest
    exact = "--exact" in rest
    rest = [argument for argument in rest if argument not in ("--ci-like", "--exact")]
    which = [argument for argument in rest if argument in WHICH]

    files = sorted(path.stem for path in TESTS.glob("*.rs") if path.name != "scenarios.rs")
    if name not in files:
        print(f"The harness has no test file {name!r}. Its files: {', '.join(files)}.", file=sys.stderr)
        return 2

    # Scenarios run the built binaries, as in `just test`.
    if subprocess.run(["cargo", "build", "--workspace", "--bins", "--locked"]).returncode != 0:
        return 1
    prefix = f"{name}::"
    chosen = [
        full for full in scenario_names(which)
        if full.startswith(prefix)
        and (full.removeprefix(prefix) == test_filter if exact else test_filter in full.removeprefix(prefix))
    ]
    if not chosen:
        kind = {"--ignored": "ignored scenario", "--include-ignored": "scenario"}.get(
            next(iter(which), ""), "scenario that runs without --ignored"
        )
        named = f" named {test_filter!r}" if exact else f" with {test_filter!r} in its name" if test_filter else ""
        print(f"No {kind} in {name}{named}.", file=sys.stderr)
        return 2

    test_arguments = [*BINARY, "--", "--exact", *chosen, *rest]
    if ci_like:
        command = [sys.executable, str(pathlib.Path(__file__).with_name("test_ci_like.py")), *test_arguments]
    else:
        command = ["cargo", "test", "--locked", *test_arguments]
    return subprocess.run(command).returncode


if __name__ == "__main__":
    sys.exit(main())
