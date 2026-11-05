# Retros

Retros on subagent sessions, with what was pursued. Per chief-of-staff practice, about one in five sessions is reviewed, starting with those where something went wrong.

- 2026-11-04, ticket #3 (Handoff prototype). Incident: an unsigned `Set` crashed WirePlumber mid-Call. Six candidates came out of it. Pursued: the safe real-Phone tooling, its brief rule and its lint, filed together as #34, which blocks #6, #13 and #14; the PipeWire source map ([telephony-api-quirks](telephony-api-quirks.md)); the no-debug-symbols note ([hardware-and-stack](hardware-and-stack.md)); and a brief line on literal Bash commands. Not pursued: none. The upstream WirePlumber bug report is offered to the user, not filed.
- 2026-11-05, chief of staff (self): merged PR #40 while CI was pending, which turned `main` red. Fix: always wait on `gh pr checks --watch` before `gh pr merge` ([session-log](session-log.md)).
