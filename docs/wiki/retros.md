# Retros

Retros on subagent sessions, with what was pursued. Per chief-of-staff practice, about one in five sessions is reviewed, starting with those where something went wrong.

- 2026-11-04, ticket #3 (Handoff prototype). Incident: an unsigned `Set` crashed WirePlumber mid-Call. Six candidates came out of it. Pursued: the safe real-Phone tooling, its brief rule and its lint, filed together as #34, which blocks #6, #13 and #14; the PipeWire source map ([telephony-api-quirks](telephony-api-quirks.md)); the no-debug-symbols note ([hardware-and-stack](hardware-and-stack.md)); and a brief line on literal Bash commands. Not pursued: none. The upstream WirePlumber bug report is offered to the user, not filed.
- 2026-11-05, chief of staff (self): merged PR #40 while CI was pending, which turned `main` red. Fix: always wait on `gh pr checks --watch` before `gh pr merge` ([session-log](session-log.md)).
- 2026-11-05, ticket-less fixer for red `main` (PR #41). It took 44 minutes and 168 tool calls, of which about 26 minutes went on 7 CI runs over 4 rounds. Seven candidates came out of it. Pursued:
  - CI runs once per PR push (push triggers on `main` only), and a later push to `main` no longer cancels the run that verifies a merge.
  - Brief: the sandbox-refused Bash forms; one background CI watcher; download artifacts before any rerun; TaskStop everything before replying.
  - #43 (blocked by #42): `just test-ci-like`, an event timeline on every harness timeout, grim and sway exit status, `just ci-wait`, and logs uploaded `always()`.

  Not pursued separately: the debug probe branch, which #43 makes unnecessary.
