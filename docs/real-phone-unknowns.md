# Real-Phone unknowns

What the Simulated Phone or a Real-Phone Checklist step assumes about the real Phone and its stack (the Pixel 9a, BlueZ, obexd, PipeWire and WirePlumber) without a real observation behind it. The app's own "inferred" marks (in `crates/core` and `crates/desktop-calls`) are outside it for now. Started 2026-11-15 from retro candidate 3 of [2026-11-15](research/2026-11-15-retro.md): the first sitting failed three times on behaviour inferred from documents, and the research notes held 268 `[I]` marks that no list tracked.

## Rules

- **Code that says "inferred" or "unverified"** about the real stack, in `crates/simulated-phone/src` or the checklist's steps (`crates/automation/src/checklist/steps.rs` and `steps/`), names the entry here within two lines of the word, as `RPU-7`. The lint reads only those two words; any other assumption ("a guess", "not seen") gets an entry and an id by the same rule, unenforced. `just check` fails otherwise, and on an id with no entry, an entry with no "Observed by", an entry whose step doesn't exist, and code that still calls an observed entry inferred.
- **A step that rests on an open unknown observes and never judges it.** What the Phone does there is recorded with `Sitting::observed`, or with `Sitting::unexpected` ("observed, unexpected") where the step had expected something; it never fails the step on that point alone. Failing on it is what sent the owner to re-pair three times on 2026-11-15. A step that still fails where an open unknown is involved says why under `- Judged:` in the entry: the failure is the app's or the step's purpose not met, never the Phone differing from the assumption.
- **Before "agent-side complete"** for a sitting, `just real-phone-unknowns <group>` lists the open unknowns on that group's path, and the message to the owner names them.
- **When a sitting observes one,** set its Status to `observed <date> (<issue or note>): <what was seen>`, correct the Simulated Phone or step that assumed it, and drop "inferred" from the code that cites it. A new assumption gets an entry in the same PR that makes it.

Each entry: `## RPU-<n>: <title>`, then `- Assumed:`, `- Source:`, `- Observed by:` (checklist step ids, such as `1.3`, or `no step yet`) and `- Status:` (`open`, or `observed ...`), and `- Judged:` where the rule above needs it. Ids are never reused.

## Pairing and BlueZ

