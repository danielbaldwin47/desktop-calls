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
  - Planned next: move this review to GitHub Actions as the first task of a software factory, after v1.

- 2026-11-05: confirm checks with `gh pr checks <n> --watch` and that the PR head contains `origin/main`, then merge with `--match-head-commit <full sha>` (a short SHA is rejected). Use `just merge <n>` once #56 lands. Never merge with CI pending: see the #40 incident in [retros](retros.md).
- 2026-11-05: a PR that contains another open PR (#72 contained #69, #71 contained #68) can be merged alone; close the contained PR as superseded. That saves a CI round. Stacked part PRs (#75 on #74) conflict once the lower part is squash-merged, so the implementer brief now forbids stacking.
- 2026-11-05 (retro on #42): never tell an agent to wait for another PR to land before pushing. Let it push and reply. The merge step asks for a branch update if it's needed.
