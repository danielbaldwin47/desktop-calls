# Implementer brief

Standing instructions for an agent implementing an issue in this repo. The chief of staff's brief for your task names the issues, their order and anything that differs from this page; where the two differ, that brief wins.

## Before you start

1. Read `CLAUDE.md`, `CODING_STANDARDS.md` (it binds you), `GLOSSARY.md`, the ADRs in `docs/adr/` that touch your area, `docs/wiki/index.md` and the wiki pages your issue touches.
2. Read your issue (`gh issue view <n> --json number,title,body,labels,comments`) and the spec sections of issue #1 it cites.
3. Branch from the current `origin/main`. Done when `git fetch && git merge-base --is-ancestor origin/main HEAD` exits 0.

## While you work

- **Own each PR through merge.** One PR per issue unless told otherwise; issues one at a time, in the order given. Stay attached until your last PR has merged and `main`'s run on it is green. Hand back early only for a decision that is the chief of staff's: a product or design change, a third review round, or a blocked permission.
- **Test first**, with the Skill `mattpocock-skills:tdd`: a failing scenario against the Simulated Phone for every behaviour change, through the seam in `CODING_STANDARDS.md` under "Tests".
- **Simulated Phone only.** The real Phone, Bluetooth and WirePlumber stay untouched, unless the chief of staff's brief for this task says the owner approved a real-phone step; then follow [Real Phone](real-phone.md) for that step.
- **The repo is public.** Phone numbers, contact names, device addresses and email stay out of it, its issues and its PRs; `just check` catches some of them.
- **UX is king**: when a choice trades the user's experience against anything else, UX wins. Record every design decision you make in the PR body. When the chief of staff's brief marks a decision "decided for the owner", add its line to [decided for the owner](../wiki/decided-for-the-owner.md) in your PR.
- **Glossary words** in code, tests, commit messages and PR text. A concept with no term: stop and report it.
- **Fix the cause.** A convention, lint or harness in your way gets fixed properly in your branch, or you stop and report. A suppressed lint carries a comment that justifies it. Work that contradicts an ADR says so in the PR.
- **Literal Bash commands**, because the worktree sandbox refuses `$(...)`, loop variables, variable assignments (`VAR=value command` too), heredocs and wrappers that start a shell. Put anything that needs them in a script in your scratchpad. Run without `sudo`, and leave `/etc` alone.
- **Shared machine.** Read [Shared machine](shared-machine.md) before `just ci-local`, landing, cancelling a run or killing a process. Timing proofs under load go through `just test-ci-like`, which takes a machine-wide lock; leave CPU placement to the scheduler (others copy a `taskset -c 0`). A slow or odd run: suspect another agent's load before your code.
- **Report early.** Open a PR for each verified part with `Part of #<n>`. Send the chief of staff a status report (SendMessage to `main`) before any wait over 15 minutes and after 2 failed attempts at one proof: what is verified, what you wait on, what you propose.
- **Part PRs are based on `main`**, since parts are squash-merged. If part 2 needs part 1, open part 2's PR after part 1 merges, running `git rebase --onto origin/main <part-1 tip> <part-2 branch>` before its first push; or put both in one PR.
- **A repeated-run proof is evidence.** Repeat only the thing under test, and report a failure unrelated to your change instead of resetting the count.
- **Subagents, reviewers included**, get an explicit model (Sonnet or Opus, never Fable), are never forks, and have a description ending `: <model>`.

## A red CI run

GitHub runs CI on every push to a PR; its "CI passed" check on the head is what a merge needs. A red run is answered by its cause, found and fixed; CI alone posts statuses. `gh run rerun` is not used: a rerun's pass proves nothing, and on 2026-11-15 one nearly hid a lost Missed Call (#356).

1. Read it: `just ci-wait <run>` prints each failed test's panic, with the event timeline, and downloads the scenario logs into `target/ci/<run>/`.
2. Find the cause by event order, and post it as a PR comment. A timeout waiting for a state is a possible product defect until its timeline says otherwise. A failure that shows only under CI's two CPUs reproduces with `just test-ci-like`.
3. Push the fix. Done when the run on the new head is green. For a cause outside your diff, file an issue with the evidence and send the chief of staff its number.

## Finishing

1. **Check.** Done when `just check` and the test files your change touches pass: `just test-file <file> [part of a name]` for a harness file in `crates/test-harness/tests/`, `cargo test -p <crate> --test <file>` for another crate's. CI runs the whole suite; run `just test` here only to reproduce a CI failure, under `just test-ci-like`.
2. **Bring the branch up to date**: `git fetch origin && git merge origin/main`, then step 1 again. Once a branch is pushed, update it by merging only, since a rebase rewrites pushed commits (and `gh pr update-branch` is not used). Done when the branch contains `origin/main` and step 1 passes.
3. **Push and open the PR** to `main` with `Closes #<n>`. Its body follows `mattpocock-skills:pr` and ends with the attribution line from your system prompt. A PR that touches the real Phone or an ADR also adds its durable findings as a dated note in `docs/research/`. Done when `gh pr view <n> --json headRefOid` shows your head commit.
4. **Wait for CI**: start `just ci-wait` in the background and review (step 5) while it runs. Done when it prints `CI passed`; a red run goes through "A red CI run" above.
5. **Review the pushed head** with subagents. A PR touching `crates/`, `data/`, `Cargo.toml`, `Cargo.lock` or `rust-toolchain.toml` needs a Sonnet `standards-spec` and an Opus `spec` review; any other PR, one Sonnet `tooling` review. Fix the findings, push, review again; at most two rounds, then ask the chief of staff. A new commit needs new reviews, except a merge of `origin/main` that resolved no conflict and changed nothing more. Done when every review the PR needs has passed on its head.
6. **Record each review** that happened: `just review-record <n> <kind> <model> <PASS|FAIL> <findings file>`, the file listing each finding and how it was resolved (the fixing commit, or why it was left). Done when the last record prints `Next: just land <n>`.
7. **Land**: `just land <n>`, in the background, in the worktree that has the branch. Holding the landing lock, it merges `origin/main` if `main` moved by code, pushes, waits for "CI passed", squash-merges and removes the worktree's `target/`. It refuses without the review records; `just land <n> --plan` says what it would do and why it would refuse. On a merge conflict it changes nothing and prints the next command: resolve, commit, run it again. Done when it prints `Merged PR #<n>` and the run on `main` for the merge commit is green (`gh run list --branch main --commit <sha>`, then `just ci-wait <run>`).
8. **Clean up.** Done when your worktree has no `target/` (`just land` removes it and says how much it freed; `cargo clean` if it says the removal failed, or if the work is abandoned) and every Monitor and background command you started is stopped (TaskStop). Keep `target/` while more work remains in the worktree, and say in your reply if it is still there.
9. **Reply** in 10 lines or fewer, unless the chief of staff's brief sets another length: PR URL and merge commit, the design decisions you made (in full in the PR body), what you verified and how, that you cleared `target/`, anything left unverified, and any follow-up an issue should capture.
