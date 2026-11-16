#!/usr/bin/env python3
"""Tests of the free-space preflight (tools/free_space.py, ticket #397) and
of where it runs. The disk is never asked: headroom and folder sizes are
stand-ins. `just lint` runs them."""

from __future__ import annotations

import contextlib
import io
import os
import pathlib
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import free_space  # noqa: E402

GB = free_space.GB
PLACE = free_space.Place("/home", pathlib.Path("/"), warn_below=30 * GB, refuse_below=10 * GB,
                         usual_suspects=("/nowhere/*",))
FOLDERS = [(6 * GB, "/work/a/target"), (5 * GB, "/work/b/target")]


def preflight(room: int) -> str:
    """Runs the preflight with `room` bytes free; returns what it warned."""
    err = io.StringIO()
    with mock.patch.dict(os.environ, {"GITHUB_ACTIONS": ""}), contextlib.redirect_stderr(err):
        free_space.preflight((PLACE,), room_of=lambda _: room, largest_of=lambda _: FOLDERS)
    return err.getvalue()


class Thresholds(unittest.TestCase):
    def test_with_room_to_spare_it_says_nothing(self):
        self.assertEqual(preflight(30 * GB), "")

    def test_under_the_warning_headroom_it_warns_naming_the_largest_folders(self):
        warned = preflight(29 * GB)
        self.assertIn("Warning: /home has 29.0 GB free for you", warned)
        self.assertIn("6.0 GB  /work/a/target", warned)
        self.assertIn("refuses under 10.0 GB", warned)

    def test_under_the_refusal_headroom_it_refuses_naming_the_largest_folders_and_what_to_do(self):
        with self.assertRaises(free_space.Refused) as refused:
            preflight(9 * GB)
        text = str(refused.exception)
        self.assertIn("/home has 9.0 GB free for you", text)
        self.assertIn("5.0 GB  /work/b/target", text)
        self.assertIn("Next: free space", text)

    def test_on_githubs_runners_it_does_nothing(self):
        with mock.patch.dict(os.environ, {"GITHUB_ACTIONS": "true"}):
            free_space.preflight((PLACE,), room_of=lambda _: 0, largest_of=lambda _: FOLDERS)

    def test_a_place_that_does_not_exist_is_skipped(self):
        missing = free_space.Place("/nowhere", pathlib.Path("/nowhere-at-all"), 1, 1, ())
        with mock.patch.dict(os.environ, {"GITHUB_ACTIONS": ""}):
            free_space.preflight((missing,), room_of=lambda _: 0, largest_of=lambda _: [])


class Measuring(unittest.TestCase):
    def test_headroom_is_the_smaller_of_free_space_and_quota_left(self):
        with mock.patch.object(free_space, "quota_left", return_value=2 * GB):
            self.assertLessEqual(free_space.headroom(pathlib.Path("/")), 2 * GB)
        with mock.patch.object(free_space, "quota_left", return_value=None):
            self.assertGreater(free_space.headroom(pathlib.Path("/")), 0)

    def test_largest_lists_matching_folders_biggest_first(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            for name, size in [("small", 1000), ("big", 50_000)]:
                (root / name).mkdir()
                (root / name / "file").write_bytes(b"x" * size)
            (root / "a-file").write_bytes(b"x")
            found = free_space.largest((f"{directory}/*",))
        self.assertEqual([pathlib.Path(folder).name for _, folder in found], ["big", "small"])
        self.assertGreaterEqual(found[0][0], 50_000)


class WhereItRuns(unittest.TestCase):
    """`just land`, `just ci-wait` and the harness's scenarios call it first.
    Every scenario recipe (`just test`, `just ci-scenarios`, `just test-file`,
    `just test-ci-like`) goes through scenario_shader_cache.warm."""

    def test_each_entry_point_calls_the_preflight(self):
        tools = Path(__file__).resolve().parent
        for name in ["land.py", "ci_wait.py", "scenario_shader_cache.py"]:
            self.assertIn("free_space.preflight()", (tools / name).read_text(), name)


if __name__ == "__main__":
    unittest.main()
