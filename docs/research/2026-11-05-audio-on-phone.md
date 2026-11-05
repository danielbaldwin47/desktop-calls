# Audio on the Phone: Calls placed on the Phone, and Handoff to the Desktop

Date: 2026-11-05. Ticket #13, part 1: Calls placed on the Phone and Handoff to the Desktop. The earbuds part comes in a second PR.
Legend: **[V]** = verified on this machine: against the real Pixel 9a where it says so, otherwise against the Simulated Phone. **[I]** = inferred, not tested.

**Bottom line:** this branch's app moved a live Call's audio from the Pixel to the Desktop on 8 of 8 real Calls. 7 times, the Call Window and `AudioLocation` read the Desktop within 459 ms; once, it took 2790 ms. In the 5 Calls logged at debug level, all of the time after `Activate` returned was the Phone bringing up the audio link, not the app. Once the audio was here, the app set `RejectSCO` back to true, and the audio link stayed `active`. The Desktop waits 3 s for each attempt, asks once more, and then says plainly how to move the audio on the Phone. The Call is never dropped.

## 1. On the real Pixel

The method: `tools/real-phone/test_call.py`, run under `/usr/bin/python3` without `--desktop-audio`, placed the Call to the Test Number. So `RejectSCO` stayed true, and the Call had its audio on the Pixel, as a Call placed on the Phone does. The hook started `target/debug/desktop-calls service` with no display. It waited for `AudioLocation` "phone", ran `desktop-calls audio here`, and polled `AudioLocation` every 50 ms. Then it read `RejectSCO`, `State` and `Codec` on the AudioGateway with `busctl`. Calls 4 to 8 ran the app with `RUST_LOG=desktop_calls=debug`, so the journal has the time `Activate` returned ("The Phone did MoveAudioToDesktop") and each transport change.

| Call | `AudioLocation` desktop after | `Activate` returned after | Then `pending` | Then `active` |
| --- | --- | --- | --- | --- |
| 1 | 2790 ms | not logged | not logged | not logged |
| 2 | 248 ms | not logged | not logged | not logged |
| 3 | 354 ms | not logged | not logged | not logged |
| 4 | 196 ms | 69 ms | +62 ms | +37 ms |
| 5 | 459 ms | 62 ms | +313 ms | +36 ms |
| 6 | 302 ms | 49 ms | +167 ms | +35 ms |
| 7 | 409 ms | 49 ms | +274 ms | +35 ms |
| 8 | 251 ms | 63 ms | +110 ms | +35 ms |

Each Call ended with `RejectSCO` true, `State` active and `Codec` 3, and test_call.py hung up with exit 0.

- **[V]** The app found the Call already active on start, with the audio on the Pixel (`State` idle, `RejectSCO` true), and didn't touch the audio until asked.
- **[V]** The time includes starting the CLI and its D-Bus round trip, which with both `RejectSCO=false` and `Activate` took 49 to 69 ms. The rest is the Pixel: 62 to 313 ms before the link goes `pending`, then about 35 ms to `active`.
- **The 2790 ms Call.** Its journal holds only info lines, which say nothing between start and stop, so it can't show which step was slow. The 3 s retry didn't fire, because 2790 ms is under 3000. **[I]** It was the Pixel being slow to open the link, as in ADR-0005's 2.9 s case. That is 1 in 8 here and 1 in 7 there. PipeWire answers `Activate` without waiting for the Phone, so the app can't slow that step down.
- **What the app could do better (not in this PR):**
  1. Retry earlier when the transport hasn't even gone `pending`. In every logged Call, `pending` came within 313 ms of `Activate`. A retry about 1 s after `Activate` with no `pending` would catch an ignored `Activate` 2 s sooner, while the 3 s limit still covers a link that is slow once it starts. Whether a second `Activate` speeds up a slow Pixel is untested.
  2. The Call Window already says "Moving audio here…" as soon as it is asked, so a slow Pixel shows progress rather than nothing.
  3. **[I]** A second `RejectSCO=false` and `Activate` sent while the link is `pending` should be harmless, but what `hfp_hf_transport_activate` does with a `pending` transport isn't checked.
- **[V]** `desktop-calls audio phone` printed "Tap the audio button on your phone's call screen to move the audio to your phone." It exited 1 and left the audio on the Desktop.
- **[V]** With no Call, `desktop-calls audio here` printed "There's no connected Call right now." and exited 1.
- Not tried on the real Pixel: a Phone that ignores `Activate` twice, because the Pixel can't be made to. The Simulated Phone covers it (section 3).

## 2. What changed

- **The Call state machine** has `Input::MoveAudio(AudioLocation)`. To the Desktop, it asks the Phone for `MoveAudioToDesktop`, which is `RejectSCO=false` then `Activate`, and starts `Timer::HandoffTimedOut` (3 s). When the transport goes `active`, it asks for `KeepAudioOnPhone` (`RejectSCO=true`) to re-arm, per ADR-0005. If the timer fires or the Phone refuses, it asks once more. After the second attempt it reports `AudioNotMoved` and shows it in the Call Window until the audio moves, the user asks again, or the Call ends. `RejectSCO` stays false then, so the Phone's own audio button can still send the audio here. Answering on the Desktop uses the same wait and retry.
- **To the Phone:** this is refused with `HandoffToPhoneUnavailable` and the instruction for the Phone's audio button. The Call Window shows the instruction until the audio leaves the Desktop, and #14 replaces it.
- **The Call Window** says where the audio is, in words, whenever there is a Call: while "Calling…" and "Ringing…" too, not only once connected. A Handoff button next to it names where the audio will go: "Move audio here", "Move audio to phone", or "Moving audio here…", disabled, while the Desktop waits. Its shortcut is A. Both are as wide as their widest text, so the window doesn't change width as the audio moves. Before that fix, an Omarchy-theme scenario failed 1 run in 6, because the window narrowed between two screenshots. **[V]**
- **Control interface:** `MoveAudio(s)` takes "desktop" or "phone". **CLI:** `desktop-calls audio here|phone`.
- **The Simulated Phone:** `DialOnPhone(number)` places a Call on the Phone itself (`dialing`, audio stays put). The quirks `activate-ignored` and `activate-ignored-once` make `Activate` reply and do nothing.

## 3. Against the Simulated Phone

`crates/test-harness/tests/handoff.rs`, 10 scenarios, all on a headless screen, all pass. **[V]**
- Scenario 11: a Call placed on the Phone shows "Calling…" and "Audio on phone", and for a second the Phone is asked for neither `RejectSCO=false` nor `Activate`.
- The Handoff through the button, `MoveAudio("desktop")` and `desktop-calls audio here`, during a Call placed on the Phone and an incoming Call answered on the Phone. In each, the Phone is asked for `RejectSCO=false`, `Activate`, `RejectSCO=true`, and nothing else. The Call Window reads "Audio on this computer" and "Move audio to phone" within 1.5 s.
- `activate-ignored-once`: two attempts, then the audio arrives with no message. `activate-ignored`: the plain message with the fallback, the `audio-not-moved` error, and the Call still live, with no `Hangup` asked.
