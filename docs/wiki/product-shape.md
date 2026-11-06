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

- 2026-11-05 (#161, PR #164, `5114208`): the Setup Window's key hint says "Enter presses the highlighted button; Tab moves between buttons; Escape closes" while a Problem button takes focus (Bluetooth off or blocked, while choosing a Phone), and keeps "Enter: Confirm" or "Enter pairs it" otherwise. The core decides (`problem_takes_focus()`). Reviews noted, after-v1, that the hint follows `takes_focus` and not actual focus (#154).
- 2026-11-05 (#162, PR #166, `9ebed55`): Set up audio counts as done only when systemd's restart job for WirePlumber ends `done`, within 10 s; any other result, or no end in time, keeps the Problem and says the audio didn't restart. A real restart that takes over 10 s now reads as failed even if it succeeds later, and Set up audio can be tried again (PR #166's own risk note). The drop-in keeps its permission bits when rewritten. Real systemd's `Subscribe` and `JobRemoved` are unverified here.
- 2026-11-05 (#163, PR #165, `6b4d43b`; supersedes #19's entry above where it says tones are refused with the lost-contact message): while contact with a live Call is lost, the Call Window hides the tone entry, the keypad and the keypad toggle, and shows them again when contact returns. The toggle keeps its place, invisible, so Mute and Hang up don't resize. Typed tones and an open keypad are kept for the same Call and reset for a new one.
- 2026-11-05 (#175, PR #176, `b096898`; extends ADR-0007): after a Hang up, a Call that goes away without `disconnected` is a vanished Call, not an ended one. If hands-free doesn't return in time, the Call Window shows the lost state with "The Call may still be live on your phone; check your phone and hang up there if it is.", never "Call ended". A Call that returns is not hung up by itself. A repeated Hang up goes only to a Call that can be the same one (same state progression, same known number). Not run on the Pixel; that a restarted PipeWire doesn't list Calls before announcing them is inferred.
