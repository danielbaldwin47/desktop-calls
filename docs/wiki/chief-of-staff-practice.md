# Chief-of-staff practice

Dated rules for how the chief of staff writes tickets, plans batches and merges. Implementer rules live in [`docs/agents/implementer-brief.md`](../agents/implementer-brief.md). These rules came from [retros](retros.md).

## Tickets

- 2026-11-05 (retro on #43): a ticket's test criterion reads "CI passes on the PR", not "`just test` passes". The #43 agent spent about 15 minutes on local full-suite runs that CI repeated anyway.
- 2026-11-05 (retro on #42): one shippable deliverable per ticket. #42 bundled `just merge` with a gallery fix, and the finished half waited an hour for the other.
- 2026-11-05 (retro on #42): never make "passes N times in a row on the Desktop" an acceptance criterion. The Desktop is shared with other agents, so the count measures their load as much as the change. Prove repeat-run claims in CI (a repeat job) or with a deterministic harness test, for example "a state shown for 200 ms is captured".
- 2026-11-05 (#47, `3def480`): a timing target for the Desktop gets two criteria: a regression bound that CI's software renderer can meet, and the real target judged on the Desktop by a Real-Phone Checklist step. #47's 150 ms Dialer target was unmeasurable on CI (150.4 ms run alone), so CI holds 250 ms and step 7.4 judges 150 ms. See [testing](testing.md).

## Batches

- 2026-11-05, after an independent review of the session (Fable):
  - **At most one infrastructure ticket** (CI, checklist tooling, review machinery) is in flight at a time, and every batch contains a user-facing ticket.
  - **Report the scorecard in three tiers:** verified on the Pixel, verified on the Simulated Phone only, and claimed. Don't count tickets.
  - **Cite a source for every number** in a progress report. The review found my "1.28 s audio" figure had no source.

- 2026-11-05, retro by Fable, decided with the user: **feature freeze until checklist groups 1, 2 and 6 have run on the Pixel.** Finish the PRs already open; start no new feature tickets. The review found that no scorecard item was verified on the Pixel and real Answer had never been observed.
- 2026-11-05, same retro: **minor review findings go into one `after-v1` issue** as bullets, not separate tickets. Measurement and tooling tickets (#25–#27, #139–#152) carry the `after-v1` label.

- 2026-11-05 (retro on #42): before launching a batch, list the files each ticket is likely to touch. Never run two tickets on the same file at once. #42 and the flake fix both worked on `crates/automation/src/gallery.rs`, and one fix was reverted.
- 2026-11-05: only one agent at a time uses the real Phone. In a batch, name which one.

## Merging

- 2026-11-05, decided with the user after a 5-PR trial: every PR gets two cold reviews before `just merge`, run alongside CI. Both follow [`docs/agents/reviewer-brief.md`](../agents/reviewer-brief.md):
  - **Sonnet** covers both axes, Standards and Spec.
  - **Opus** covers the Spec axis only.

  Any FAIL, or any minor finding that is a real defect, goes back to the PR's agent. After 2 failed rounds, the user decides.
  - **Trial results:** without review, all 5 PRs (#129, #130, #132, #134, #135) would have merged on green CI, and review sent all 5 back.
  - **Serious finds:**
    - Holding one Call, with one live and a third waiting: Hang up on the held Call sent CHLD=0, which declined the waiting caller (#129, Opus).
    - Checklist step 7.1 could ring a stranger on a mistyped digit (#134, Sonnet).
    - The 6.2 timing test passed without ever measuring (#134, Opus).
  - **Each model caught things the other missed:**
    - Opus: protocol bugs, vacuous tests, log-cursor races.
    - Sonnet: glossary rules, safety gaps, stale text.
  - **Lighter reviews for tooling** (Fable retro, 2026-11-05, user's decision): a PR that touches only `tools/`, `crates/automation/`, docs or `.github/` gets one Sonnet review and one round, then merges. #134, a checklist-script PR, took 4 rounds.
  - Planned next: move this review to GitHub Actions as the first task of a software factory, after v1.

- 2026-11-05 (superseded for PRs merged after `700e1aa`, by #158): confirm checks with `gh pr checks <n> --watch` and that the PR head contains `origin/main`, then merge with `--match-head-commit <full sha>` (a short SHA is rejected). Use `just merge <n>` once #56 lands. Never merge with CI pending: see the #40 incident in [retros](retros.md).
- 2026-11-05: a PR that contains another open PR (#72 contained #69, #71 contained #68) can be merged alone; close the contained PR as superseded. That saves a CI round. Stacked part PRs (#75 on #74) conflict once the lower part is squash-merged, so the implementer brief now forbids stacking.
- 2026-11-05 (retro on #42): never tell an agent to wait for another PR to land before pushing. Let it push and reply. The merge step asks for a branch update if it's needed.

## After the local-CI switch and the retro (2026-11-05, evening)

- 2026-11-05 (#158, PR #159, `700e1aa`): a PR is green when `just ci-local` has posted "CI passed" on its head commit, and a PR that was rebased or amended needs a fresh run before `just merge`. Do not wait for a GitHub run on a PR: there is none. Known load-sensitive flakes in local runs are listed in #154's comments. See [testing](testing.md).
- 2026-11-05 (#143, `955394b`): `just merge` refuses a PR that closes an issue named in a checklist step's `lands_with` until that step has a script (`tools/checklist_tickets.py`). It is caught only at merge time, after local CI. When a ticket closes an issue a checklist step waits on, put the step's script in the same PR, or tell the agent in the ticket. #160 (`1210b55`) was the follow-up that made step 1.2 say what went wrong.
- 2026-11-05 (#143, `955394b`): rebasing onto a PR that merged first can break a test the rebased PR didn't touch (#153's Setup Window change against #143's keyboard test). Local CI catches it, so a rebased PR is never pushed on the strength of the earlier run.
- 2026-11-05, Sonnet implementers (user's trial; see [retros](retros.md)): #145, #149 and #160 were Sonnet implementations, comparable to Opus. Three PRs is a small sample.
- 2026-11-05, restated from the Fable retro (`c79d731`) for the record: research notes in `docs/research/` only when a PR touches the real Phone or an ADR (the [implementer brief](../agents/implementer-brief.md), Finishing step 4); tooling, docs and CI PRs get one Sonnet review and one round (Merging, above).

## Night into 2026-11-06: PR-only docs, the #154 triage, merging from behind main

- 2026-11-05, decided by the chief of staff: changes to `docs/wiki/` and other docs go through a PR and `just ci-local`, like code, and the author doesn't merge it. `2173174` (the wiki update of 2026-11-05 afternoon) has no PR number in its subject: it was pushed straight to `main` and bypassed the "CI passed" gate.
- 2026-11-05, the #154 triage (see [session-log](session-log.md)): the `after-v1` bullets were triaged by GPT 6.1 Sol's proposals and Sonnet's approval, and per the chief of staff Sonnet overturned 5 of 19. Three findings became tickets #161, #162 and #163, each scoped with a test that fails today, and landed as PRs #164, #166 and #165 within about 3 hours. Findings already fixed were closed from the list with the commit that fixed them (`7c8b607`, `ba22b51`, `700e1aa`).
- 2026-11-05 (PR #166): a reviewer asks whether the app could drop a call it depends on and every test still pass, because the stand-in is more lenient than the real service. The rule and the evidence are on [testing](testing.md).
- 2026-11-05 (#167, PR #173; #154's tooling notes): a failure under local CI is not answered with "rerun until green". `just ci-local` now says whether load was high, a flake gets a ticket, and the fix goes at the cause found by event order, without loosening a bound. The causes of seven flakes are on [testing](testing.md).
- 2026-11-06 (#181, PR #182, `1139783`; supersedes "the merge step asks for a branch update if it's needed" under "Merging", above, and the `update-branch` note in #154): `just merge` no longer calls `gh pr update-branch`, because GitHub's merge commit has no "CI passed" status and the merge stopped (seen on #177). When `main` has moved it prints `git fetch origin && git switch <branch> && git merge origin/main && just ci-local`, then `just merge <n>`. The merge commit fast-forwards the pushed branch, so one `just ci-local` run tests it, pushes it and posts the status; an agent merges `origin/main` into its branch and never rebases a pushed branch ([implementer brief](../agents/implementer-brief.md)). Before this, a rebase made `just ci-local` refuse the non-fast-forward push after a two-minute run (seen on #179). PR #182 says its one-run demonstration on a throwaway PR is in a comment on it; I did not check that comment.

## 2026-11-13: owned PRs, a concept review before the spec, probes before building

- 2026-11-13, per the chief of staff: **each implementer owned its PR through merge, dispatching its own cold reviews** ([reviewer brief](../agents/reviewer-brief.md)), so the chief of staff spent one launch and one report per ticket chain, not one per review round. This follows the user's request that interactions with subagents stay minimal ([user-preferences](user-preferences.md)). Evidence that the chain closed itself: PRs #208, #210, #211 and #214 merged between 23:38 and 23:56 UTC on 2026-11-13, and #210's body names a round-1 review by Opus. It does not change the rule that the author of a wiki PR doesn't merge it ([docs-layout](docs-layout.md)); that rule is about docs written by the chief of staff and still applies to this page's own PR.
- 2026-11-13, user's rule, with the chief of staff's evidence: **when a decision is a conceptual or bigger design direction, the chief of staff takes the concept to a review before the spec is written and before builders are sent.** Why: the first Calls Off spec (#198) settled about 20 points without the user (the chief of staff's count; the issue's "Decided without the user" list names 12), and one was overturned after the tickets were published: the user asked for a GUI for Quiet Hours, and ticket #209 was filed at 23:31 UTC, after #202 to #207 at 23:26 UTC (from the issues' creation times). The concept review that followed produced two more changes before anything was built (a Dialer Call during Calls Off as a one-Call exception; uninstalling unblocks the Phone, #215; see [product-shape](product-shape.md)). The concept review is not the same as listing the chief of staff's decisions for the user afterwards; the specs do both.
- 2026-11-13, chief of staff's practice: **a real-stack probe before building on an unverified mechanism.** The probe of `Device1.Blocked` on the real Pixel ([note](../research/2026-11-13-calls-off-mechanism-probe.md), PR #214) corrected the spec: BlueZ does not refuse a `Connect` from the Desktop, it pages the Phone and hangs 25 s or more. Without it the Simulated Phone would have modelled a quick refusal, which is the stand-in being kinder than the real service ([testing](testing.md), "Stand-ins must be as strict as the real service"). The user approved the probe first, with no Call on the Phone (per the note); the Real-Phone rule that only one agent uses the Phone at a time is unchanged.
- 2026-11-13, supersedes the note in [session-log](session-log.md) (2026-11-06 to 2026-11-13) that the permission check denied the chief of staff's `gh issue create`: agents were allowed to run it later that day. Evidence: issues #196, #197 to #207, #209, #212, #213, #215 and #216 (#196 is the red-CI ticket the earlier note says waited on the user) were created between 23:15 and 23:57 UTC on 2026-11-13. They show the user's GitHub account as author, as an agent's `gh` does (inferred from PR #211's comment, written by an agent and shown under that account). The rule that decides it is the permission check's, not a wiki rule, so a later denial is possible.
- 2026-11-13, order for tickets that edit the reconnect code, decided by the chief of staff from the tickets' own "Order" sections: **#212, #213, #216, then #202, #215, #203, #204, #205, #209, #206, #207, strictly in sequence**, because they share `crates/core/src/machine/presence.rs` and `crates/desktop-calls/src/bluez.rs` (and #212 and #202 share `crates/simulated-phone/src/range.rs`), and the one-ticket-per-file rule above holds. #212's text says to land it, then #213, then #202; #216's says any order among the three, all before #202, so the order of #213 and #216 is the chief of staff's choice. #197's text puts the Call Pill tickets first in the shared chain, since they and Calls Off both edit the core's dispatch, Messages and control interface; whether the Call Pill tickets could safely overlap the reconnect tickets was not recorded.
