# Connected and Muted only once the Phone has done it

Date: 2026-11-05. No ticket: CI run 41972547257 on PR #56 failed twice, in `connecting_rejects_desktop_audio_until_a_call_is_answered_here` (attempt 1) and `set_muted_sets_the_microphone_volume_to_zero_and_back` (attempt 2), and CI run 41974164075 on PR #58 failed in `just screenshot` at `gallery.rs:135`. Only the Simulated Phone was used.
Legend: **[V]** = verified on this machine against the Simulated Phone. **[I]** = inferred, not tested.

**Bottom line:** the two test flakes were the app reporting a state before the Phone had done what that state promises. The service published `connected` in the same step that queued `RejectSCO=true`, and showed Muted in the same step that queued `MicrophoneVolume=0`. The app now waits for the Phone's reply to the D-Bus call before reporting either. The test harness no longer offers a snapshot of the Phone's recorded method calls, only waiting checks. The gallery flake was the gallery's own race: it pressed Mute before the audio reached the Desktop, and the app rightly refused.

## 1. Cause

- `service::decide` runs the Call state machine, hands the step's Phone commands to the Phone link's channel, then publishes the new state at once. The Phone link sends the D-Bus call later, on its own task. So `ConnectionState` read `connected`, and `Muted` read `true`, while the call had not even left the app. **[V]** (code reading, then the loop below)
- With a 300 ms delay put in front of the `RejectSCO` write, the old read-once test failed 5 of 5 with CI's message: "the Phone was never asked to set RejectSCO=true on connect". Without the delay, on one CPU shared with two niced busy loops, it failed 1 of 20. **[V]**
- This was an ordering bug in the app, not only a test race: until `RejectSCO=true` reaches PipeWire, a Call that arrives can open its audio on the Desktop, and the app was already saying the Phone was ready. In practice `bluez5.telephony.default-reject-sco = true` makes a new AudioGateway start with `RejectSCO=true`, so the window matters most when something left it false. **[I]**

## 2. What changed

- The Phone link now reports each command the Phone carried out (`Input::PhoneCommandDone`), next to the existing `PhoneCommandFailed`.
- **Connect:** on `PhoneConnected`, the state machine asks for `KeepAudioOnPhone` and shows `connected` once the Phone has done it. If the Phone refuses, it shows `connected` anyway, because the Phone is there and the next Call end or connect asks again. If hands-free drops first, it doesn't show `connected`. With Call audio already on the Desktop (an AudioGateway replaced mid-Call), it shows `connected` at once, as before.
- **Mute:** Muted shows only once the Phone has confirmed. Mute shows Muted only after `MicrophoneVolume=0` is done, and Unmute shows unmuted only after the gain is back. The UX choice: showing Muted while the other person can still hear is the worst thing a Mute control can do, and showing unmuted before the microphone is back on loses the user's first words. The cost is the local D-Bus round trip before the button changes. **[I]** that this round trip is too short to notice: PipeWire replies to `Properties.Set` without waiting for the Phone (`telephony.c:862`, see the [mute note](2026-11-04-mute-and-keypad.md)).
- When the audio leaves the Desktop or the Call ends, the gain still goes back without waiting, as there is no Mute left to show.

## 3. The harness

- `SimulatedPhone::recorded_method_calls()` is now private. Tests check the Phone's calls only with `wait_for_recorded` (until the calls match, with a deadline) or the new `recorded_holds` (the calls keep matching for a window, for something the Phone must not be asked). Every snapshot in `crates/test-harness/tests/` was rewritten: 24 sites in 8 files, including the 3 in `contacts.rs` that came with Contacts sync (#52).
- The Simulated Phone has a new quirk, `slow-property-writes`: it records a `Properties.Set` at once but replies a second later. Two scenarios use it to hold the app between the call and the reply: `the_phone_shows_connected_only_once_desktop_audio_is_rejected` and `muted_shows_only_what_the_phone_has_done`. Both failed before the fix and pass after it. **[V]**

## 4. The gallery

- `gallery.rs:135` called `SetMuted` once the Call Window showed, and the app answered `MuteUnavailable`: the Call Window opens, reading "Mute", before the Audio Location reaches the Desktop, and Mute is refused until it has. The muted state now waits for `audio_location == "desktop"` and a Mute button that isn't marked unavailable; the live state also waits for the Desktop. **[V]** (code reading, then the runs below)
- Every other action in `gallery.rs` follows a wait on the state it needs, except the click that gives the Call Window the keyboard before typing tones. Focus has no observable signal in the app's state, so it stays a click after the redraw, and the next wait (the keypad shows `123#`) fails with a message if the keys were lost. **[I]** that this can't flake the way `SetMuted` did: the click lands after the screenshot shows the muted strip.

## 5. Proof

All on one CPU shared with two niced busy loops, against the Simulated Phone, after merging main (#52). **[V]**

- `connecting_rejects_desktop_audio_until_a_call_is_answered_here`: 30 of 30 pass.
- `set_muted_sets_the_microphone_volume_to_zero_and_back`: 30 of 30 pass.
- `incoming_call.rs` and `mute_and_tones.rs`, whole files: 10 of 10 runs pass.
- `just screenshot` (`automation screenshot`): 10 of 10 runs pass, each with 24 screenshots of 6 states.
- `just check` passes; `just test --no-fail-fast` without load: 155 of 155 pass.
- Not tested: the Real Phone. The Mute round trip being too short to notice is inferred from PipeWire's code, not measured.
