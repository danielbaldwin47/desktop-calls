# Chief-of-staff practice

Dated rules for how the chief of staff writes tickets, plans batches and merges. Implementer rules live in [`docs/agents/implementer-brief.md`](../agents/implementer-brief.md). These rules came from [retros](retros.md).

## Tickets

- 2026-11-05 (retro on #43): a ticket's test criterion reads "CI passes on the PR", not "`just test` passes". The #43 agent spent about 15 minutes on local full-suite runs that CI repeated anyway.
- 2026-11-05 (retro on #42): one shippable deliverable per ticket. #42 bundled `just merge` with a gallery fix, and the finished half waited an hour for the other.
- 2026-11-05 (retro on #42): never make "passes N times in a row on the Desktop" an acceptance criterion. The Desktop is shared with other agents, so the count measures their load as much as the change. Prove repeat-run claims in CI (a repeat job) or with a deterministic harness test, for example "a state shown for 200 ms is captured".
- 2026-11-05 (#47, `3def480`): a timing target for the Desktop gets two criteria: a regression bound that CI's software renderer can meet, and the real target judged on the Desktop by a Real-Phone Checklist step. #47's 150 ms Dialer target was unmeasurable on CI (150.4 ms run alone), so CI holds 250 ms and step 7.4 judges 150 ms. See [testing](testing.md).

## Batches

- 2026-11-05 (retro on #42): before launching a batch, list the files each ticket is likely to touch. Never run two tickets on the same file at once. #42 and the flake fix both worked on `crates/automation/src/gallery.rs`, and one fix was reverted.
- 2026-11-05: only one agent at a time uses the real Phone. In a batch, name which one.

## Merging

- 2026-11-05: confirm checks with `gh pr checks <n> --watch` and that the PR head contains `origin/main`, then merge with `--match-head-commit <full sha>` (a short SHA is rejected). Use `just merge <n>` once #56 lands. Never merge with CI pending: see the #40 incident in [retros](retros.md).
- 2026-11-05: a PR that contains another open PR (#72 contained #69, #71 contained #68) can be merged alone; close the contained PR as superseded. That saves a CI round. Stacked part PRs (#75 on #74) conflict once the lower part is squash-merged, so the implementer brief now forbids stacking.
- 2026-11-05 (retro on #42): never tell an agent to wait for another PR to land before pushing. Let it push and reply. The merge step asks for a branch update if it's needed.
