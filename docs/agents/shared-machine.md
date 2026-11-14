# Shared machine

Rules for every agent on this machine (implementers, reviewers, the chief of staff), because several agents work here at once and share one CI lane, one CPU budget and one `/tmp`. Each rule below is one an agent broke on 2026-11-13, at a cost to other agents' runs.

## CI: one lane, queued by a lock

- `just ci-local` is the only CI. Runs from every worktree take turns on one lock, so a run you start may wait behind others for minutes. A waiting run tests `HEAD` as it is when its turn comes, so committing while it is queued is fine. A worktree has one run at a time: a second `just ci-local` from it is refused, with a message naming the first.
- Start it with the Bash tool's `run_in_background: true`, then do other work or end your turn: the harness sends a completion notice when it exits. Never wait with `sleep`, an `until` loop, `tail -f` or a poll of its output. 31 such loops were written in one evening, and each held a process for as long as the queue.
- Don't redirect its output (`> file`, `| tee`) or choose log names in a shared directory. It prints its own log path once it starts running; read that file only if the run fails.
- To stop a run, `just ci-local --cancel` in the worktree that started it. It stops only that worktree's run, queued or running. To replace a run, cancel it, then start again.
- Never `pkill -f`, `killall`, or `kill` a process ID you did not start. `pkill -f 'tools/ci_local.py'` once killed two other agents' runs, one at 346 of 348 scenarios.

## Waiting for another agent

- A reviewer's report, or any other agent's answer, arrives as a message. Don't sleep, loop or poll waiting for it: finish what you can, then end your turn.
- Before a wait you expect to exceed 15 minutes, send the chief of staff a status report (`SendMessage` to `main`): what you're waiting on.
