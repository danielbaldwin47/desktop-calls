# Real-Phone test tooling: `just test-call` on the Pixel 9a

Date: 2026-11-04. Ticket #34. Phone: Pixel 9a `AA:BB:CC:DD:EE:FF`. Desktop and stack as in the [Handoff prototype report](2026-11-04-handoff-to-phone-prototype.md).
Legend: **[V]** = verified on this machine. **[I]** = inferred, not tested.

**Bottom line:** `just test-call` placed and hung up 5 Calls to the Test Number, and the moved Handoff probe 1 more. After every one, `GetCalls` was empty, checked by the tool and again by hand with `busctl`. The Call was hung up when the hook exited, when the hook was killed with `SIGKILL`, on `SIGINT` (Ctrl-C), and when `test-call` itself was killed with `SIGKILL` (the guard process hung up). The Test Number appeared 0 times in the tool output, the repo, or the user and system journals for the test window.

## 1. What was built

- `tools/real-phone/real_phone.py`: the shared module. `call()` always sends an explicit signature (it calls `call_blocking` directly, so dbus-python never guesses). `set_property()` takes a typed value (`dbus.Boolean(False)`), checks it against the property's introspected type and access, then sends `ssv`. Redaction wraps standard output and standard error, so tracebacks and hook output are redacted too. `Telephony.hang_up_test_call()` retries for 30 s and re-finds the Call from the bus each time. `remaining_calls()` is the last-resort check that `GetCalls` is empty on every AudioGateway.
- `tools/real-phone/test_call.py`, run by `just test-call [--hold S] [--max-seconds S] [HOOK]`.
- `tools/real-phone/handoff_probe.py`: the ticket #3 probe, rebuilt on the shared module.
- `just check` now rejects any `Set(` under `tools/` without `signature=` inside its parentheses.

## 2. Results [V]

| Run | Scenario | Call appeared after Dial | `active` after Dial | `Hangup` returned in | Call gone after `Hangup` | Exit |
|---|---|---|---|---|---|---|
| 1 | no hook, hold 2 s | 0.61 s | 2.71 s | 0.035 s | 1.51 s | 0 |
| 2 | hook killed with `SIGKILL` | 0.87 s | 2.18 s | 0.040 s | 1.41 s | 1 |
| 3 | `SIGINT` to `test-call` | 0.66 s | 1.87 s | 0.075 s | 1.71 s | 130 |
| 4 | `SIGKILL` to `test-call` (guard hangs up) | 0.44 s | 1.75 s | 0.035 s | 1.51 s | killed |
| 5 | as run 4, after the guard fix below | 0.71 s | 1.82 s | 0.044 s | 1.51 s | killed |
| 6 | `handoff_probe.py activate-only` | 0.47 s | 1.89 s | 0.035 s | 2.22 s | 0 |

In run 6, `Set(RejectSCO, false)` through `set_property`, then `Activate()`, brought the audio to the Desktop in 0.228 s, matching the prototype report §2.3.

Before any Call, `set_property` was tried once with no Call active: it rejected a plain Python `bool`, a `dbus.Byte` and a `dbus.String` for `RejectSCO` (type `b`), and a write to the read-only `State`, all before sending anything. A correctly typed `dbus.Boolean(True)` went through, and WirePlumber stayed up.

## 3. Findings

1. **[I] Python leaves `SIGINT` ignored if it starts with it ignored,** which is how a non-interactive shell starts background jobs (Python's `signal` documentation; not tested without the fix). `test-call` therefore installs its own handlers for `SIGINT`, `SIGTERM` and `SIGHUP`. While hanging up, it logs and ignores further signals, so a second Ctrl-C cannot abandon the hang-up.
2. **[V] Killing `test-call` with `SIGKILL` orphaned the hook** (run 4), because the hook runs in its own process group so that it can be stopped cleanly. The guard now kills the hook's process group before hanging up (run 5: no hook left).
3. **[V] The prototype's retrying hang-up had a gap:** when the AudioGateway was absent (as during a WirePlumber restart), it found no Call and reported success. The shared module believes "no Call left" only while the Phone's AudioGateway is present, and otherwise keeps waiting until its 30 s deadline, then exits 2.
4. **[I] `org.ofono.VoiceCallManager.HangupAll` is deliberately not used** as a fallback. It sends `AT+CHUP`, which would also reject a real incoming Call that happened to be ringing.

## 4. Not tested

- Hanging up across a real WirePlumber restart mid-Call (the ticket #3 incident). It would mean restarting the user's audio stack on purpose. The code path is the same polling loop that ran in every row above, but the restart itself was not exercised.
- The refusal when a Call already exists, and the message when no AudioGateway is present.
- `handoff_probe.py trial-a`, `trial-b` and `pbap-snapshot` after the move. `activate-only` shares their setup, Call placement and hang-up.
