#!/usr/bin/env python3
"""Run one of the harness's test files, or some of its scenarios, without
knowing how the harness's tests are compiled.

Usage (through just):
  just test-file contacts                     # every scenario in tests/contacts.rs
  just test-file contacts with_5000           # those whose names contain with_5000
  just test-file dialer_surface --ignored --test-threads=1
  just test-file incoming_call_surfaces the_first_banner --ignored --ci-like

NAME is a file in crates/test-harness/tests/, without `.rs`. FILTER, if
given, picks the scenarios whose names contain it. Other arguments go to
the test binary (`--ignored`, `--test-threads=1`, `--nocapture`, ...).
`--ci-like` runs them under CI's limits, through `just test-ci-like`.

The harness's test files are modules of one test binary, `scenarios`
(crates/test-harness/build.rs, #118), so a scenario's full name is
`<file>::<scenario>`. This picks the matching names and runs exactly those.

Exit status: that of the tests; 2 if NAME or FILTER matches nothing.
"""

from __future__ import annotations

import pathlib
import subprocess
import sys

BINARY = ["-p", "test-harness", "--test", "scenarios"]
TESTS = pathlib.Path(__file__).resolve().parent.parent / "crates/test-harness/tests"


def scenario_names() -> list[str]:
    """Every scenario's full name, ignored ones too."""
    listed = subprocess.run(
        ["cargo", "test", "--locked", "--quiet", *BINARY, "--", "--list", "--format", "terse", "--include-ignored"],
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
    rest = [argument for argument in rest if argument != "--ci-like"]

    files = sorted(path.stem for path in TESTS.glob("*.rs") if path.name != "scenarios.rs")
    if name not in files:
        print(f"The harness has no test file {name!r}. Its files: {', '.join(files)}.", file=sys.stderr)
        return 2

    # Scenarios run the built binaries, as in `just test`.
    if subprocess.run(["cargo", "build", "--workspace", "--bins", "--locked"]).returncode != 0:
        return 1
    chosen = [
        full for full in scenario_names()
        if full.startswith(f"{name}::") and test_filter in full.removeprefix(f"{name}::")
    ]
    if not chosen:
        print(f"No scenario in {name} has {test_filter!r} in its name.", file=sys.stderr)
        return 2

    test_arguments = [*BINARY, "--", "--exact", *chosen, *rest]
    if ci_like:
        command = [sys.executable, str(pathlib.Path(__file__).with_name("test_ci_like.py")), *test_arguments]
    else:
        command = ["cargo", "test", "--locked", *test_arguments]
    return subprocess.run(command).returncode


if __name__ == "__main__":
    sys.exit(main())
