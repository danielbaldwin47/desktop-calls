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
