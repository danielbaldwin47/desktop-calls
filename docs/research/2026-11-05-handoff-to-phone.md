# Handoff to the Phone: the hands-free bounce in the app

Date: 2026-11-05. Ticket #14, building on [the prototype](2026-11-04-handoff-to-phone-prototype.md) and [ADR-0005](../adr/0005-handoff-to-the-phone-bounces-the-hands-free-profile.md).
Legend: **[V]** = verified on this machine: against the real Pixel 9a where it says so, otherwise against the Simulated Phone. **[I]** = inferred, not tested.

**Bottom line:** "Move audio to phone", `MoveAudio("phone")` and `desktop-calls audio phone` now drop hands-free and connect it again through BlueZ, and the Call carries on with the same Call id, timer and controls. On the real Pixel, one Call to the Test Number went here, phone, here, phone. Each Handoff to the Phone moved the audio off the Desktop in 29 ms, and the Call was back in 846 and 1024 ms. Each Handoff here after a bounce took 219 and 281 ms. `RejectSCO` stayed true, and the Phone stayed paired and connected throughout.

## 1. What changed

- **The Call state machine.** `Input::MoveAudio(Phone)` with the audio on the Desktop asks the Phone for `PhoneCommand::MoveAudioToPhone`. It also starts `Timer::HandoffTimedOut` at 5 s (`HANDOFF_TO_PHONE_TIMEOUT_MS`), and the Call Window shows "Moving audio to phone…". **[V]** core tests, `crates/core/tests/handoff.rs`.
  - While hands-free is away, `PhoneDisconnected` doesn't end the Call or show the Phone as disconnected. The audio counts as on the Phone from then on.
  - The Handoff ends when the AudioGateway is back and the Call has been seen again. The AudioGateway coming back sets `KeepAudioOnPhone` (`RejectSCO` true) again, and its Contacts are not synced again.
  - A Handoff asked for while one is in progress does nothing.
- **Failures.**
  - A bounce BlueZ refuses before anything dropped leaves the audio here. The Call Window then shows `Failure::AudioNotMovedToPhone`: "The audio stayed here; to move it to your phone, tap the audio button on your phone's call screen." The Call stays live. **[V]**
  - If hands-free isn't back 5 s after a drop, the Phone shows as disconnected, as the Phone link's own grace would have it. **[V]** core.
- **The Phone link** resolves `MoveAudioToPhone` to the connected Phone's address. `bluez::bounce_hands_free` finds its `Device1` by `Address` through `GetManagedObjects`, then calls `DisconnectProfile` and `ConnectProfile` with the hands-free UUID (signature `s`). `AlreadyConnected` and `InProgress` from `ConnectProfile` count as success. Nothing else of `Device1`'s is called.
  - The Calls that vanish come back through the existing grace (2 s) as the same Calls.
  - A hang-up during the gap waits for the Call to come back. **[V]** harness.
  - The command execution moved to `phone_link/commands.rs` to keep `phone_link.rs` under 1,000 lines.
- **The Simulated Phone** answers `Device1.DisconnectProfile` and `ConnectProfile` for the hands-free UUID with the prototype's sequence (§3.2), on its own thread:
  1. It removes the Calls, then the AudioGateway, then `Battery1`. `Device1.Connected` stays true.
  2. It replies to `ConnectProfile` after 150 ms.
  3. The AudioGateway comes back at the same path 600 ms after the drop, with `RejectSCO` true.
  4. The Calls come back 40 ms later, already `active`.
  5. The Phone then makes two refused audio link tries (Codec 3, then 1, with State staying `idle`).

  A new quirk, `disconnect-profile-refused`, has BlueZ reply `org.bluez.Error.Failed`.
- **Not against ADR-0005.** It said nothing about how long to wait for hands-free to come back. The 5 s limit and showing the Phone as connected during the bounce are new.

## 2. Against the Simulated Phone

`crates/test-harness/tests/handoff.rs` has 16 scenarios, and all pass. **[V]** They are:

- The Handoff to the Phone by `MoveAudio("phone")`, by `desktop-calls audio phone` and by the button. Each one is checked for:
  - the same Call id, `active` state, timer and connection;
  - "Audio on phone" and "Move audio here" shown;
  - exactly `DisconnectProfile` then `ConnectProfile` on the Phone's `Device1`, with the hands-free UUID;
  - no `Hangup`, `Disconnect`, `RemoveDevice` or `Deactivate`;
  - `RejectSCO` true;
  - `Device1.Paired` and `Connected` true;
  - the app's own part within 500 ms by its log.
- Scenario 12: here, phone, here, phone in one Call, with one `Activate` per Handoff here.
- The no-`Deactivate()` quirk: the transport introspects without `Deactivate`, and the Handoff to the Phone works without it.
- BlueZ refusing the bounce gives the plain message and the `audio-not-moved-to-phone` error, and the Call stays live.
- A hang-up while hands-free is away ends the Call once it is back, and the Phone stays connected.

## 3. On the real Pixel

The tool is `tools/real-phone/handoff_to_phone.py`.

- **No Call, one bounce** (`--bounce-only`, which refuses if any Call exists). Both signatures were checked against BlueZ's introspection first. `DisconnectProfile` replied in 1 ms, and the AudioGateway was gone at once. `ConnectProfile` replied at 396 ms, and the AudioGateway was back at 477 ms at the same path (`/org/pipewire/Telephony/ag1`). Before and after: Paired true, Connected true, `RejectSCO` true. **[V]**
- **One Call to the Test Number**, run with `just test-call --max-seconds 90 '/usr/bin/python3 tools/real-phone/handoff_to_phone.py'` on this branch's debug build. The hook started the app, then ran `desktop-calls audio here|phone` with 3 s between steps. Times are from running the CLI. **[V]**

| Step | `AudioLocation` changed | Call back on the Phone | After |
| --- | --- | --- | --- |
| here | 219 ms | n/a | |
| phone | 29 ms | 1024 ms | `RejectSCO` true, Paired, Connected |
| here | 281 ms | n/a | |
| phone | 29 ms | 846 ms | `RejectSCO` true, Paired, Connected |

The app showed one Call, id 1, `active`, with the same connect time from start to end. `test_call.py` hung up the re-added Call by its `LineIdentification` (via `Call1.Hangup`, 34 ms), and `GetCalls` was then empty. Exit status 0.

- The Handoff here after a bounce works on the AudioGateway that came back, and its timing matches the 2-5 s band in [#70's note](2026-11-05-earlier-handoff-retry.md). **[V]**
- The tool's "AudioGateway gone at" only catches the drop if its first poll sees it: "never" on the first step to the Phone means the poll missed it, not that nothing dropped.

## 4. Not verified

- **The earpiece by ear.** That the audio actually lands on the Pixel's earpiece is Real-Phone Checklist step 3.1, for the user.
- **`ConnectProfile` failing on the real Phone**, and how a Call left live on the Phone then looks. **[I]** After the drop, the Desktop shows the Phone as disconnected once 5 s pass, and the Call ends in the app after the 2 s grace while it may still be live on the Phone.
- **A bounce while the Call is still `alerting`** (placed on the Desktop, audio here). **[I]** It should work the same way, since the Calls come back in whatever state they had.
- **Mute held during the bounce.** **[I]** A `SetMicrophoneVolume` sent while hands-free is away fails, as any command to a missing AudioGateway does.
