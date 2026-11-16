"""Tests for scenario_shader_cache.py: when the scenarios' shared shader
cache is filled, and what the scenarios are then given (#298)."""

import contextlib
import io
import subprocess
import unittest
from unittest import mock

import scenario_shader_cache as cache


class Warming(unittest.TestCase):
    def warm(self, environment: dict[str, str], status: int = 0):
        ran = []

        def run(command, env):
            ran.append((command, env.get(cache.VARIABLE)))
            return subprocess.CompletedProcess(command, status)

        with mock.patch.object(cache.pathlib.Path, "mkdir"), mock.patch.object(cache.free_space, "preflight"), \
                contextlib.redirect_stderr(io.StringIO()):
            return cache.warm(environment, run=run), ran

    def test_unnamed_it_fills_the_shared_cache_first_and_gives_it_to_every_scenario(self):
        environment, ran = self.warm({"PATH": "/bin"})
        self.assertEqual(ran, [(cache.WARM_UP, str(cache.CACHE))])
        self.assertEqual(environment, {"PATH": "/bin", cache.VARIABLE: str(cache.CACHE)})

    def test_a_cache_the_caller_names_is_kept_and_not_filled(self):
        environment, ran = self.warm({cache.VARIABLE: "/elsewhere"})
        self.assertEqual(ran, [])
        self.assertEqual(environment, {cache.VARIABLE: "/elsewhere"})

    def test_set_but_empty_keeps_every_scenario_cold(self):
        # For timing cold first frames by hand.
        environment, ran = self.warm({cache.VARIABLE: ""})
        self.assertEqual(ran, [])
        self.assertEqual(environment, {cache.VARIABLE: ""})

    def test_a_failed_warm_up_runs_no_scenario_and_says_how_to_see_why(self):
        with self.assertRaises(cache.WarmUpFailed) as failed:
            self.warm({}, status=101)
        self.assertIn("Next:", str(failed.exception))
        self.assertIn(cache.WARM_UP_SCENARIO, str(failed.exception))


if __name__ == "__main__":
    unittest.main()
