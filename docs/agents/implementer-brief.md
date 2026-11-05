# Implementer brief

The standing instructions for any agent implementing a ticket in this repo. The chief of staff points you here, along with a ticket number.

## Before you start

1. Read `CLAUDE.md`, `GLOSSARY.md`, every ADR in `docs/adr/` that touches your area, and `docs/wiki/index.md` plus the pages relevant to your ticket. If `CODING_STANDARDS.md` exists, read it; it binds you.
2. Read the spec (issue #1) sections your ticket cites, then your ticket: `gh issue view <n> --json number,title,body,labels,comments`.
3. Confirm your branch is based on the current tip of `origin/main` (`git fetch && git merge-base --is-ancestor origin/main HEAD`). If it isn't, reset onto it.

## While you work

- Build test-first: call the Skill tool with `mattpocock-skills:tdd`.
- Tests go through the agreed seam only: the real app against the Simulated Phone, driven and observed through the app's control interface, plus screenshots for visuals ([product-shape](../wiki/product-shape.md), Q27). Pure modules named in the spec (matching, Call state machine, messages) may also be tested at their public interface.
- Use the glossary's terms in code, tests, commit messages and PR text. If a concept you need has no term, stop and report it. Don't invent one.
- **No workarounds.** If a convention, lint or the harness is in your way, fix it properly in your branch, or stop and report. Never suppress a lint without a comment that justifies it.
- If your work contradicts an ADR, say so explicitly in the PR instead of overriding it silently.
- Real Phone: the user's Pixel 9a is paired. You may place Calls **only** to the Test Number in `~/.config/desktop-calls/test-number`. Never write that number into the repo, issues, PRs or logs. Never Call any other number, and never answer or decline real incoming Calls.
- Real-Phone D-Bus work goes through `just test-call` and `tools/real-phone/`, never ad-hoc scripts. `just test-call 'HOOK'` dials the Test Number, runs your hook once the Call is active, and always hangs up: when the hook exits or is killed, on Ctrl-C, and through a guard process even if `test-call` itself is killed. It exits 2 if a Call may still be live; if it does, tell the user at once. New probes import `tools/real-phone/real_phone.py`: `call()` for methods with an explicit signature, `set_property()` for writes, `Telephony.hang_up_test_call()` and `remaining_calls()` before exiting. Read-only `busctl` with exact signatures is fine.
- Introspect first, and never guess a signature. On 2026-11-04 a malformed `Properties.Set` crashed WirePlumber mid-Call (ticket #3). `just check` rejects any `Set(` under `tools/` without `signature=`.
- Try any write method that is new to you (a property `Set`, or a method that changes state) once with **no Call active** before using it during one.
- Keep Bash commands literal, because the worktree sandbox refuses the rest. That rules out `$(...)`, loop variables, variable assignments (including `VAR=value command`), heredocs, and wrappers that start a shell (such as `taskset … bash`). Write a script to your scratchpad instead.
- Don't run `sudo`. Don't edit `/etc`. Don't unpair the Phone.

## Finishing

1. Run the repo's check entry point (`just check` once it exists) and make it pass.
2. Merge the tip of `origin/main` into your branch, re-run checks, and push the branch.
3. Open a PR to `main` with `Closes #<n>`. The body follows `mattpocock-skills:pr` and ends with the attribution line from your system prompt.
4. Record durable findings (hardware behaviour, quirks, decisions) as a dated research note in `docs/research/` within your PR. The chief of staff ingests them into the wiki.
5. Wait on CI with one background `gh run watch <id> --exit-status`, never stacked watchers. If a run fails, download its artifacts before any `gh run rerun`, because a rerun discards them. Before replying, stop every Monitor and background command you started (TaskStop), so nothing outlives your session.
6. Reply in **10 lines or fewer**: PR URL, what you verified and how, anything left unverified, any follow-up a ticket should capture.
