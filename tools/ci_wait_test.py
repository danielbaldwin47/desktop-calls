#!/usr/bin/env python3
"""Tests of ci_wait's log parsers. `just lint` runs them.

The nextest log is the shape of a real failure from CI (run 42034428129 on
the old repo), with the harness compiled as one binary, `scenarios`.
"""

from __future__ import annotations

import unittest

from ci_wait import StepLog, nextest_failures, test_failures

SCENARIO = "answered_on_phone_surfaces::the_caller_hanging_up_while_ringing_closes_the_banner_at_once"

NEXTEST_LOG = f"""\
        PASS [   2.101s] (  1/220) test-harness::scenarios contacts::a_contact_names_the_caller
        FAIL [   3.034s] (  2/220) test-harness::scenarios {SCENARIO}
  stdout ───

    running 1 test
    test {SCENARIO} ... FAILED

    test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 222 filtered out; finished in 3.02s

  stderr ───

    thread '{SCENARIO}' (20397) panicked at crates/test-harness/src/control.rs:292:9:
    the app took 1.032816s for the Banner to show once the Call appeared, against 1s
    note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
    Kept this scenario's logs in /tmp/desktop-calls-test-LR23j9

        PASS [   1.500s] (  3/220) test-harness::scenarios dialer::typing_a_number_dials_it
────────────
     Summary [  60.000s] 220 tests run: 219 passed, 1 failed, 0 skipped
        FAIL [   3.034s] (  2/220) test-harness::scenarios {SCENARIO}
error: test run failed
"""

CARGO_TEST_LOG = f"""\
running 2 tests
test dialer_surface::with_5000_contacts_the_dialer_shows_and_follows_each_keystroke_promptly ... ok
test incoming_call_surfaces::the_first_banner_shows_without_holding_up_the_main_loop ... FAILED

failures:

---- incoming_call_surfaces::the_first_banner_shows_without_holding_up_the_main_loop stdout ----

thread 'incoming_call_surfaces::the_first_banner_shows_without_holding_up_the_main_loop' (311) panicked at crates/test-harness/src/control.rs:300:9:
the first Banner took 412ms, against 300ms
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
Kept this scenario's logs in /tmp/desktop-calls-test-Ab12Cd

failures:
    incoming_call_surfaces::the_first_banner_shows_without_holding_up_the_main_loop

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 221 filtered out; finished in 4.20s

error: test failed, to rerun pass `-p test-harness --test scenarios`
"""


def step(log: str) -> StepLog:
    return StepLog("shard 1 of 4", "Run the shard", log.splitlines())


class NextestFailures(unittest.TestCase):
    def test_a_failed_scenario_comes_back_once_with_its_panic_logs_and_rerun(self) -> None:
        [failure] = nextest_failures(step(NEXTEST_LOG))
        self.assertEqual(failure.name, SCENARIO)
        self.assertEqual(
            failure.target,
            "just test-file answered_on_phone_surfaces "
            "the_caller_hanging_up_while_ringing_closes_the_banner_at_once --exact",
        )
        [panic] = failure.panics
        self.assertEqual(panic[0], "crates/test-harness/src/control.rs:292:9")
        self.assertIn("the app took 1.032816s for the Banner to show", panic[1])
        self.assertEqual(failure.kept, ["desktop-calls-test-LR23j9"])

    def test_a_run_where_every_scenario_passed_has_no_failures(self) -> None:
        passed = "\n".join(line for line in NEXTEST_LOG.splitlines() if "PASS" in line)
        self.assertEqual(nextest_failures(step(passed)), [])


class CargoTestFailures(unittest.TestCase):
    def test_a_failed_harness_scenario_reruns_through_just_test_file(self) -> None:
        [failure] = test_failures(step(CARGO_TEST_LOG))
        self.assertEqual(failure.name, "incoming_call_surfaces::the_first_banner_shows_without_holding_up_the_main_loop")
        self.assertEqual(
            failure.target,
            "just test-file incoming_call_surfaces the_first_banner_shows_without_holding_up_the_main_loop --exact",
        )
        self.assertEqual(failure.panics[0][0], "crates/test-harness/src/control.rs:300:9")
        self.assertEqual(failure.kept, ["desktop-calls-test-Ab12Cd"])

    def test_another_crates_failure_keeps_cargos_rerun(self) -> None:
        log = CARGO_TEST_LOG.replace("-p test-harness --test scenarios", "-p automation --test gallery")
        [failure] = test_failures(step(log))
        self.assertEqual(failure.target, "-p automation --test gallery")


if __name__ == "__main__":
    unittest.main()