## RPU-1: Pairing times
- Assumed: discovery finds the Pixel, the page is answered and hands-free connects after pairing within the Simulated Phone's `FOUND_AFTER`, `PAGE_ANSWERED` and connect times.
- Source: `crates/simulated-phone/src/pairing.rs` header; retro A5.
- Observed by: no step yet (1.2 times only the whole, from the Setup Window's first step to the first Call)
- Status: open

## RPU-2: A Phone that refuses Calls fails `ConnectProfile` with `br-connection-refused`
- Assumed: with the Pixel's "Phone calls" switch off for this Desktop, `ConnectProfile` for hands-free fails `org.bluez.Error.Failed` `"br-connection-refused"`.
- Source: `crates/simulated-phone/src/pairing.rs` header; `docs/research/2026-11-05-handsfree-not-back.md`.
- Observed by: no step yet
- Status: open

## RPU-3: A removed Phone is found again by discovery
- Assumed: a Phone removed on the Desktop, on its "Pair new device" screen, is found again by discovery under way, whether it was removed in range or away and back.
- Source: `crates/simulated-phone/src/pairing.rs` header.
- Observed by: 1.3
- Status: open

## RPU-37: A Phone the Desktop alone forgot stays quiet
- Assumed: a Pixel that kept its pairing after the Desktop removed it doesn't page the Desktop when back in range, so the app sees only a nearby Phone that isn't paired; a real bonded Phone may try to reconnect, and BlueZ then asks the pairing agent or refuses it.
- Source: `crates/automation/src/checklist/sitting/demo.rs` (`desktop-removes-phone`); `crates/automation/tests/real_phone_checklist_leftover_state.rs`.
- Observed by: no step yet (no step asks the owner to remove the Phone on the Desktop only)
- Status: open

## RPU-4: `RemoveDevice` with hands-free up disconnects and succeeds
- Assumed: real BlueZ takes a Phone with hands-free up down and removes it; the Simulated Phone fails it instead, and "Pair again" (#382) rests on the simulation.
- Source: `crates/simulated-phone/src/pairing.rs` header; retro A5; `docs/research/2026-11-05-two-phones.md`.
- Observed by: 1.1, 1.3
- Status: open

## RPU-5: A Phone that forgot the pairing fails `Connect` with `br-connection-key-missing`
- Assumed: `Device1.Connect` to a Phone that forgot the pairing fails `"br-connection-key-missing"` (from BlueZ's error table), and lost pairings show only as key-missing, `AuthenticationFailed` or `AuthenticationRejected`.
- Source: `crates/simulated-phone/src/range.rs` header; `docs/research/2026-11-05-out-of-range.md`.
- Observed by: no step yet
- Status: open

## RPU-6: On key-missing BlueZ starts a pairing of its own
- Assumed: real BlueZ answers key-missing by starting a pairing, which with no agent the Pixel shows as a wrong PIN (seen in a journal, #373); the Simulated Phone starts none.
- Source: retro A5; #373.
- Observed by: no step yet
- Status: open

## RPU-7: `Paired` is signalled late, once services resolve
- Assumed: BlueZ 5.87 holds back the `Paired` signal until the Phone's services resolve, then sends it once; a late `Paired` after the app moved on by `Bonded` is untested.
- Source: `crates/simulated-phone/src/main.rs` (`--signal-held Paired`); retro A5; #372.
- Observed by: 1.2
- Status: open

## RPU-8: Every `Device1` property the app reads arrives as a signal
- Assumed: each `Device1` property the app depends on is signalled when it changes, as `Paired` turned out not to be (#372); `InterfacesAdded` names what `GetManagedObjects` lists, and Phones list in a stable order. With `--signal-held`, scenarios hold back `Paired`, `Bonded` and `Connected` going true and the app catches up; `UUIDs`, `Name`, `Alias` and `Class` never change in the Simulated Phone once a Phone is listed, so their read-versus-signal split is untested.
- Source: retro A5; `docs/research/2026-11-04-test-seam.md`; `docs/research/2026-11-05-two-phones.md`.
- Observed by: no step yet
- Status: open

## RPU-9: A `Connect` paging when the adapter goes down fails `NotReady`
- Assumed: a `Device1.Connect` still paging when the adapter is powered off fails as one made while it is down: `org.bluez.Error.NotReady` `"br-connection-adapter-not-powered"`.
- Source: `crates/simulated-phone/src/desktop.rs` header.
- Observed by: no step yet (6.2 suspends the Desktop once the Phone is connected, never while a `Connect` pages)
- Status: open

## RPU-10: BlueZ's `Telephony1` child path
- Assumed: the experimental `hfp` plugin's `org.bluez.Telephony1` object sits at the path the Simulated Phone gives it; its last part is a guess.
- Source: `crates/simulated-phone/src/desktop.rs` header.
- Observed by: no step yet
- Status: open

## RPU-11: A `Connect` that is never answered
- Assumed: the real Desktop's app once waited on a `Device1.Connect` for good (#216); the Simulated Phone's `ConnectNeverAnswersOnce` models that shape.
- Source: `crates/simulated-phone/src/telephony.rs` (`ConnectNeverAnswersOnce`); #216.
- Observed by: 6.1
- Status: open

## RPU-12: `ConnectProfile` errors on a Phone with unresolved services
- Assumed: hands-free `ConnectProfile` on the Pixel fails with a `br-connection-*` error, and `Connect` on a Phone with one profile down brings it up; the error for unresolved services is unknown.
- Source: `docs/research/2026-11-05-handsfree-not-back.md`; #385; retro A5.
- Observed by: 6.7
- Status: open

## RPU-13: Calls off: reconnecting an unblocked Pixel
- Assumed: after Calls Off, `Connect` on the unblocked Pixel reconnects in time, `br-connection-busy` is possible when the Pixel pages at the same moment, and WirePlumber brings hands-free back after every reconnect.
- Source: `docs/research/2026-11-13-calls-off-mechanism-probe.md`.
- Observed by: 6.4
- Status: open

## Contacts and obexd

## RPU-14: The Pixel's Contacts prompt
- Assumed: what moves the Pixel between sharing Contacts, refusing them and asking is the Pair dialog's box and the per-Desktop Contacts switch, and with the box unticked it may ask rather than refuse (1.3's premise until #392). When it asks, the prompt lasts about 30 s, asks again on the next session if unanswered, and has no "Don't ask again". The Pixel 9a showed no prompt in two sittings: it shared at pairing with the box ticked, and refused at once with it unticked (#380, #392).
- Source: `docs/research/2026-11-05-setup-contacts.md`; `crates/automation/src/checklist/steps/first_run.rs` (`PROMPT_DEADLINE`); retro B2 and "Open questions".
- Observed by: 1.2, 1.3
- Status: open
- Judged: 1.2 fails when a prompt times out before Allow is tapped, because it then has no first Call to time. 1.3 records it as observed, unexpected.

## RPU-15: The Pair dialog's Contacts box wording
- Assumed: the Pixel's Pair dialog words its box "Allow access to your contacts and call history", as the runner quotes it.
- Source: `crates/automation/src/checklist/steps/first_run.rs` (`CONTACTS_BOX`).
- Observed by: 1.2, 1.3
- Status: open

## RPU-16: Other refusals reach obexd as known errors
- Assumed: a refusal other than the one seen (`Transport got disconnected`) arrives as `Forbidden`, `Unauthorized` or a plain `Failed` the app reads as not shared.
- Source: `docs/research/2026-11-04-contacts-sync.md`.
- Observed by: 1.3
- Status: open
- Judged: 1.3 fails a finish page that says Contacts aren't shared "yet", and one with no `denied` line in the log, because the same page follows the Desktop's own sync failing, which the step must not pass. Telling the two apart from the log is #415's item 5.

## RPU-17: obexd when the link drops during the prompt
- Assumed: real obexd fails a `CreateSession` waiting on the prompt with `Failed` when the Phone goes out of range, and a later pull starts cleanly.
- Source: `docs/research/2026-11-05-setup-contacts.md`.
- Observed by: no step yet (6.1 takes the Pixel out of range with no prompt up)
- Status: open

## RPU-18: The Pixel's vCards
- Assumed: the Pixel's Contacts hold no owner card the app mishandles, and its vCards decode cleanly.
- Source: `docs/research/2026-11-04-contacts-sync.md`.
- Observed by: 1.2
- Status: open

## RPU-19: Android writes a Missed Call before the app's refresh
- Assumed: Android has written a Missed Call to its call log by the time the app's refresh reads it, so it isn't shown as declined on the Phone.
- Source: `docs/research/2026-11-05-missed-calls.md`.
- Observed by: 2.5
- Status: open

## RPU-20: Calls declined on the Pixel and the missed folder
- Assumed: the Pixel's PBAP server lists, or doesn't list, a Call declined on the Phone in its missed folder, and the app labels it right either way.
- Source: `docs/research/2026-11-05-missed-calls.md`.
- Observed by: 2.4
- Status: open

## Calls and audio

## RPU-21: Answered or declined on the Pixel
- Assumed: a Call answered or declined on the Pixel sends the signals the Simulated Phone models, PipeWire removes it promptly, and with `RejectSCO` the audio stays on the Pixel.
- Source: `docs/research/2026-11-04-answered-on-phone.md`.
- Observed by: 2.2
- Status: open

## RPU-22: Late line identification
- Assumed: the Pixel reports an incoming caller's number within one RING, and a Call that vanishes and returns keeps its number.
- Source: `docs/research/2026-11-05-late-line-identification.md`.
- Observed by: 2.1
- Status: open

## RPU-23: Withheld numbers
- Assumed: a withheld Call reaches the app with an empty line identification, and an unanswered one is a Missed Call.
- Source: `docs/wiki/telephony-api-quirks.md`.
- Observed by: 2.6
- Status: open

## RPU-24: The number the Pixel reports for an outgoing Call
- Assumed: the Pixel reports an outgoing Call's number in a form (plus, 00 or national 0) that matches the Test Number, up to 3 s after the Call appears.
- Source: `docs/research/2026-11-05-checklist-step-scripts.md`.
- Observed by: 7.1
- Status: open

## RPU-25: Early `Activate` after a Call goes active
- Assumed: the Pixel ignores or delays an `Activate` sent within about a second of the Call going active, and a second one while pending is harmless.
- Source: `docs/research/2026-11-05-earlier-handoff-retry.md`; `docs/research/2026-11-05-audio-on-phone.md`.
- Observed by: 3.1
- Status: open

## RPU-26: A Handoff to the Phone that meets a failed `ConnectProfile`
- Assumed: if `ConnectProfile` fails after the Handoff drop, the Call stays live on the Pixel and the Desktop shows the Phone disconnected after 5 s.
- Source: `docs/research/2026-11-05-handoff-to-phone.md`.
- Observed by: 3.1
- Status: open

## RPU-27: Mute by `MicrophoneVolume`
- Assumed: writing `MicrophoneVolume` 0 makes the Pixel stop sending the user's voice, and restoring it unmutes.
- Source: `docs/wiki/telephony-api-quirks.md`; `docs/research/2026-11-04-foner-evaluation.md`.
- Observed by: 3.2
- Status: open

## RPU-28: Keypad tones one at a time
- Assumed: the Pixel rejects an `AT+VTS` of more than one tone, and one `SendTones` per tone reaches a keypad menu.
- Source: `docs/research/2026-11-04-mute-and-keypad.md`.
- Observed by: 3.3
- Status: open

## RPU-29: Call waiting: release-and-answer and swap
- Assumed: `ReleaseAndAnswer` ends the active Call and answers the waiting one, leaving a held Call on hold; `SwapCalls` with one held Call answers the waiting one.
- Source: `docs/research/2026-11-05-call-waiting.md`; `crates/automation/src/checklist/steps/call_waiting.rs` header.
- Observed by: 4.5
- Status: open

## RPU-30: A waiting Call rings again as an ordinary incoming Call
- Assumed: once the Call the user is on ends, a still-ringing waiting Call comes back as an ordinary incoming Call, and a held Call counts as one the user is on.
- Source: `docs/research/2026-11-05-call-waiting.md`.
- Observed by: 4.6
- Status: open

## RPU-31: Newly connected earbuds take the Call audio
- Assumed: Android makes newly connected earbuds its active hands-free device, so a Desktop answer or a Handoff back may land audio on them (inferred, not read in AOSP).
- Source: `crates/simulated-phone/src/telephony.rs` (`Earbuds`); `docs/research/2026-11-05-earbuds.md`.
- Observed by: 5.1
- Status: open

## RPU-32: A refused `Activate` looks like an ignored one
- Assumed: PipeWire answers `Activate` before the Phone acts, so a Phone refusing audio to the Desktop can't be told from one ignoring it.
- Source: `docs/research/2026-11-05-earbuds.md`.
- Observed by: 5.2
- Status: open

## RPU-33: Wideband speech with the Pixel 9a
- Assumed: the Pixel and the Desktop negotiate mSBC, and `Codec` changes from 0 when the audio link opens; no real Call's log has shown a codec yet.
- Source: `docs/research/2026-11-04-first-phone-connection.md`; `docs/research/2026-11-13-real-call-log.md`.
- Observed by: 2.1
- Status: open

## RPU-34: A WirePlumber restart: Calls, owner and order
- Assumed: after a WirePlumber restart the old service's audio link drops, the new one announces its Calls with `InterfacesAdded` without listing them first, owner changes and signals arrive in an order the 2 s grace absorbs, and a Hang up in the gap fails `ServiceUnknown` or `NoReply`.
- Source: `crates/simulated-phone/src/telephony/phone.rs` (`restart`); `docs/research/2026-11-05-vanished-call-settled-by-listing.md`; `docs/research/2026-11-05-hang-up-lost-in-a-restart.md`.
- Observed by: 6.3
- Status: open

## RPU-35: Hands-free dropped by the Pixel mid-Call
- Assumed: turning off the Pixel's "Phone calls" switch mid-Call ends the Call in the app after the 2 s grace, and nothing else breaks (#112).
- Source: `docs/research/2026-11-05-handsfree-not-back.md`.
- Observed by: no step yet
- Status: open

## The Desktop

## RPU-36: The Incoming Call Banner draws above Omarchy's notifications
- Assumed: from Hyprland's drawing order, the Incoming Call Banner shows above Omarchy's notification cards.
- Source: `crates/automation/src/checklist/steps/keyboard_and_look.rs` (7.2); spec #221.
- Observed by: 7.2
- Status: open
- Judged: 7.2 fails a Banner hidden under a notification: spec #221 requires it on top, so that is the app's bug however Hyprland draws.
