---
status: proposed
---

# Calls Off blocks the Phone with BlueZ's `Device1.Blocked`

## Context

Spec #198 (from the outside request #194) lets the user turn calls off: the Desktop keeps the Phone disconnected, and nothing connects it again, until calls are turned on. The user wants the Phone disconnected entirely, in their words: "I'm personally ok with disconnecting entirely, I don't want the battery drain of constant phone bluetooth and also I've experienced my alarm in the morning bugging out because it's connected." That is two reasons:

1. the battery drain of a constant Bluetooth connection to the Phone;
2. the Phone's morning alarm misbehaving while it is connected to the Desktop.

The user confirmed this on 2026-11-13, knowing that blocking the Phone also stops its media audio to the Desktop.

On the real Pixel 9a ([probe note](../research/2026-11-13-calls-off-mechanism-probe.md)), `Blocked = true` dropped the link and every profile within 0.2 s, and nothing reconnected the Phone in 83 s. After `Blocked = false` nothing reconnected it by itself either, and one `Device1.Connect` connected it in 0.95 s. BlueZ did **not** refuse a `Connect` from the Desktop on the blocked device: the link came up and no reply came within 25 s. BlueZ's source says it fails `br-connection-profile-unavailable` after an SDP browse.

## Decision

Turning calls off sets `org.bluez.Device1.Blocked = true` on the chosen Phone. Turning calls on sets it back to `false`, then connects the Phone with the existing reconnect loop: `Device1.Connect` at once, then the 1 s, 2 s, then every 3 s waits.

The app keeps its own record, `$XDG_STATE_HOME/desktop-calls/calls-off.json`, written before it blocks or unblocks the Phone and removed once the Phone is unblocked. At startup:

- an "off" record keeps the Phone blocked, blocking it again if needed;
- an "on" record that names an address, left by a run stopped halfway, unblocks it;
- a Phone blocked with no record is the user's own setting, and the app never touches it.

Amended by ticket #205 (Quiet Hours): one record outlives the unblock. Calls turned on, or kept on, during Quiet Hours stay on until the Quiet Hours next begin, and that choice must survive a restart. So the app keeps a `"kept-on"` record, with that instant as `until` and no address, while the Phone is unblocked; it blocks nothing, and is removed once the Quiet Hours begin or end, or are turned off. An "on" record left halfway carries the same `until` when it was turned on during Quiet Hours, and an "off" or pending record begun by Quiet Hours says so with `"quiet_hours": true`.

Amended by ticket #206 (a Call placed while calls are off): the app also unblocks the Phone, without turning calls on, for a Call the user places from the Dialer while calls are off. Before it unblocks, it writes a pending record with `"exception": true` for what remains of the earlier state, so calls go off again, through #204's pending turn-off, once that Call ends, also after a restart. A build without the field reads it as an ordinary pending record.

This **amends [ADR-0004](0004-bluez-device-api-for-presence-and-pairing.md)**, which allows BlueZ's `Device1` for presence and pairing only, and ADR-0005's hands-free bounce. Desktop Calls may now also write `Device1.Blocked`, for this one purpose. Everything else in ADR-0004 stands: Desktop Calls still never registers a Bluetooth profile and never speaks AT commands, and nobody edits the WirePlumber drop-in.

## Considered options

- **`Device1.Blocked`** (chosen). BlueZ drops the Phone and all its profiles at once, refuses the Phone's own connections, and PipeWire's `bluez5.auto-connect` skips a blocked device. The pairing is kept.
- **`Device1.Disconnect`, and stopping the app's reconnect loop.** The Phone and WirePlumber connect it again; #194's requester saw this.
- **Removing `hfp_hf` from `bluez5.auto-connect` while off.** It fights the setup checks, needs a WirePlumber restart, and doesn't stop the Phone paging in.
- **`Trusted = false`.** It depends on the pairing agent refusing authorization prompts, and doesn't drop the link.
- **Turning the Desktop's Bluetooth off (rfkill).** It disconnects the user's other Bluetooth devices.

## Consequences

- While calls are off, the Phone's media audio (A2DP) to the Desktop is off too. The user accepted this, for the two reasons above.
- BlueZ refuses the Phone's connections, but not a `Connect` from the Desktop: that pages the Phone and fails slowly. So while calls are off, nothing in the app calls `Device1.Connect` or `ConnectProfile` on the Phone. The Call state machine decides this, and the Simulated Phone counts any such call made while blocked.
- `Blocked` lives in BlueZ's storage and outlives the app. That is why the app keeps its own record of having set it, and why uninstalling must unblock a Phone the app blocked (ticket #215).
- Turning calls on needs a `Connect` from the Desktop: nothing reconnects a Phone by itself after it is unblocked.
