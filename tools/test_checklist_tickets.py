#!/usr/bin/env python3
"""Tests for the closed-ticket rule (tools/checklist_tickets.py) and the
`just merge` gate that runs it (tools/merge.py). `just check` runs them.
GitHub is never asked: a stand-in says which tickets are open."""

from __future__ import annotations

import contextlib
import io
import os
import sys
import unittest
from pathlib import Path
from unittest import mock

TOOLS = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOLS))

import checklist_tickets  # noqa: E402
import merge  # noqa: E402

OPEN = {16, 22, 148}


def is_open(ticket: int) -> bool:
    return ticket in OPEN


def step(id_: str, script: str, lands_with: str) -> str:
    return f"""
            Step {{
                id: "{id_}",
                title: "A step",
                script: {script},
                lands_with: &[{lands_with}],
            }},"""


def steps_rs(*blocks: str, runs_with: str = "") -> str:
    return (
        "pub struct Step {\n    pub id: &'static str,\n}\n"
        f"pub const RUNS_WITH: &[(&str, &str)] = &[{runs_with}];\n"
        "pub const GROUPS: &[Group] = &[Group { steps: &[" + "".join(blocks) + "] }];\n"
    )


class Rules(unittest.TestCase):
    def test_the_real_steps_are_all_read_and_none_waits_on_a_closed_ticket(self):
        text = (TOOLS.parent / checklist_tickets.STEPS).read_text(encoding="utf-8")
        self.assertEqual(len(checklist_tickets.steps(text)), 35)
        self.assertEqual(checklist_tickets.problems(text, set(), is_open), [])

    def test_a_step_without_a_script_may_not_land_with_a_closed_ticket(self):
        text = steps_rs(step("7.1", "None", "10, 16"))
        self.assertEqual(
            checklist_tickets.problems(text, set(), is_open),
            ["Step 7.1 has no script and lands with #10, which is closed."],
        )

    def test_nor_with_a_ticket_the_pr_closes(self):
        text = steps_rs(step("1.2", "None", "22"))
        self.assertEqual(
            checklist_tickets.problems(text, {22}, is_open),
            ["Step 1.2 has no script and lands with #22, which this PR closes."],
        )

    def test_a_step_without_a_script_must_say_what_it_waits_on(self):
        text = steps_rs(step("6.1", "None", ""))
        self.assertIn("must name the open ticket", checklist_tickets.problems(text, set(), is_open)[0])

    def test_a_step_with_a_script_lands_with_nothing(self):
        text = steps_rs(step("3.1", "Some(handoff_twice)", "16"))
        self.assertIn("has a script", checklist_tickets.problems(text, set(), is_open)[0])

    def test_a_step_may_run_with_another_until_that_one_has_its_script(self):
        waiting = steps_rs(step("1.1", "None", ""), step("1.2", "None", "22"), runs_with='("1.1", "1.2")')
        self.assertEqual(checklist_tickets.problems(waiting, set(), is_open), [])
        landed = steps_rs(step("1.1", "None", ""), step("1.2", "Some(pair)", ""), runs_with='("1.1", "1.2")')
        self.assertEqual(
            checklist_tickets.problems(landed, set(), is_open),
            ["Step 1.1 runs with step 1.2, which has its script, so 1.1 needs one too."],
        )

    def test_a_step_the_parser_cant_read_fails_the_check(self):
        text = steps_rs(step("2.1", "None", "16"), '\n            Step {\n                id: "2.2",\n                script: pick(),\n            },')
        with self.assertRaises(checklist_tickets.Unreadable):
            checklist_tickets.steps(text)


