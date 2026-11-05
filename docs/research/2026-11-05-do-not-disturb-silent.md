# Do Not Disturb: the Banner shows, the Desktop stays silent

Date: 2026-11-05. Ticket #17. Builds on #96 (PR #100). Simulated Phone and stand-ins only: no sound was played on, and no volume changed on, the user's audio devices, and the user's Do Not Disturb was never touched.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** the rule is one condition in the Call state machine: `State::ringing` is false while Do Not Disturb is on. The ringer, `Ringing` and the Banner already followed that function, so the Banner still shows, the ringtone never starts under Do Not Disturb, and a toggle mid-ring stops or starts it. The Real-Phone Checklist's step 2.7 is now scripted, and the runner's test for "Answer or Decline pressed on the Desktop" no longer depends on `Ringing` under Do Not Disturb.

## The app

- `State::ringing` (`crates/core/src/machine.rs`) now reads `!do_not_disturb && a Call rings here && no Call is live`. `step` already compares `ringing()` before and after each input and emits `StartRinging` or `StopRinging`, so `Input::DoNotDisturbChanged` mid-ring starts or stops the ringer without new code. **[V]** (`crates/core/tests/do_not_disturb.rs`)
- The Banner is chosen by `Call::rings_here`, not by `ringing()`, so it shows under Do Not Disturb unchanged. No change in `surfaces/`. **[V]** (scenario 16)
- The Indicator's ringing look follows the Banner, not `ringing()`, so it still shows a ringing Call under Do Not Disturb. **[V]** by reading `State::indicator` in `crates/core/src/indicator.rs`. **[I]** that this is what the user wants; spec story 27 asks for attention while a Call rings, and says nothing about Do Not Disturb.
- Missed Call notifications are unchanged: urgency normal, no `transient` hint and no `sound-*` hint. So Omarchy 4.5 silences them under Do Not Disturb and files them to its notification history (`Service.qml:601-604`, `Delivery.js:37-41`, per the 4.5 recheck note). **[V]** for the hints, in the stand-in. **[I]** for Omarchy's history, which was not exercised on the real Desktop.

## Timings (scenario `turning_do_not_disturb_on_mid_ring_…`)

- The scenario asserts 250 ms from the moment the harness starts replacing `quiet.json` (its wall clock) to the app's `ringing-stopped` or `ringing-started` log line (the app's wall clock, same machine). Nothing but the app sits between: no D-Bus round trip, no test process start. **[V]**
- In one kept run on the Desktop, the app's own part, from `do-not-disturb-changed` to the ringer acting, was 0.6 ms to stop and 0.7 ms to start. The rest of the 250 ms is inotify delivery and the GLib main loop picking up the directory monitor's event. **[V]**
- Under `just test-ci-like` (CI's CPU quota, four scenarios at a time), all 8 `do_not_disturb` scenarios passed, the 250 ms toggle among them, and all 9 `missed_calls` scenarios. One run. **[V]**

## Seeing the ringtone in tests

- Scenarios see the ringtone through `Ringing` (published from `State::ringing`) and the ringer's own `ringing-started` and `ringing-stopped` log lines. Scenario 16 also checks that no `ringing-started` line is ever written. **[V]**
- Scenarios never reach the user's PipeWire: `XDG_RUNTIME_DIR` is the scenario's own, so `pw-play` finds no `pipewire-0` socket and fails at once, which the ringer logs as `ringing-failed`. **[V]** by reading `crates/test-harness/src/builder.rs`; **[I]** that `pw-play` has no other fallback, from PipeWire's documented lookup order (`PIPEWIRE_RUNTIME_DIR`, `XDG_RUNTIME_DIR`, `USERPROFILE`).
- A `pw-play` stand-in on the scenario's `PATH` would let a scenario see the player process itself start and be killed. Not built; the log lines answered every acceptance criterion. **[I]**

## The Real-Phone Checklist runner

- **The Answer-detection problem (#24's comment on #17).** `handled_on_desktop` decided "Answer or Decline pressed on the Desktop" by "the Desktop stops ringing while the Phone still rings". Under Do Not Disturb, `Ringing` is false from the start, so every step would have read a Desktop Answer the moment the Banner showed. **[V]** by reading the code.
- **The fix.** Without Do Not Disturb it still uses `Ringing`, which is published in the same `PropertiesChanged` as the Call's state, so it can't be confused with the Pixel answering. Under Do Not Disturb it uses the Banner closing while the Call still rings. **[V]** (checklist test, step 2.7)
- **The catch.** `Surfaces` is published from GTK's map and unmap, a moment after the Call's state, and `service.rs` awaits the Indicator between rendering the surfaces and publishing the state. So under Do Not Disturb a Call answered on the Pixel can, for that moment, look as if it was handled on the Desktop. Step 2.7 asks only for Decline on the Desktop, group 2's list asks for Do Not Disturb off, and 2.7 ends by asking the user to turn it off again. **[I]** that the moment is ever long enough to see; not reproduced.
- **Step 2.7** asks the user to turn Do Not Disturb on, has the helper call, checks the Banner shows within 1 s, checks `Ringing` stayed false from Do Not Disturb turning on to the end of the Call, asks only "Apart from the Pixel itself, was the Desktop silent while the Call rang?", and has the user decline on the Desktop and turn Do Not Disturb off. In `--demo`, `do-not-disturb-on` and `do-not-disturb-off` write the demo's own `quiet.json`. **[V]** (`crates/automation/tests/real_phone_checklist.rs`)

## Follow-ups

- A field in `Calls` saying a Call is being answered or declined on the Desktop would give the runner an atomic signal under Do Not Disturb too, and close the catch above. **[I]**
- `service.rs` renders the surfaces, then awaits the Indicator, then publishes the state. Publishing first would make `Surfaces` never run ahead of `Calls`. Not changed here: `service.rs` and app startup belong to #47. **[I]**
