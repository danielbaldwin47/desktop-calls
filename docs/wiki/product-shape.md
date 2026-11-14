# Product shape

Terms are as defined in [GLOSSARY.md](../../GLOSSARY.md).

## Grilling round 1, 2026-11-04 (user accepted all recommendations except where noted)

- v1 scope is Calls only: Incoming Call Banner, answer, decline, hang up, mute, Dialer with Contacts search, Call History. SMS, notification mirroring and file transfer are out of scope.
- v1 needs nothing installed on the Phone ([ADR-0001](../adr/0001-bluetooth-hands-free-not-wifi-relay.md)). User amendment: a Companion App is wanted **soon** after v1, for contact photos and starting calls from the Phone's UI that hand off to the Desktop.
- Target the user's Omarchy/Hyprland Desktop first, built on standard pieces (BlueZ, PipeWire, freedesktop notifications) so other desktops are likely to work without being tested.
- Incoming: the Incoming Call Banner appears at the top right and the Desktop rings through its speakers while the Phone also rings. Answering on the Desktop puts the Audio Location on the Desktop's current default audio devices.
- Audio Location follows the device the Call was answered on. Handoff in either direction takes one click.
- Outgoing: a Hyprland keybinding or the app launcher opens the Dialer, which fuzzy-matches Contacts and Call History; Enter places the Call. The Call Window offers timer, mute, keypad, Handoff and hang up.
- Personal use first, built well enough to publish later (AUR package, README).

## Grilling round 2, 2026-11-04 (user accepted all recommendations)