class Command(unittest.TestCase):
    def run_main(self, arguments, environment):
        out, err = io.StringIO(), io.StringIO()
        with mock.patch.dict(os.environ, environment), contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = checklist_tickets.main(arguments)
        return code, out.getvalue(), err.getvalue()

    def test_offline_it_says_skipped(self):
        code, out, _ = self.run_main([], {checklist_tickets.OFFLINE: "1"})
        self.assertEqual((code, out.strip()), (0, checklist_tickets.SKIPPED))

    def test_github_unreachable_fails_and_says_how_to_skip(self):
        def unreachable(_repo, _token):
            def ask(ticket):
                raise checklist_tickets.Unreachable(f"Couldn't ask GitHub whether #{ticket} is open: no network")

            return ask

        with mock.patch.object(checklist_tickets, "github_is_open", unreachable), mock.patch.object(
            checklist_tickets, "repository", lambda: "owner/repo"
        ):
            code, _, err = self.run_main([], {checklist_tickets.OFFLINE: "0"})
        self.assertEqual(code, 1)
        self.assertIn("no network", err)
        self.assertIn(f"{checklist_tickets.OFFLINE}=1", err)


class MergeGate(unittest.TestCase):
    PR = {"number": 7, "headRefOid": "abc1234"}

    def gate(self, text, closing, ask=is_open, environment=None):
        def gh(*arguments):
            self.assertEqual(arguments[:2], ("pr", "view"))
            return '{"closingIssuesReferences": [%s]}' % ", ".join(f'{{"number": {n}}}' for n in closing)

        def git(*arguments, **_):
            return text if arguments[0] == "show" else ""

        out = io.StringIO()
        with mock.patch.object(merge, "gh", gh), mock.patch.object(merge, "git", git), mock.patch.dict(
            os.environ, environment or {checklist_tickets.OFFLINE: "0"}
        ), contextlib.redirect_stdout(out):
            merge.require_no_step_waits_on_a_closed_ticket(self.PR, ask)
        return out.getvalue()

    def test_it_refuses_a_pr_that_closes_a_ticket_a_step_still_waits_on(self):
        with self.assertRaises(merge.Refused) as refused:
            self.gate(steps_rs(step("1.2", "None", "22")), [22])
        self.assertIn("lands with #22, which this PR closes", str(refused.exception))
        self.assertIn("Next: write those steps' scripts", str(refused.exception))

    def test_it_lets_through_a_pr_whose_steps_wait_only_on_open_tickets(self):
        self.assertEqual(self.gate(steps_rs(step("1.2", "None", "22")), [120]), "")

    def test_offline_it_says_skipped(self):
        printed = self.gate("", [], environment={checklist_tickets.OFFLINE: "1"})
        self.assertEqual(printed.strip(), checklist_tickets.SKIPPED)

    def test_github_unreachable_has_its_own_next_step(self):
        def unreachable(ticket):
            raise checklist_tickets.Unreachable(f"Couldn't ask GitHub whether #{ticket} is open: timed out")

        with self.assertRaises(merge.Refused) as refused:
            self.gate(steps_rs(step("1.2", "None", "22")), [], ask=unreachable)
        self.assertIn("Next: check the network and `gh auth status`", str(refused.exception))

    def test_just_merge_runs_the_gate_before_it_merges(self):
        pr = {**self.PR, "headRefName": "ticket/22", "baseRefName": "main"}
        asked = []

        def gh(*arguments):
            asked.append(arguments[:2])
            return "{}"

        def gate(checked, *_):
            self.assertEqual(checked, pr)
            raise merge.Refused("a step waits on #22")

        with contextlib.ExitStack() as stack:
            for name, stand_in in {
                "pull_request": lambda number: pr,
                "require_open": lambda pr: None,
                "require_contains_base": lambda pr: None,
                "wait_for_checks": lambda pr, seconds: None,
                "require_checks_passed": lambda pr, minutes: None,
                "require_mergeable": lambda pr: pr,
                "require_no_step_waits_on_a_closed_ticket": gate,
                "gh": gh,
            }.items():
                stack.enter_context(mock.patch.object(merge, name, stand_in))
            with self.assertRaises(merge.Refused) as refused:
                merge.merge(7, 1)
        self.assertIn("a step waits on #22", str(refused.exception))
        self.assertNotIn(("pr", "merge"), asked)


if __name__ == "__main__":
    unittest.main()
