# Retros

Retros on subagent sessions, with what was pursued. Per chief-of-staff practice, about one in five sessions is reviewed, starting with those where something went wrong.

- 2026-11-04, ticket #3 (Handoff prototype). Incident: an unsigned `Set` crashed WirePlumber mid-Call. Six candidates came out of it. Pursued: the safe real-Phone tooling, its brief rule and its lint, filed together as #34, which blocks #6, #13 and #14; the PipeWire source map ([telephony-api-quirks](telephony-api-quirks.md)); the no-debug-symbols note ([hardware-and-stack](hardware-and-stack.md)); and a brief line on literal Bash commands. Not pursued: none. The upstream WirePlumber bug report is offered to the user, not filed.
- 2026-11-05, chief of staff (self): merged PR #40 while CI was pending, which turned `main` red. Fix: always wait on `gh pr checks --watch` before `gh pr merge` ([session-log](session-log.md)).
- 2026-11-05, ticket-less fixer for red `main` (PR #41). It took 44 minutes and 168 tool calls, of which about 26 minutes went on 7 CI runs over 4 rounds. Seven candidates came out of it. Pursued:
  - (Superseded 2026-11-05 by #158: from `700e1aa` GitHub runs full CI only on pushes to `main`, and a PR's status comes from `just ci-local`.) CI runs once per PR push (push triggers on `main` only), and a later push to `main` no longer cancels the run that verifies a merge.
  - Brief: the sandbox-refused Bash forms; one background CI watcher; download artifacts before any rerun; TaskStop everything before replying.
  - #43 (blocked by #42): `just test-ci-like`, an event timeline on every harness timeout, grim and sway exit status, `just ci-wait`, and logs uploaded `always()`.

  Not pursued separately: the debug probe branch, which #43 makes unnecessary.
- 2026-11-05, ticket #42 (`just merge` plus the gallery "Call ended" capture). It took 2 h 13 min and 276 tool calls, 88 min of it waiting. About 60 min went on local gallery loops that couldn't pass on a shared Desktop: another agent's `pkill -f` killed one run, and busy loops pinned to the same CPU 0 starved another. Seven candidates came out of it. Pursued:
  - Brief: kill only your own process IDs; don't pin to a fixed CPU; ship verified parts as `Part of #n` PRs; send a status report before a 15-minute wait or after 2 failed proofs; repeat only what is under test; no waiting on other agents' PRs; Monitors only for lines from a job that is still running.
  - New page [chief-of-staff-practice](chief-of-staff-practice.md): one deliverable per ticket; no repeat-on-the-Desktop acceptance criteria; one file per ticket within a batch; let agents push without waiting.
  - #43 amended: a machine-wide lock, a CPU quota instead of busy loops, and a TMPDIR per run.
  - #53 amended: `automation screenshot --only <state> --repeat N`.
  - #42's remaining criterion replaced by a deterministic capture test.

  Not pursued: none.

## 2026-11-05, #43 implementer (diagnosable CI)

- 221 tool calls in about 73 minutes; shipped #73, #74 and #77 (#75 was superseded). Source: the session transcript, analysed by a retro subagent.
- Local full-suite runs cost about 15 minutes, all repeated by CI. Applied: the brief's Finishing step 1 now runs `just check` plus the touched test files, and tickets say "CI passes" (practice page).
- The brief named two CI watchers (`just ci-wait` and `gh run watch`), and the agent ran both at once: 12 waits cut off, 8 TaskStops. Applied: Finishing step 5 names only `just ci-wait`.
- Reading `crates/test-harness/src/lib.rs` (2,442 lines as of `5f24008`) whole took 88k of the 276k characters before the context was compacted. Filed as a ticket: split it, and add a file-length check.



## 2026-11-05 (evening): Fable retro decisions, and the Sonnet implementer trial

- Retro by Fable, decided with the user, recorded in `c79d731` ([chief-of-staff-practice](chief-of-staff-practice.md), [implementer brief](../agents/implementer-brief.md)): feature freeze until Real-Phone Checklist groups 1, 2 and 6 run on the Pixel; one `after-v1` issue (#154) plus the `after-v1` label for minor findings; research notes only when a PR touches the real Phone or an ADR; tooling, docs and CI PRs get one Sonnet review, one round.
- Sonnet implementer trial, 2026-11-05, three PRs. Source: the review outcomes reported to the chief of staff, not a transcript analysis.
  - #145 (`94dcb16`, #116): passed both reviews in round 1.
  - #149 (`f2ddbbf`, #121): merged after 2 review rounds and a fix-up.
  - #160 (`1210b55`): passed one review.
  - Reading (the chief of staff's, with the user): comparable to Opus implementers. Three PRs is too few to rank the models.
- A lesson from #143 and the local-CI switch is on [chief-of-staff-practice](chief-of-staff-practice.md): `just merge`'s checklist check only fires at merge time, and a rebase can break a test that local CI then catches.

## 2026-11-13 evening, where agents' time went (CI and waiting)

- Source: the chief of staff's retro of the evening, from `~/.cache/desktop-calls-ci/logs` and `gh pr list`; not a transcript analysis.
- Runs took 177 to 222 s whether the Desktop was alone or busy, so a run's own length did not depend on load. The contention was queueing: the median wait for the lock was about 3 minutes, over 13 waits.
- 22 full runs: 9 tested docs-only PRs. PR #220 spent about 25 of its 38 minutes waiting on CI.
- 4 failures, none from load. One failed lint in 4 s and the other jobs still ran for 205 s (head `8b3c277`).
- 31 sleep or poll loops were written in the evening to wait on CI or reviewers. One agent ran `pkill -f 'tools/ci_local.py'` to replace its own queued run and killed two other agents' runs, one at 346 of 348 scenarios.
- Applied: a docs-only run is lint alone and a failed lint ends the run (#239, PR #242); [Shared machine](../agents/shared-machine.md) and `just ci-local --cancel` (#240); the checkpoint rule on [chief-of-staff-practice](chief-of-staff-practice.md).

## 2026-11-14, the disk filled (build caches)

- Source: the chief of staff's account to the author, and `df` on 2026-11-14; not a transcript analysis.
- What happened: `/home` reached 100% from 34 agent worktrees' `target/` folders (about 134 GB; 23 of the worktrees were for merged PRs, 5 to 6 GB each). All work stopped on ENOSPC, including this checkpoint's own first wiki edit.
- Why it got that far: nothing removed a worktree's `target/` once its PR merged, and each worktree builds its own. The brief said nothing about cleaning up after a merge.
- Applied: Finishing step 6 of the [implementer brief](../agents/implementer-brief.md) (clear the cache, say so in the report); 23 merged worktrees removed; #277 asks `just land` to clear the worktree's `target/` after a merge. Not applied: any alert before the disk fills, or a cap on `target/`; neither is ticketed.

## 2026-11-15, the `/tmp` quota, the review gate and two reruns

- Source: the PRs and issues named (read 2026-11-15 after 11:42Z) and the chief of staff's account to the author; not a transcript analysis.
- **`/tmp` quota.** On 2026-11-15 the per-user `/tmp` quota was hit (about 8 GB by the chief of staff's figure; #346 counts 158 `/tmp/desktop-calls-test-*` folders, 3.1 GB) and every tool that writes there failed until the folders were removed by hand. Why it got that far: a failing scenario keeps its folder as evidence, a killed process never reaches `Drop`, and nothing removed either. The same leak was found on 2026-11-05 (473 folders, 5.6 GB) and the fix of that day was `just ci-local` removing folders over 60 minutes old ([testing](testing.md)); scenario runs outside `just ci-local` had no such clean-up (inferred, not checked). Applied: #349 prunes folders over a day old once per test process. One agent also swept shared `/tmp` folders older than 5 minutes that day (relayed); the rule is on [chief-of-staff-practice](chief-of-staff-practice.md). Not applied: any alert before a quota fills.
- **Review gate.** Reviews were trusted to agents until four omissions in two days, and #332 makes `just land` refuse without a record. The cold post-merge reviews that justified it found #328 (a lost proof) and the commit-status hole in #306's check. Lesson, the chief of staff's: a rule an agent can skip at no cost is checked by tooling, with the limit that the tooling records a claim. The gate's own PR took four review rounds; the brief's limit of two was exceeded with approval recorded for the third.
- **Reruns.** Two CI reruns without a found cause: #349's implementer, and main's run on `2846987` (gallery part 1, `missed-calls-out-of-range`). The rule is old ([chief-of-staff-practice](chief-of-staff-practice.md), #167). Earlier gallery-capture flakes (#248, #354) each had a cause that stopping the app before capture fixed; this one had none found when first written; **superseded 2026-11-15 (later):** it turned out (#356, PR #357, `d71a7f4`) to be a product bug: a Call History read 1 ms before a Call's end made the app drop a real Missed Call, which the passing rerun nearly hid from the user. Lesson: a failure of this kind is a possible user-visible defect until its event timeline says otherwise. Not applied: nothing mechanical refuses a rerun.
- **What went well, with evidence:** #340's Dialer regression was caught by CI in its first runs, diagnosed from timings in #347, and fixed twice (#348 at 09:36Z and #351 at 10:10Z, 46 and 80 minutes after #340's merge at 08:50Z) with measurements in each PR and the 50 ms bound unmoved ([testing](testing.md)).
