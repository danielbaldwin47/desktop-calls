"""Tests for tools/unknowns.py: `python3 -m unittest discover -s tools -p '*_test.py'`."""

import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import unknowns  # noqa: E402

STEPS = 'Step { id: "1.2", needs: &[], }\nStep { id: "1.3", needs: &[], }\nStep { id: "6.1", needs: &[], }\n'


def entry(number, status="open", observed_by="1.3", title="A thing the Pixel does"):
    return (
        f"## RPU-{number}: {title}\n"
        f"- Assumed: the Pixel does it.\n"
        f"- Source: a note.\n"
        f"- Observed by: {observed_by}\n"
        f"- Status: {status}\n\n"
    )


class Check(unittest.TestCase):
    def check(self, ledger, files=None):
        return unknowns.check(ledger, STEPS, files or {})

    def test_a_cited_open_entry_passes(self):
        files = {"pairing.rs": "//! The times are inferred (RPU-1).\nfn x() {}\n"}
        self.assertEqual(self.check(entry(1), files), [])

    def test_inferred_with_no_id_fails_and_names_the_line(self):
        files = {"pairing.rs": "fn x() {}\n/// Its path is inferred.\nfn y() {}\n"}
        problems = self.check(entry(1), files)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("pairing.rs:2", problems[0])
        self.assertIn("no ledger id", problems[0])

    def test_unverified_counts_too_and_case_does_not_matter(self):
        files = {"steps/x.rs": "//! Each of the Unverified points.\n"}
        self.assertEqual(len(self.check(entry(1), files)), 1)

    def test_an_id_far_up_the_same_block_does_not_cover_it(self):
        files = {"a.rs": "//! Pairing (RPU-1).\n//!\n//! a\n//! b\n//! The path is inferred.\n"}
        problems = self.check(entry(1), files)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("a.rs:5", problems[0])

    def test_an_id_within_two_lines_covers_it(self):
        files = {"a.rs": "/// The link drops, which is inferred,\n/// not observed (RPU-1).\nfn x() {}\n"}
        self.assertEqual(self.check(entry(1), files), [])

    def test_an_id_two_lines_above_covers_it(self):
        files = {"a.rs": "/// Pairing (RPU-1):\n/// a\n/// the path is inferred.\n"}
        self.assertEqual(self.check(entry(1), files), [])

    def test_an_id_three_lines_away_does_not(self):
        files = {"a.rs": "/// The path is inferred.\n/// a\n/// b\n/// RPU-1\n"}
        problems = self.check(entry(1), files)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("a.rs:1", problems[0])

    def test_inner_doc_lines_make_one_block(self):
        files = {"a.rs": "//! The unverified points\n//! in the notes (RPU-1).\n"}
        self.assertEqual(self.check(entry(1), files), [])

    def test_an_id_in_another_block_does_not(self):
        files = {"a.rs": "/// Inferred.\nfn x() {}\n/// RPU-1\nfn y() {}\n"}
        self.assertEqual(len(self.check(entry(1), files)), 1)

    def test_a_trailing_comment_is_its_own_block(self):
        files = {"a.rs": "const X: u8 = 1; // inferred\n"}
        self.assertEqual(len(self.check(entry(1), files)), 1)

    def test_inferred_outside_comments_is_left_alone(self):
        files = {"a.rs": 'const BOX: &str = "inferred";\n'}
        self.assertEqual(self.check(entry(1), files), [])

    def test_an_unknown_id_fails(self):
        files = {"a.rs": "/// Inferred (RPU-9).\n"}
        problems = self.check(entry(1), files)
        self.assertTrue(any("RPU-9 isn't in" in problem for problem in problems), problems)

    def test_code_calling_an_observed_entry_inferred_fails(self):
        ledger = entry(1, status="observed 2026-11-15 (#392): refused at once, no prompt")
        files = {"a.rs": "/// The prompt lasts 30 s (inferred, RPU-1).\n"}
        problems = self.check(ledger, files)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("observed 2026-11-15", problems[0])

    def test_code_citing_an_observed_entry_without_the_word_passes(self):
        ledger = entry(1, status="observed 2026-11-15 (#392): refused at once")
        files = {"a.rs": "/// The Pixel refuses at once (RPU-1).\n"}
        self.assertEqual(self.check(ledger, files), [])

    def test_a_step_the_checklist_lacks_fails(self):
        problems = self.check(entry(1, observed_by="1.3, 9.9"))
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("'9.9'", problems[0])

    def test_no_step_yet_is_allowed_with_why(self):
        self.assertEqual(self.check(entry(1, observed_by="no step yet")), [])
        self.assertEqual(self.check(entry(1, observed_by="no step yet (6.1 can't)")), [])

    def test_a_mistyped_heading_fails_and_keeps_the_entry_above_whole(self):
        ledger = entry(1) + "### RPU-2: Typo\n- Status: seen\n"
        problems = self.check(ledger)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("isn't an entry's heading", problems[0])

    def test_a_missing_field_fails(self):
        ledger = "## RPU-1: Something\n- Assumed: x.\n- Status: open\n"
        problems = self.check(ledger)
        self.assertTrue(any("no Source, Observed by" in problem for problem in problems), problems)

    def test_a_status_of_another_shape_fails(self):
        problems = self.check(entry(1, status="seen"))
        self.assertTrue(any("Status is" in problem for problem in problems), problems)

    def test_a_reused_id_fails(self):
        problems = self.check(entry(1) + entry(1))
        self.assertTrue(any("used twice" in problem for problem in problems), problems)

    def test_an_empty_ledger_fails(self):
        self.assertEqual(len(self.check("# Nothing here\n")), 1)


class OpenOnPath(unittest.TestCase):
    def test_lists_a_groups_open_unknowns_and_those_no_step_observes(self):
        ledger = (
            entry(1, observed_by="1.2, 1.3", title="Pairing times")
            + entry(2, observed_by="6.1", title="Out of range")
            + entry(3, observed_by="1.3", status="observed 2026-11-15 (#392): refused", title="Prompt")
            + entry(4, observed_by="no step yet", title="Key missing")
        )
        text = unknowns.open_on_path(ledger, 1)
        self.assertIn("Open unknowns on group 1's path: 1 (of 3 open", text)
        self.assertIn("RPU-1 (step 1.2, 1.3): Pairing times", text)
        self.assertNotIn("RPU-2 ", text)
        self.assertNotIn("RPU-3", text)
        self.assertIn("Open with no step to observe them: RPU-4.", text)

    def test_group_1_is_not_group_10(self):
        ledger = entry(1, observed_by="10.1", title="Later")
        self.assertIn("group 1's path: 0 (of 1 open", unknowns.open_on_path(ledger, 1))


if __name__ == "__main__":
    unittest.main()
