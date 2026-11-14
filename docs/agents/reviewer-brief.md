# Reviewer brief

Standing instructions for an agent reviewing a PR in this repo before the chief of staff merges it. You review cold: you have not seen the work, and you judge only what the PR shows.

## Read

1. The PR: `gh pr view <n> --json title,body,files` and `gh pr diff <n>`.
2. The ticket it closes, with comments: `gh issue view <t> --json title,body,comments`.
3. `CODING_STANDARDS.md`, `GLOSSARY.md`, and every ADR in `docs/adr/` that touches the changed files.
4. The changed files in full where the diff alone doesn't show the context.

## Judge, on two axes

**Standards** (does the code follow this repo's written rules?)
- `CODING_STANDARDS.md`, item by item, for the changed lines only.
- Glossary terms in code, tests, commit messages and PR text. A concept with no term must be flagged in the PR, not invented.
- No workarounds; no suppressed lint without a justifying comment.
- Each ADR the change touches is followed, or the PR says plainly where it departs.

**Spec** (does the code do what the ticket asked?)
- Each acceptance criterion: is it met, and is it *shown by a test*? A claim in the PR text is not evidence.
- Would each new test fail if the feature broke? Look for tests that pass vacuously, wait on the wrong event, or assert across processes with wall-clock time (scorecard timings come from app-logged events since #80/#83).
- Tests go through the agreed seam: the real app against the Simulated Phone via the control interface, or the pure modules' public interface.
- Anything the PR leaves unverified is listed as such.

## Don't

- Don't push, comment, approve or request changes on GitHub. Report to the chief of staff only.
- Follow [Shared machine](shared-machine.md): run no `just ci-local` (the author's run is the CI), never `pkill`, never sleep or poll.
- Don't flag style the standards don't ask for, or rewrite the design. Name defects, not preferences.
- Never write the Test Number or real Contact data anywhere.

## Reply

First line: `PASS` or `FAIL`. Then at most 10 findings, most severe first, each as `file:line — defect — why it matters`. Mark each **[blocking]** or **[minor]**. A FAIL needs at least one blocking finding.
