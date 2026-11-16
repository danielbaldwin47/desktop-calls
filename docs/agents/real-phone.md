# Real Phone

Rules for a real-phone step, which an implementer takes only when the chief of staff's brief for the task says the owner approved it ([implementer brief](implementer-brief.md)). The owner's Pixel 9a is paired, and the installed `desktop-calls.service` runs on it.

- **Calls go to the Test Number only**, read from `~/.config/desktop-calls/test-number`. The number stays out of the repo, issues, PRs and logs. Incoming real Calls are left to ring: never answered, never declined. The Phone stays paired.
- **Start the app against the real Phone through `just run` or a `just test-call` hook.** Both stop the installed service for the run and start it again afterwards, even if killed ([ADR-0006](../adr/0006-real-phone-dev-runs-stop-the-installed-service.md)); the service is started and stopped no other way, and the app never runs beside it. Exit 3 from either means the service didn't start again: tell the owner at once.
- **D-Bus work goes through `just test-call` and `tools/real-phone/`.** `just test-call 'HOOK'` dials the Test Number, runs the hook once the Call is active, and always hangs up, through a guard process even if `test-call` itself is killed. Exit 2 means a Call may still be live: tell the owner at once. A new probe imports `tools/real-phone/real_phone.py`: `call()` for methods with an explicit signature, `set_property()` for writes, and `Telephony.hang_up_test_call()` and `remaining_calls()` before it exits. Read-only `busctl` with exact signatures is fine.
- **Introspect first; take every signature from the introspection.** On 2026-11-04 a guessed `Properties.Set` signature crashed WirePlumber mid-Call (ticket #3); `just check` rejects a `Set(` under `tools/` without `signature=`.
- **Try a write that is new to you** (a property `Set`, or a method that changes state) once with no Call active before using it during one.
- **Record what the hardware showed** as a dated note in `docs/research/`, in the PR.
