# Session log

Dated checkpoints so that a resumed or new session can pick up the hill climb. Read the newest entry first.

## 2026-11-05 ~08:00 UTC, batch 1 landed; batch 2 in flight (supersedes the entries below)

- Merged since 04:30, each with green CI: #44 (CI once per PR push), #8 (PR #46), #15 (PR #50), #39 (PR #48), #9 (PR #52), #61 (three flakes, plus Connected/Muted only once the Phone confirms), #24 (PR #58), #56 (`just merge`), #42 (PR #60, gallery records on change), #10 (PR #49, `1231d8e`).
- Merge with `just merge <n>` (verified 2026-11-05 on #56, #60 and #49). See [chief-of-staff-practice](chief-of-staff-practice.md).
- In flight: #55 (faster CI, PR #57, fixing the docs/code race before merge), #11 (Dialer), #13 (the only agent on the real Phone), #54 (system Python).
- Queued: #43 after #55 (both touch `ci.yml`); #65 after #11 (Dial code); then #47 and #53 after #43.
- Waiting on the user: whether to audit and make the repo public. Free Actions minutes ran at about 235 a day before #55, so they would run out around 2026-11-12 (estimate). Also: delete the leftover GHCR package `desktop-calls-ci`, which needs the `delete:packages` scope.
- Real-Phone facts gained: a PBAP pull of 319 Contacts takes 5.35 s; the Pixel caps each history pull at 50 entries ([contacts sync](../research/2026-11-04-contacts-sync.md)). A real Call via `test_call.py --desktop-audio` sent its audio to the Desktop (#10).

## 2026-11-05 04:30 UTC, main green again; batch of six in flight (superseded above)

- PR #41 merged at `41cecbb` after `gh pr checks 41 --watch` passed on both runs. There were two causes: the Banner waited on phone-number data loading on the main thread, and grim crashed with SIGBUS against Docker's 64 MB `/dev/shm`. See `docs/research/2026-11-04-ci-banner-latency.md`.
- Verified 2026-11-05: branch protection and rulesets return HTTP 403 ("Upgrade to GitHub Pro or make this repository public") on this private Free repo. This supersedes "likely unavailable (unverified)" below. #42 adds `just merge` so that the guard lives in tooling.
- A known flake remains: the gallery can miss the 2 s "Call ended" state on a slow runner. Covered by #42.
- In flight, each in its own worktree: #8, #9, #10, #15, #39, #42, plus a retro on the #41 fixer session. Only #10 may use the real Phone (Test Number only). #9 may read PBAP read-only.
- On resume: for each open PR, run `gh pr checks <n> --watch`, then merge (`just merge <n>` once #42 lands). Conflicts go to a merger agent.

## 2026-11-05 01:30 UTC, paused for the user's session reset (superseded above)

- **`main` is red.** PR #40 (#7) was merged at `596fe37` while its CI was still pending, and CI then failed: 7 screen tests time out on "Banner shows within 1s" in CI but pass locally (runs 41947111574 and 41947055883). A fixer agent was started and then stopped at the user's request before it did meaningful work. Its stale worktree may exist under `.claude/worktrees/agent-acc43b6e45a437a6c`; delete it. **First action on resume:** relaunch the fixer (branch `fix/ci-banner-tests`, using diagnosing-bugs). Distinguish CI slowness from a real bug, and keep the 1 s scorecard measurement meaningful.
- Process lesson: `gh pr merge` does not wait for checks, and branch protection is likely unavailable on a private Free repo (unverified). Always run `gh pr checks <n> --watch` and confirm a pass before merging.
- After main is green, the frontier is #8, #9, #10, #15, #39.

## 2026-11-04, later checkpoint

- #7 finished as PR #40: 74 of 74 tests pass, the Banner meets the 1 s target, and the Omarchy 4.5 geometry is in. Its merge status is recorded in the commit log, so check `gh issue view 7`. No implementer is in flight.
- Next frontier: #8, #9, #10, #15, #39. Launch them in parallel worktrees, keeping justfile/harness conflicts in mind; a merger agent resolves conflicts.
- Unverified on the Pixel: real Answer and Hangup, covered by Real-Phone Checklist group 2 (`docs/research/2026-11-04-incoming-call.md` §6).

## 2026-11-04, checkpoint before the user's session limit

- Merged to `main`: #2 foundation, #3 Handoff prototype ([ADR-0005](../adr/0005-handoff-to-the-phone-bounces-the-hands-free-profile.md)), #34 real-Phone tooling, #4 test seam, #6 conformance, #5 screen harness.
- In flight: **#7** (incoming Call, end to end), an implementer in worktree `.claude/worktrees/agent-a13ce3f618da09c0a` on branch `ticket/7-incoming-call`. It was told about the Omarchy 4.5 Banner geometry. On resume, check whether a PR for #7 exists (`gh pr list`). If not, inspect that worktree and branch, then relaunch an implementer that continues from it.
- Frontier once #7 merges: #8, #9, #10, #15, #39 (theme), plus anything else whose open blockers reach zero.
- Open offers to the user: draft an upstream WirePlumber bug report (unsigned `Set` crash), which has not been accepted yet.
- Process: implementers follow [`docs/agents/implementer-brief.md`](../agents/implementer-brief.md). The chief of staff merges green PRs, and conflicts go to a merger agent. Run a retro on about one in five sessions ([retros](retros.md)).
