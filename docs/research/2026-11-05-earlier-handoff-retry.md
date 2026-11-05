# Handoff to the Desktop: asking once more after 1 s

Date: 2026-11-05. Ticket #70, follow-up from #13 ([audio on the Phone](2026-11-05-audio-on-phone.md)).
Legend: **[V]** = verified on this machine: against the real Pixel 9a where it says so, otherwise against the Simulated Phone. **[I]** = inferred, not tested.

**Bottom line:** if the Phone hasn't started opening the audio link 1 s after `Activate`, the Desktop now asks once more, and 3 s from the request stays the limit. On the real Pixel this mattered more than #13 suggested. Every Handoff started about 0.4 s after the Call went active went past 1 s: 3 of 4 never started until the retry, which brought the audio in 1.21 to 1.25 s instead of over 3 s, and the 4th started at 723 ms. Started 2 to 5 s into the Call, the Pixel answered the first `Activate` in 144 to 274 ms. 7 of 7 Calls ended with the audio on the Desktop and `RejectSCO` true.

## 1. What changed

- **The Call state machine.** A Handoff to the Desktop (`Input::MoveAudio`, or answering on the Desktop) asks the Phone for `MoveAudioToDesktop` (`RejectSCO=false`, then `Activate`) and starts `Timer::HandoffTimedOut` at 3 s. When the Phone has carried out `Activate` (`PhoneCommandDone`), it starts `Timer::HandoffNotStarted` at 1 s. If that fires while the transport is neither `pending` nor `active`, the Phone is asked once more. The 3 s timer, from the request, stays the overall limit: when it fires, the Call Window shows `AudioNotMoved` and the Phone's audio button instruction. A refused `MoveAudioToDesktop` is asked again at once, as before, and a second refusal reports `AudioNotMoved` at once. "Moving audio here…" shows the whole time. **[V]** core tests, `crates/core/tests/handoff.rs`.
- **Against ADR-0005.** Its Consequences say the adapter "waits about 3 s for `State = "active"` and calls `Activate()` once more before reporting failure", so the worst case was 6 s. Now the second `Activate` comes 1 s after the first if the link hasn't started, and the failure comes at 3 s. A link that has started (`pending`) but isn't `active` by 3 s is no longer asked again: it fails at 3 s. No such link was seen on the Pixel in 12 timed Handoffs (#13 and below), where `active` always followed `pending` within 37 ms.
- **The 1 s timer starts when `Activate` returns**, not when the Desktop asks, so a slow `RejectSCO` write (WirePlumber busy) doesn't trigger a second `Activate` before the first was even sent.
- **The Simulated Phone** gains `SetActivateDelay(delay_ms)`: the Phone acts on each `Activate` that long after PipeWire's reply, as the Pixel took 62 to 313 ms to start the link. Quirks such as `activate-ignored-once` are used up when the Phone acts.
- **A real-Phone tool**, `tools/real-phone/handoff_timing.py`, runs as a `just test-call` hook. It starts the app, runs `desktop-calls audio here`, and timestamps every `RejectSCO` write, `Activate` and transport `State` change by watching the session bus as a monitor (`BecomeMonitor`, read-only). The app runs at its default log level, so no phone number reaches the journal. A dbus-python monitor's message filter must be added before `BecomeMonitor`; added after, it saw nothing. **[V]**

## 2. Against the Simulated Phone

`crates/test-harness/tests/handoff.rs`, 11 scenarios, all pass. **[V]**

- The Phone takes 300 ms to act on `Activate` and ignores the first: the audio reaches the Desktop in under 2.5 s, with the Call Window saying "Moving audio here…" for at least the first second. The Phone is asked `RejectSCO=false`, `Activate`, `RejectSCO=false`, `Activate`, `RejectSCO=true`. Against the core from `main`, the same scenario took 3.31 s and failed. **[V]**
- The Phone takes 300 ms and doesn't ignore it: exactly one `Activate`, and still one after a further second. **[V]**
- The Phone keeps the audio (`activate-ignored`): two `Activate`s, and the plain message no sooner than 3 s and within 4.5 s of asking (it was 6 s). The Call stays live. **[V]**

## 3. On the real Pixel

The method: `just test-call --max-seconds 40 '/usr/bin/python3 tools/real-phone/handoff_timing.py [--settle S]'`, 7 Calls to the Test Number, one at a time, on this branch's debug build. `test_call.py` places the Call with `RejectSCO` true, so its audio is on the Pixel, as for a Call placed on the Phone. Once the Call is active, the hook starts the app. When the app shows the Call with the audio on the Phone, it waits `S` seconds and runs `desktop-calls audio here`. Times are from running the CLI. The monitor's timestamps are accurate to about 20 ms, and the first `Set` and `Activate` show together because they arrive while the CLI runs.

| Call | Waited | `Activate` | `pending` | `active`, audio on Desktop | `Activate`s |
| --- | --- | --- | --- | --- | --- |
| 1 | 0 s | 29 ms, again 1041 ms | 1206 ms (+165 after the second) | 1237 ms, 1247 ms | 2 |
| 2 | 0 s | 28 ms, again 1036 ms | 1175 ms (+139) | 1211 ms, 1221 ms | 2 |
| 3 | 0 s | 28 ms, again 1039 ms | 1174 ms (+135) | 1210 ms, 1209 ms | 2 |
| 4 | 5 s | 28 ms | 294 ms (+266) | 330 ms, 336 ms | 1 |
| 5 | 5 s | 36 ms | 180 ms (+144) | 211 ms, 222 ms | 1 |
| 6 | 0 s | 35 ms | 758 ms (+723) | 790 ms, 805 ms | 1 |
| 7 | 2 s | 30 ms | 304 ms (+274) | 340 ms, 340 ms | 1 |

Each Call ended with `RejectSCO` true, `State` active and `Codec` 3, and `test_call.py` exited 0 with `GetCalls` empty.

- **[V]** With no wait, the Handoff started about 0.4 s after the Call went active. **[I]** That figure is the hook's run time minus its 4 s watch, for Calls 1 and 6. In 3 of 4 such Calls the first `Activate` led to nothing: no `pending` and no property change in the next second. The second `Activate`, 1 s later, started the link within 135 to 165 ms. Before this change, those 3 Handoffs would have waited until 3 s for the second `Activate`, so about 3.2 s.
- **[V]** In the 4th such Call (6), the link started 723 ms after `Activate`, more than twice the slowest in #13 (313 ms), but under 1 s, so there was no retry.
- **[V]** Started 2 or 5 s into the Call, the first `Activate` started the link within 144 to 274 ms, as in #13.
- **[I]** So the Pixel tends to ignore or delay an `Activate` sent within about a second of the Call going active. That fits #13's 2790 ms Call and ADR-0005's ignored `Activate`. It matters most for **answering on the Desktop**, where `Activate` follows `Answer` at once. That case wasn't measured on the Pixel, because it needs an incoming Call from a Helper (Real-Phone Checklist).
- **[I]** In #13 (5 of 5 within 313 ms) the app ran at debug level and its startup was a little slower. That may have put the Handoff past this window.
- **[V]** PipeWire emits `PropertiesChanged` for `RejectSCO` only shortly before the transport goes `pending`, 20 to 50 ms before it, not when it is written. It isn't evidence of when the write took effect.

## 4. Not checked

- **[I]** What PipeWire's `hfp_hf_transport_activate` does with a second `Activate` while the Phone is still answering the first; the source host refused the automated fetch. The retry only goes out while the transport is still `idle`.
