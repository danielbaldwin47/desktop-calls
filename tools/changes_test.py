#!/usr/bin/env python3
"""Tests of the docs-only rule (tools/changes.py, ticket #239). `just lint` runs them."""

from __future__ import annotations

import pathlib
import re
import unittest

import changes

REPOSITORY = pathlib.Path(__file__).resolve().parent.parent
TOOLS = REPOSITORY / "tools"


def git_stub(files: list[str], named: str = ""):
    """A `git` that lists `files` as changed and, if `named`, has one file
    outside docs/ and *.md name it."""

    def git(*args: str, allowed: tuple[int, ...] = (0,)) -> str:
        if args[0] == "diff":
            return "\n".join(files)
        if args[0] == "grep":
            return f"{args[args.index('--') - 1]}:tools/some_tool.py:{named}\n" if named else ""
        raise AssertionError(f"unexpected git {args}")

    return git


class TheRule(unittest.TestCase):
    def test_docs_that_no_code_names_are_not_code(self):
        code, reason = changes.code_changes(git_stub(["docs/fixture/page.md", "notes.md"]), "base", "head")
        self.assertFalse(code, reason)

    def test_docs_that_a_file_outside_docs_names_are_code(self):
        code, reason = changes.code_changes(
            git_stub(["docs/fixture/page.md"], named="docs/fixture/page.md"), "base", "head"
        )
        self.assertTrue(code)
        self.assertIn("tools/some_tool.py names it", reason)

    def test_a_changed_file_outside_docs_is_code(self):
        code, _ = changes.code_changes(git_stub(["docs/fixture/page.md", "justfile"]), "base", "head")
        self.assertTrue(code)


class TheToolsTestsDoNotNameTheDocs(unittest.TestCase):
    """A tools test that names a real docs page as a fixture makes every
    change to that page count as code: a wiki page named in
    tools/merge_test.py did, so every wiki PR ran the full suite. Tests only:
    a probe under tools/real-phone that cites the research note it came from
    is the rule working as written ("comments count too")."""

    def test_no_tools_test_names_a_docs_path_the_repository_holds(self):
        references = [re.compile(pattern) for pattern in changes.DOCS_REFERENCE]
        found = []
        tests = sorted([*TOOLS.glob("*_test.py"), *TOOLS.glob("test_*.py")])
        for path in tests:
            relative = path.relative_to(REPOSITORY).as_posix()
            text = path.read_text()
            for reference in references:
                for name in reference.findall(text):
                    if (REPOSITORY / name.lstrip("./")).exists():
                        found.append(f"{relative} names {name}")
        self.assertEqual(
            found, [], "Name a path that does not exist (docs/fixture/page.md), so no change to a real page counts as code"
        )


if __name__ == "__main__":
    unittest.main()
