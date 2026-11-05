# Hands-free not back after a Handoff to the Phone

Date: 2026-11-05. Ticket #101, building on [#14's Handoff to the Phone](2026-11-05-handoff-to-phone.md) and [#19's out of range](2026-11-05-out-of-range.md).
Legend: **[V]** = verified against the Simulated Phone. **[I]** = inferred, not tested. Nothing in this note was tried on the real Pixel.

**Bottom line:** When hands-free drops for a Handoff to the Phone and doesn't come back, the app no longer shows "Call ended". It treats the Call the way #19 treats a Call whose Phone went out of range. The Call Window says "Connection to phone lost: the Call continues on your phone." and keeps the timer. The Desktop asks BlueZ for hands-free again with `ConnectProfile` after 1 s, 2 s, then every 3 s. When hands-free returns with the Call, the Call Window shows it live again with its first timer, audio on the Phone. **[V]**

## Decisions

- **Lost means one of three things after the drop:** `ConnectProfile` fails, the Handoff's 5 s limit passes, or the Phone link's 2 s grace gives up on the Call first. Any of them makes hands-free lost. Before this, the first two showed the Phone disconnected, and the grace then ended the Call with "Call ended". **[V]** (core tests, `crates/core/tests/handoff_recovery.rs`)
- **It reuses #19's lost-Call handling, with no second path.**
  - `lose_calls` puts the Call Window into its lost state, and `lost_track` drops the Call quietly when the Phone link gives up: no "Call ended", no Missed Call, no Call History refresh.
  - `found_lost_call` matches the Call when it returns, by id inside the grace and by caller after it, and `earlier_calls` carries the timer.
  - The window closes 10 s after the loss if hands-free is still away.
  - Hang-up, mute, tones and Handoff are refused with the lost message.
  - The Phone link's "Call ended" condition is now "the Calls are out of reach": the Phone is absent, or hands-free is lost.
- **Presence holds the reason as `Reach` (`Present`, `Absent`, `HandsFreeLost`).** While hands-free is lost, the reconnect backoff, suspend (no asking) and resume (ask at once) work as they do for an absent Phone. If BlueZ then reports the Phone absent, #19's `Device1.Connect` takes over. **[V]** core.
- **The retry is `Device1.ConnectProfile(HFP AG)`, not `Device1.Connect`.**
  - The Phone is still connected; only the profile is missing.
  - `ConnectProfile` with this UUID is the call ADR-0005 already allows, and it brought hands-free back on the Pixel in 0.4 s (#14, no Call).
  - What `Connect` does on an already-connected device with one profile down is unverified **[I]**.
  - `AlreadyConnected` and `InProgress` count as success, as in the bounce.
  - It is a new Phone command, `ConnectHandsFree`, carried out by the Phone link through `bluez::connect_hands_free`.
- **The bounce's own failed `ConnectProfile` counts as the first failed connect**, so the first retry waits 1 s rather than going at once.
- **The connection state is `disconnected` while hands-free is lost**, as #14 had it. So the Indicator says "<Phone> out of range" though the Phone is in range. That is inaccurate, and a follow-up candidate.

## Simulated Phone

- Two new quirks:
  - `connect-profile-fails`: every `ConnectProfile` after a `DisconnectProfile` replies `org.bluez.Error.Failed` "br-connection-refused" after 150 ms. Hands-free stays down, and the Calls carry on on the Phone.
  - `connect-profile-fails-once`: the next one only. A later `ConnectProfile` brings hands-free and the Calls back with the usual timing.
  - The error name and text are **[I]**: BlueZ's `br-connection-*` family, not seen from the Pixel.
- PipeWire's late `ModemRemoved` isn't sent when `ConnectProfile` fails. The app ignores it either way.
- `CallsOnPhone` now also lists the Calls away with a dropped hands-free, not only those away out of range.

## Scenarios

`crates/test-harness/tests/handoff_recovery.rs`, both passing **[V]**:

1. Failing once:
   - The lost message shows, then the Call is back: same number, same connect time, audio on the Phone, connected, no notice.
   - BlueZ was asked `DisconnectProfile`, then `ConnectProfile` twice. There was never a "Call ended" and no notification, and the audio can then be moved here again.
2. Failing always:
   - The lost message shows, the Call Window keeps the Call's id and timer, the state is `disconnected`, and `MoveAudio` is refused with the message.
   - The Desktop asks for hands-free at least twice more, and the Call is still active on the Phone.
   - The window closes after about 10 s, never having said "Call ended", with no notification.

Gallery state `call-window-connection-lost` shows the lost Call Window in all four looks.

## Real-Phone Checklist

No item added. The ticket asks for one only if the failure can be provoked safely. Nothing safe makes BlueZ's `ConnectProfile` fail within the 0.5 s between the drop and the reconnect. Turning off the Pixel's "Phone calls" toggle for the Desktop drops hands-free on the Phone's side instead, which is a different path (below). Unpairing and Bluetooth off are ruled out.

## Not covered

- **Hands-free dropped by the Phone mid-Call without a Handoff**, with `Device1.Connected` still true (for example the Pixel's per-device "Phone calls" toggle switched off). This still ends the Call in the app after the 2 s grace. **[I]** from the code path. Retrying there would fight the user's own setting, so it needs its own decision.
- **The lost Call Window's controls** (#19's surface): Hang up stays red and clickable but is refused, and the "Mute works only while…" hint shows beside the lost message.
