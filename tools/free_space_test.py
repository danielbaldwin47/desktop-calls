#!/usr/bin/env python3
"""Tests of the free-space preflight (tools/free_space.py, ticket #397) and
of where it runs. The disk is never asked: headroom and folder sizes are
stand-ins. `just lint` runs them."""

from __future__ import annotations

import contextlib
import ctypes
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
                         usual_suspects=("/nowhere/*",), next_step="clear /work")
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
        missing = free_space.Place("/nowhere", pathlib.Path("/nowhere-at-all"), 1, 1, (), "")
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
    """What each entry point does on a nearly full disk. Every scenario
    recipe (`just test`, `just ci-scenarios`, `just test-file`) goes through
    scenario_shader_cache.warm; `just test-ci-like` calls it before building."""

    def refusing(self):
        return mock.patch.object(free_space, "preflight", side_effect=free_space.Refused("/tmp has 0.5 GB free"))

    def test_just_land_refuses_before_the_lane(self):
        import land
        pr = {"number": 1}
        with self.refusing(), mock.patch.object(land.merge, "pull_request", return_value=pr), \
                mock.patch.object(land.merge, "require_open"), mock.patch.object(land, "require_here"), \
                mock.patch.object(land.ci_local, "require_clean_head"), \
                mock.patch.object(land.merge, "take_landing_turn") as lane, \
                mock.patch.object(sys, "argv", ["land.py", "1"]), contextlib.redirect_stderr(io.StringIO()) as err:
            self.assertEqual(land.main(), land.EXIT_REFUSED)
        lane.assert_not_called()
        self.assertIn("Not merged: /tmp has 0.5 GB free", err.getvalue())

    def test_scenarios_refuse_before_the_warm_up(self):
        import scenario_shader_cache
        run = mock.Mock()
        with self.refusing(), self.assertRaises(free_space.Refused):
            scenario_shader_cache.warm({}, run=run)
        run.assert_not_called()

    def test_just_ci_wait_only_warns(self):
        import ci_wait
        with self.refusing(), mock.patch.object(ci_wait, "latest_run", side_effect=KeyboardInterrupt), \
                mock.patch.object(sys, "argv", ["ci_wait.py"]), contextlib.redirect_stderr(io.StringIO()) as err:
            with self.assertRaises(KeyboardInterrupt):
                ci_wait.main()
        self.assertIn("Warning: /tmp has 0.5 GB free", err.getvalue())


class Quota(unittest.TestCase):
    def test_the_quota_struct_is_the_kernels_if_dqblk(self):
        self.assertEqual(ctypes.sizeof(free_space._Quota), 72)

    def test_quota_left_is_the_hard_limit_less_what_is_used(self):
        def syscall(number, descriptor, command, uid, pointer):
            quota = ctypes.cast(pointer, ctypes.POINTER(free_space._Quota)).contents
            quota.hard, quota.used = 2_000_000, 500_000_000  # 1 KiB blocks; bytes
            return 0
        with mock.patch.object(free_space.ctypes, "CDLL", return_value=mock.Mock(syscall=syscall)), \
                mock.patch.object(free_space.ctypes, "byref", side_effect=ctypes.pointer):
            self.assertEqual(free_space.quota_left(Path("/")), 2_048_000_000 - 500_000_000)

    def test_no_quota_or_a_refusing_kernel_means_none(self):
        for status, hard in [(-1, 5), (0, 0)]:
            def syscall(number, descriptor, command, uid, pointer, status=status, hard=hard):
                ctypes.cast(pointer, ctypes.POINTER(free_space._Quota)).contents.hard = hard
                return status
            with mock.patch.object(free_space.ctypes, "CDLL", return_value=mock.Mock(syscall=syscall)), \
                    mock.patch.object(free_space.ctypes, "byref", side_effect=ctypes.pointer):
                self.assertIsNone(free_space.quota_left(Path("/")))


if __name__ == "__main__":
    unittest.main()
