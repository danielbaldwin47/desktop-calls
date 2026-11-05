# Implementer brief

The standing instructions for any agent implementing a ticket in this repo. The chief of staff points you here, along with a ticket number.

## Before you start

1. Read `CLAUDE.md`, `GLOSSARY.md`, every ADR in `docs/adr/` that touches your area, and `docs/wiki/index.md` plus the pages relevant to your ticket. If `CODING_STANDARDS.md` exists, read it; it binds you.
2. Read the spec (issue #1) sections your ticket cites, then your ticket: `gh issue view <n> --json number,title,body,labels,comments`.
3. Confirm your branch is based on the current tip of `origin/main` (`git fetch && git merge-base --is-ancestor origin/main HEAD`). If it isn't, reset onto it.

## While you work

- Build test-first: call the Skill tool with `mattpocock-skills:tdd`.
- A failure that shows only on CI: reproduce it with `just test-ci-like` before pushing a fix. Wait on CI with `just ci-wait`, which prints the failed tests' panics and downloads their scenario logs.
- Tests go through the agreed seam only: the real app against the Simulated Phone, driven and observed through the app's control interface, plus screenshots for visuals ([product-shape](../wiki/product-shape.md), Q27). Pure modules named in the spec (matching, Call state machine, messages) may also be tested at their public interface.
- Use the glossary's terms in code, tests, commit messages and PR text. If a concept you need has no term, stop and report it. Don't invent one.
- **No workarounds.** If a convention, lint or the harness is in your way, fix it properly in your branch, or stop and report. Never suppress a lint without a comment that justifies it.
- If your work contradicts an ADR, say so explicitly in the PR instead of overriding it silently.
- Real Phone: the user's Pixel 9a is paired. You may place Calls **only** to the Test Number in `~/.config/desktop-calls/test-number`. Never write that number into the repo, issues, PRs or logs. Never Call any other number, and never answer or decline real incoming Calls.
- The user's installed `desktop-calls.service` runs on the real Phone. Start the app against the real Phone only through `just run` or a `just test-call` hook: both stop the service for the run and start it again afterwards, even if killed ([ADR-0006](../adr/0006-real-phone-dev-runs-stop-the-installed-service.md)). Never stop, start or restart the service any other way, and never run the app beside it. If `just test-call` or `just run` exits 3, the service didn't start again: tell the user at once.
- Real-Phone D-Bus work goes through `just test-call` and `tools/real-phone/`, never ad-hoc scripts. `just test-call 'HOOK'` dials the Test Number, runs your hook once the Call is active, and always hangs up: when the hook exits or is killed, on Ctrl-C, and through a guard process even if `test-call` itself is killed. It exits 2 if a Call may still be live; if it does, tell the user at once. New probes import `tools/real-phone/real_phone.py`: `call()` for methods with an explicit signature, `set_property()` for writes, `Telephony.hang_up_test_call()` and `remaining_calls()` before exiting. Read-only `busctl` with exact signatures is fine.
- Introspect first, and never guess a signature. On 2026-11-04 a malformed `Properties.Set` crashed WirePlumber mid-Call (ticket #3). `just check` rejects any `Set(` under `tools/` without `signature=`.
- Try any write method that is new to you (a property `Set`, or a method that changes state) once with **no Call active** before using it during one.
- Keep Bash commands literal, because the worktree sandbox refuses the rest. That rules out `$(...)`, loop variables, variable assignments (including `VAR=value command`), heredocs, and wrappers that start a shell (such as `taskset … bash`). Write a script to your scratchpad instead.
- Don't run `sudo`. Don't edit `/etc`. Don't unpair the Phone.
- **You share this machine with other agents.** Kill only process IDs you started yourself, never `pkill -f` or `killall`, because a pattern can match another agent's run. Don't pin load to a fixed CPU (`taskset -c 0`), since others copy the same recipe. Timing proofs under load use the repo's shared recipe (`just test-ci-like`), which takes a machine-wide lock. If a run is slow or odd, suspect another agent before your code.
- **Ship in parts, report early.** As soon as one part of the ticket is verified, open a PR for it with `Part of #<n>` rather than holding it for the rest. Before any wait expected to exceed 15 minutes, and after 2 failed attempts at the same proof, send the chief of staff a status report (SendMessage to `main`): what is verified, what you're waiting on, and what you propose. Don't keep looping.
- **Part PRs are based on `main`, never on another part's branch.** Parts are squash-merged, so a PR stacked on another part conflicts once that part lands. If part 2 needs part 1's code, branch part 2 from part 1 but open its PR only after part 1 merges, then `git rebase --onto origin/main <part-1 tip> <part-2 branch>` first. Or put both in one PR.
- Repeated-run proofs ("passes N times in a row") are evidence, not a gate you loop on. Repeat only the thing under test, not a whole suite. If a run fails for a reason unrelated to your change, report it rather than resetting the count.

## Finishing

1. Run `just check` and the test files your change touches, and make them pass: `just test-file <file>` for a harness file in `crates/test-harness/tests/` (`just test-file <file> <part of a name>` for some of its scenarios), `cargo test -p <crate> --test <file>` for any other crate's. Don't run the whole `just test` locally: CI runs it, and on this shared Desktop a local full run costs 10 minutes or more and measures other agents' load. Run it locally only to reproduce a CI failure, under `just test-ci-like`.
2. Merge the tip of `origin/main` into your branch, re-run checks, and push the branch.
3. Open a PR to `main` with `Closes #<n>`. The body follows `mattpocock-skills:pr` and ends with the attribution line from your system prompt.
4. If your PR touches the real Phone or an ADR, record durable findings (hardware behaviour, quirks, decisions) as a dated research note in `docs/research/` within your PR. Otherwise the PR body is the record; write no note. The chief of staff ingests them into the wiki.
5. Wait on CI with one `just ci-wait`, started with `run_in_background: true`. It is your only watcher: no `gh run watch`, and no `tail -f` or `grep` on its output file. A background command already notifies you when it exits, so add a Monitor only to react to a line from a job that is still running. Don't wait for other agents' PRs to land: push, reply, and let `just merge` ask for an update if your branch falls behind. If a run fails, download its artifacts before any `gh run rerun`, because a rerun discards them. Before replying, stop every Monitor and background command you started (TaskStop), so nothing outlives your session.
6. Reply in **10 lines or fewer**: PR URL, what you verified and how, anything left unverified, any follow-up a ticket should capture.