- "Polished" is the scorecard on [polish-scorecard](polish-scorecard.md); the hill climb iterates until it passes in full.
- A missed Call raises a desktop notification with Call back and Dismiss, and is marked in the Dialer's Call History.
- Call waiting is in v1: the Incoming Call Banner offers Hold & answer, End & answer, and Decline.
- When Omarchy's Do Not Disturb is on, the Incoming Call Banner shows but the Desktop does not ring.
- Phone out of range: the indicator quietly shows disconnected, and reconnection happens on its own when the Phone returns.
- First run: a guided "Connect your phone" window handles pairing and the Phone's contacts/call-history permission prompt, and handles re-pairing later.
- Contacts and Call History are cached on the Desktop (in the user's home directory, readable only by the user) and refreshed on each connect.
- Earbuds connected to the Phone: the Incoming Call Banner still shows. Audio Location follows where the Call was answered. Feasibility is not yet verified.
- Autostart at login, with a top-bar indicator for connected, disconnected and in-call states; clicking it opens the Dialer. Whether Omarchy's bar can show it is not yet verified.
- The app is named **Desktop Calls**, command `desktop-calls`.
- The Test Number is the user's own mobile number, which goes to voicemail. Ask the user for it when the first test needs it.

## Grilling round 3, 2026-11-04 (user accepted all recommendations)

- Clicking a missed-Call notification opens the Dialer with that Call selected; Enter calls back. Omarchy draws no notification buttons ([omarchy research](../research/2026-11-04-omarchy-shell-integration.md)).
- Handoff to the Phone: Claude prototypes an in-app fix first. Any upstream PipeWire patch is drafted and shown to the user; nothing is submitted without their OK.
- Agents may edit the user's WirePlumber config (hands-free auto-connect, default-reject-sco). The user installed `bluez-obex` themselves on 2026-11-04. The first-run window does the WirePlumber steps for other users, and the AUR package depends on `bluez-obex`.
- License: GPL-3.0-or-later, so Foner's handset-quirk fixes can be reused.
- The user confirmed a shared understanding: grilling is complete. Next: spec, tickets, test harness, hill climb.
- The user asked how much Foner already solves. An evaluation was dispatched before writing the spec, because build-vs-contribute changes it.
- 2026-11-04: Foner evaluation verdict: build Desktop Calls in Rust/GTK and borrow Foner's knowledge. Of 29 items, Foner has 7, partially has 15 and is missing 7. On Omarchy its Incoming Call Banner would show no buttons and is hidden under Do Not Disturb. It has one author and is inactive since 2026-10-07. Source: [Foner evaluation](../research/2026-11-04-foner-evaluation.md) (at `AminBlg/kaller@fc25b13`).
- 2026-11-04 (Q27, user accepted): Test seam. Tests run the real app against the Simulated Phone (fake `org.pipewire.Telephony` and obexd interfaces on a private bus), drive and observe it through the app's own D-Bus control interface (which also serves the keybinding, CLI and tray), and check visuals with screenshots in a headless compositor. There are no unit tests of internals.
- 2026-11-04: The user accepted all seven UX calls listed under "Further Notes" in spec #1: launcher-like layer-shell Dialer and Call Window; no Desktop ringtone for a waiting Call; Swap included; a Call declined on the Phone is not a Missed Call; mute only when the Audio Location is the Desktop; one contextual keybinding; the Setup Window offers to add the keybinding.
- 2026-11-04: The Handoff-to-Phone mechanism is the hands-free profile bounce ([ADR-0005](../adr/0005-handoff-to-the-phone-bounces-the-hands-free-profile.md), accepted by the chief of staff). No PipeWire patch is needed for v1. A `Deactivate()` upstream would supersede it.
- 2026-11-04, after the Omarchy 4.5 upgrade: notification buttons now render, so the missed-Call notification gets the **Call back** and **Dismiss** buttons the user chose in round 2 (Q12). Clicking the card body still opens the Dialer with that Call selected (Q22). Both answers now hold together. Decided by the chief of staff from the user's earlier answers.

## Built behaviour and implementer decisions, 2026-11-05 (batch 3)

Decisions implementers made within their tickets, as built on `main`. Each holds against the Simulated Phone only unless it says otherwise.

- 2026-11-05 (#17, `417411e`): the round 2 Do Not Disturb rule is built. Under Do Not Disturb the Banner shows and the ringtone never starts; turning Do Not Disturb on or off mid-ring stops or starts it. The Indicator still shows its ringing look under Do Not Disturb. That the user wants this is inferred, not asked: spec story 27 is silent on Do Not Disturb. Missed Call notifications are unchanged, so Omarchy 4.5 silences them and files them to its history (inferred for the history). Source: [Do Not Disturb silent](../research/2026-11-05-do-not-disturb-silent.md).
- 2026-11-05 (#19, `6bf11cb`): out of range changes only the Indicator, with no notification, as round 2 asked. A ringing Call is hidden while the Phone is absent. A live Call that loses the link stays in the Call Window with its timer and the message "Connection to phone lost: the Call continues on your phone."; hang-up, mute, tones and Handoff are refused with that message. It closes 10 s after the loss if the Phone stays away, and is never reported as "Call ended" or as a Missed Call. A Phone that has forgotten the pairing becomes the "pairing lost" Problem, and paging stops; the "Pair again" action belongs to the Setup Window (#21). Source: [out of range](../research/2026-11-05-out-of-range.md).
- 2026-11-05 (#101, `f7a35a6`): hands-free that doesn't come back after a Handoff to the Phone is treated like #19's lost link: the same message and timer, never "Call ended", and the Call returns live with its first timer when hands-free returns. Known gaps: the Indicator then says "out of range" though the Phone is in range; and hands-free dropped by the Phone itself mid-Call (for example the Pixel's per-device "Phone calls" toggle) still ends the Call after 2 s, which needs its own decision (#112, needs-info). The lost Call Window's controls still look usable (#111). Source: [hands-free not back](../research/2026-11-05-handsfree-not-back.md).
- 2026-11-05 (#103, `f6ecaae`): a Call that rings with no number shows "Identifying caller…" on the Banner for up to 2 s, then "Withheld number" if no number came. The cost is 2 s of "Identifying caller…" before a truly withheld Call reads "Withheld number". The Indicator's tooltip still reads "Call from Withheld number" in that window (#115). Source: [identifying caller](../research/2026-11-05-identifying-caller.md).

## Built behaviour, 2026-11-05 night into 2026-11-06

Implementer decisions as built on `main`. Each holds against the Simulated Phone only.

- 2026-11-05 (#161, PR #164, `5114208`): the Setup Window's key hint says "Enter presses the highlighted button; Tab moves between buttons; Escape closes" while a Problem button takes focus (Bluetooth off or blocked, while choosing a Phone), and keeps "Enter: Confirm" or "Enter pairs it" otherwise. The core decides (`problem_takes_focus()`). Reviews noted, after-v1, that the hint follows `takes_focus` and not actual focus (#154). Superseded by #228: the footer line shows keycaps (`Enter Turn on · Tab Next button · Esc Close`), the core names it `blocks_step()`, and only a Problem that stops pairing dims the steps.
- 2026-11-05 (#162, PR #166, `9ebed55`): Set up audio counts as done only when systemd's restart job for WirePlumber ends `done`, within 10 s; any other result, or no end in time, keeps the Problem and says the audio didn't restart. A real restart that takes over 10 s now reads as failed even if it succeeds later, and Set up audio can be tried again (PR #166's own risk note). The drop-in keeps its permission bits when rewritten. Real systemd's `Subscribe` and `JobRemoved` are unverified here.
- 2026-11-05 (#163, PR #165, `6b4d43b`; supersedes #19's entry above where it says tones are refused with the lost-contact message): while contact with a live Call is lost, the Call Window hides the tone entry, the keypad and the keypad toggle, and shows them again when contact returns. The toggle keeps its place, invisible, so Mute and Hang up don't resize. Typed tones and an open keypad are kept for the same Call and reset for a new one.
- 2026-11-05 (#175, PR #176, `b096898`; extends ADR-0007): after a Hang up, a Call that goes away without `disconnected` is a vanished Call, not an ended one. If hands-free doesn't return in time, the Call Window shows the lost state with "The Call may still be live on your phone; check your phone and hang up there if it is.", never "Call ended". A Call that returns is not hung up by itself. A repeated Hang up goes only to a Call that can be the same one (same state progression, same known number). Not run on the Pixel; that a restarted PipeWire doesn't list Calls before announcing them is inferred.

## Built behaviour, audit fixes merged 2026-11-11 03:52 UTC (PR #193, `a589e5d`)

Fixes from the 2026-11-06 GPT 6.1 Sol audit. None was run on the Pixel.

- 2026-11-11 (#190, `bfcb805`): when a later Call History page fails, the folder is not marked complete, so cached older Calls survive a restart. Contacts take the same path: a failed later page keeps the cached Contacts and drops the partial new ones, and with no cache the first sync fails. #193 claims that only happens on a Phone that ignores `Offset`, and that the Pixel honours it; not checked against `bfcb805`, which also marks a failure on a page error.
- 2026-11-11 (#188, #189, `844022e`): dial timers and connect attempts keep unique identities across a Phone replacement, so a stale timeout or connect result no longer fails the new Phone's Dial or disables its recovery. Hands-free connect results still carry no identity (#193's follow-ups, unfiled).
- 2026-11-11 (#191, `805b63d`, `806cedc`): the systemd unit, D-Bus service and Desktop Entry quote launch paths containing spaces, quotes, `$` and `%`.
- 2026-11-11 (#192, `ea50218`): a queued Decline stays bound to the Call it was chosen for, so a new caller that reuses the ended Call's path is not declined. A queued Answer for a Call that vanishes is dropped silently, because the core sends the Answer again if the Call returns ringing. A commit that reported it as failed (`83e4286`) was reverted for showing a false "answer failed".

## 2026-11-13: the Call Pill and Calls Off (user decisions; specs #197 and #198)

Product decisions made by the user in conversation with the chief of staff on 2026-11-13, and what the specs record. Nothing here is built: as of about 19:05 EST that day no ticket of either spec had a merged PR (checked with `gh issue list` and `gh pr list`). The user lifted the feature freeze of 2026-11-05 for these two features and the UI pass, and for nothing else ([session-log](session-log.md), [destination](destination.md)).

### The Call Pill (spec #197, tickets #200 and #201)

- 2026-11-13, built since (checked 19:25 EST): the Call Pill's first ticket, #200, merged as PR #218 (`2b5ca80`, 00:23Z). Its behaviour is not recorded here: this page has not read the PR, and nothing of it ran on the Pixel. Ticket #201 was still open.
- 2026-11-13, user, after the first 16-minute real Call ([real Call note](../research/2026-11-13-real-call-log.md)): "I'd like for the call popup to be dismissable or to minimize itself so it's not blocking the screen". The user chose that the Call Window shrinks itself to a corner pill a few seconds after the Call connects.
- 2026-11-13, decided by the chief of staff and not by the user (#197, "Decided without the user", 11 items): the delay is 5 s; the corner is bottom right, 12 px from the edges, so the top right stays for the Incoming Call Banner, Omarchy's notifications and the expanded Call Window; the Call Pill shows caller, timer, Mute and Hang up and never takes keyboard focus; the Call Window stays expanded while it has the keyboard, the pointer is over it, the keypad is open, a notice shows, contact is lost, a Call is on hold, or the Call isn't connected yet; it expands by itself on lost contact, a notice, a Call going on hold or a new Call connecting, but not for a ringing Waiting Call or a Problem; Escape collapses it; the Indicator's left click expands it during a Call; no ADR; a new glossary term, **Call Pill**. The chief of staff listed these to the user, who objected to none (the user's reply is not in the issue; this is the chief of staff's account).
- Dependencies recorded in the tickets: the keybinding, Escape and shortcuts keep scorecard item 7 (every action by keyboard); Real-Phone Checklist steps 3.1, 3.3 and 7.1 change and a step 3.4 is added in #200's PR.

### Calls Off and Quiet Hours (spec #198, from the outside request #194)

- 2026-11-13, user: chose Calls Off with timed choices (until turned on, 1 hour, until tomorrow morning) plus daily Quiet Hours, as an in-app answer to #194. Tickets #202 to #207, #209 and #215.
- 2026-11-13, user: "Quiet hours should also have something in the gui for settings" (ticket #209). The spec first had presets in an Indicator submenu and no settings window. The chief of staff replaced the presets with one **Quiet Hours…** entry that opens a small Quiet Hours Window; the user asked for a GUI, and the entry replacing the presets is the chief of staff's call within that request (#198, "Changed by the user"). The CLI command stays.
- 2026-11-13, user, confirming the mechanism, `Device1.Blocked`, knowing it also turns off the Phone's media audio (A2DP) while calls are off: "I'm personally ok with disconnecting entirely, I don't want the battery drain of constant phone bluetooth and also I've experienced my alarm in the morning bugging out because it's connected." The user agreed that blocking is the only v1 mode, and that a mode which keeps the media audio is an after-v1 concept ([destination](destination.md)). The mechanism is to be recorded in ADR-0008 (status "proposed", written by ticket #202; it does not exist on `main` as of 2026-11-13).
- 2026-11-13, the user's two changes after a concept review by the chief of staff, before the tickets were built (#198, "Changed after concept review"): a Call placed from the Dialer while calls are off is an exception for that one Call, and calls go off again when it ends for whatever remains of the state they were in (this replaced "Enter turns calls on and leaves them on"; ticket #206); and uninstalling unblocks the Phone itself (this replaced a README note; ticket #215). The user agreed both.
- 2026-11-13, decided by the chief of staff and not by the user (#198, "Decided without the user", 12 items listed): a live Call is never cut, so a turn-off waits for the last Call to end, with **Keep calls on** to cancel (the user's lean, per the issue); timed choices count from when they are chosen; "until tomorrow morning" means the next 07:00 or the end of Quiet Hours; a hand-chosen "until I turn them on" outlasts Quiet Hours; Setup Window while off offers **Turn calls on**; no Missed Call notifications for Calls rung while off; the off Indicator never asks for attention; a Phone the user blocked outside Desktop Calls is left alone; two new glossary terms, **Calls Off** and **Quiet Hours**. The user was shown the list and objected to none except where Quiet Hours are set (above).
- 2026-11-13, a correction from the probe, to the spec's first draft: BlueZ does not refuse a `Device1.Connect` from the Desktop on a blocked device ([probe note](../research/2026-11-13-calls-off-mechanism-probe.md); see [telephony-api-quirks](telephony-api-quirks.md)). The spec and tickets #202 and #206 were updated: nothing in the app may call `Connect` or `ConnectProfile` while calls are off.

### What the first long real Call showed about the product (2026-11-13)

- 2026-11-13, user: the Call was incoming; no Incoming Call Banner appeared and they answered on the Phone, then moved the audio from the Call Window; the audio was perfect; they did not mute.
- 2026-11-13, observed in the journal, and inferred from it ([real Call note](../research/2026-11-13-real-call-log.md)): the Desktop's Bluetooth was off, and the Phone connected 3.56 s after it came back on, with the Call already active, so the app never saw the Call ring. That explains the missing Banner without a defect in the Banner (inferred). The chief of staff's triage found an `rfkill unblock bluetooth` run from the user's own account 3.6 s before the Call appeared; I could not check that against the cited sources, which record only the kernel's rfkill lines. The user could not remember whether they toggled it.
- 2026-11-13: an incoming Call answered on the Desktop had still not been observed on the Pixel. Real-Phone Checklist group 2 ([testing](testing.md)) is where that gets observed, by the user.
