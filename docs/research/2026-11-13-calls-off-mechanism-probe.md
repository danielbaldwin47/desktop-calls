# Calls Off mechanism probe on the real Pixel

Date: 2026-11-13. Spec #198 ("Mechanism" and "Unverified on the real stack"), ticket #202. User-approved probe, run by one agent with no Call on the Phone.
Legend: **[V]** = observed on the Desktop with the real Pixel 9a. **[S]** = read in primary source (BlueZ 5.87 `src/device.c` and `doc/org.bluez.AdminPolicySet.rst`, PipeWire 1.6.9 `spa/plugins/bluez5/bluez5-dbus.c`), not tested. **[I]** = inferred.

**Bottom line:** `Device1.Blocked = true` drops the live link and every profile within 0.2 s, and nothing reconnected the Phone during 83 s blocked. After `Blocked = false`, nothing reconnected it by itself either (30 s watched), so the app must call `Device1.Connect`. That call connected the Phone in 0.95 s, and hands-free was up 0.27 s after the link. The mechanism holds, with one correction to #198: BlueZ does **not** refuse a `Device1.Connect` from the Desktop on a blocked device. It pages the Phone, brings the baseband link up, browses SDP and hangs for 25 s or more before failing. So nothing on the Desktop may call `Connect` while calls are off, and the Simulated Phone's "Connect on a blocked device fails" must model the slow failure, not a quick refusal. Separately, the installed app made **no** logged reconnect attempt in either disturbance (see "The running app").

## Setup

- Stack as in [hardware-and-stack](../wiki/hardware-and-stack.md): BlueZ 5.87, PipeWire 1.6.9, WirePlumber 0.5.18. The installed Desktop Calls service (built 2026-11-13 17:36) was left running throughout.
- Before starting: `desktop-calls status` said connected, and PipeWire Telephony listed one AudioGateway and no `Call1`. **[V]** Each probe script checked for a `Call1` again and would have stopped.
- Recorded with timestamps: `gdbus monitor --system --dest org.bluez`, `gdbus monitor --session --dest org.pipewire.Telephony`, `pactl subscribe`, and the user and `bluetooth` journals. `btmon` needs root and was not run, so radio-level paging is not visible.
- Times below are seconds from the action. The Phone was disturbed for about 2 min 5 s (Probe A) and 32 s (Probe B).

## Probe A: `Device1.Blocked`

### A1. Does `Blocked = true` drop the live connection, and all profiles?

Yes, at once. **[V]**

| t (s) | Event |
|---|---|
| 0.000 | `Properties.Set Blocked true` sent |
| 0.004 | AVRCP player objects removed |
| 0.021 | `Blocked` true; `Network1` and `MediaControl1` interfaces removed; the Set returned |
| 0.212 | `ServicesResolved` false; `Device1.Disconnected` with `org.bluez.Reason.Local` "Connection terminated by local host"; `Connected` false; A2DP `MediaTransport1` removed; PipeWire Telephony's AudioGateway removed (`ModemRemoved`); the app logged "The Phone is disconnected" |
| 0.221 | A2DP endpoints and `Battery1` removed; the PipeWire Bluetooth card removed |

BlueZ's `device_block` disconnects everything, removes every service from the device, then adds the address to the kernel's block list and stores `Blocked` on disk. **[S]**

### A2. What does `Device1.Connect` return while blocked?

It does not fail quickly. **[V]** Called 1.6 s after blocking:

- The baseband link came up 0.73 s later (`Connected` true while `Blocked` stayed true).
- No reply arrived within `busctl`'s 25 s timeout ("Call failed: Connection timed out", which is the client's timeout, not a BlueZ error).
- `ConnectProfile(HFP AG)` then returned `org.bluez.Error` "No more profiles to connect to" after 0.35 s, and `ServicesResolved` went true at the same moment.
- BlueZ dropped the idle link 4.6 s later ("Connection terminated by local host").

Why, from source **[S]**: blocking removed every service, so `Connect` finds no profile to connect and starts an SDP browse instead. The kernel's block list rejects incoming connections only, so the outgoing page and link are allowed. After the browse, profile probing is skipped for a blocked device ("Skipping profiles for blocked device"), so `Connect` is answered `br-connection-profile-unavailable` ("No more profiles to connect to"). **[I]** that the 25 s and more was the SDP browse itself; with no `btmon` this is not confirmed.

Consequence: every `Connect` while blocked pages the Phone and holds a link for seconds, which is the battery drain the user wants to avoid. The reconnect loop must not run while calls are off (as #198 already says), and the app's own BlueZ calls need a timeout so one hung `Connect` can't stall the loop. **[I]**

The running app's reconnect loop logged nothing while blocked: no "BlueZ couldn't connect the Phone" line. **[V]** See "The running app".

### A3. Over about 90 s blocked, does the Pixel page in? Does WirePlumber try anything?

- Blocked for 83 s (t = 0 to 83). Apart from the link that our own `Connect` caused, `Connected` never went true, and the `bluetooth` journal logged only "No matching connection for device" at t = 0.2. **[V]**
- Whether the Pixel paged at all, and whether each page was refused, is **not observed**: that needs `btmon` as root. **[I]** that it did not page, because it also did not page in during the 30 s after unblocking (A4), when nothing would have refused it.
- WirePlumber logged nothing, and no profile reappeared. **[V]** PipeWire's auto-connect skips a blocked device, and only runs while a device's connection is first being set up. **[S]** (`device_timer_event` checks `!device->blocked`.)

### A4. After `Blocked = false`: who reconnects, how fast, and any side effects?

- Nothing reconnected the Phone by itself in 30 s: not the Phone, not WirePlumber, not the app. **[V]** `Network1` and `MediaControl1` came back at once from the stored service list, with no link. **[V]**
- `Device1.Connect` from the Desktop, 41 s after unblocking: `Connected` after 0.56 s, the call returned after 0.95 s, A2DP and AVRCP came up, Telephony's AudioGateway appeared at 0.83 s and the app logged "The Phone is connected" at the same moment. **[V]** So `Connect` brings A2DP back too, which answers an open question from the [out of range](2026-11-05-out-of-range.md) note.
- No `br-connection-busy`: only one `Connect` was made and it succeeded first time. **[V]** Busy remains possible when the Phone pages in at the same moment **[I]**, so the loop's retry stays.
- `Trusted`, `Paired` and `Bonded` stayed true throughout; the 22 service UUIDs were the same after. `Blocked` itself persists in BlueZ's storage. **[V]** for the properties, **[S]** for the storage.

## Probe B: `DisconnectProfile` for hands-free only

- `DisconnectProfile(0000111f…)` (HFP AG) returned at once. Telephony's AudioGateway and `Battery1` went away; the device stayed connected, the A2DP transport stayed, and the PipeWire card stayed on its `audio-gateway` profile (A2DP source plus HFP). **[V]** So media audio stays up.
- `DisconnectProfile(00001112…)` (HSP AG) failed "Invalid arguments in method call": the Pixel offers no HSP AG. **[V]** for the error, **[I]** for the reason.
- Hands-free did **not** come back in 32 s, from either side: not the Phone, not WirePlumber, not the app. **[V]** Meanwhile the app logged "The Phone is disconnected" and `desktop-calls status` said the Phone "is out of range or has Bluetooth off", although it was connected with media audio. **[V]**
- Restored with `ConnectProfile(HFP AG)` from the Desktop: the call returned after 0.67 s and Telephony's AudioGateway appeared after 0.77 s. **[V]**

Can hands-free stay down while A2DP stays up, without restarting WirePlumber?

- Within one connection, yes: observed above. PipeWire's `bluez5.auto-connect` reconnects missing profiles only while a connection is first being set up (a 2 s, then 6 s timer that stops once the device counts as connected), so it never restores a profile dropped later. **[S]**
- Across a reconnect (out of range and back, a reboot, the Phone's Bluetooth toggled), no. WirePlumber's auto-connect includes `hfp_hf` here and would connect hands-free again, and the app would have to drop it again each time, briefly giving the Phone a hands-free device to route Call audio to. **[S]** for WirePlumber, **[I]** for the Phone's side.
- BlueZ has no per-device profile block. The only per-service control is `AdminPolicySet1.SetServiceAllowList`, which is adapter-wide, experimental, and not exposed on this adapter (no `AdminPolicySet1` on the adapter object, so bluetoothd runs without experimental interfaces). **[S]** and **[V]** for its absence.
- Removing `hfp_hf` from the WirePlumber roles works but needs a WirePlumber restart and fights the setup checks, as #198 already records.

So a "hands-free off, media on" mode is possible only as a best effort that redoes `DisconnectProfile` after every reconnect. **[I]** Not part of #198.

## The running app

The installed app (one process, running since 17:36) made no logged reconnect attempt during either disturbance **[V]**:

- Probe A: from the disconnect to the manual reconnect, 2 min, no `phone-connect-failed` line. An earlier process of the same app logged a failed `Connect` about every 3 s for hours earlier that day, so the line is not filtered out.
- Probe B: hands-free was missing for 32 s with the Phone connected; the hands-free-not-back retry (`ConnectProfile` after 1 s, 2 s, then every 3 s, #101) would have restored it within about 1 s, and did not.

Cause not investigated: no product code was read beyond the logging and reconnect paths. **[I]** One explanation that fits both: a `Connect` or `ConnectProfile` from the app left unanswered, which zbus waits on with no timeout, so the core still thinks a connect is under way and only remembers to try again "at once" after it. Worth its own ticket.

## For #198 and its tickets

- `Blocked = true` drops the Phone at once, and all profiles. **[V]**
- `Device1.Connect` on a blocked device pages the Phone and hangs for 25 s or more, then fails `br-connection-profile-unavailable`. **[V]** for the hang and the link, **[S]** for the error. The Simulated Phone should model that, and the app must never call it while off.
- The Pixel's own pages: no reconnect seen while blocked, nor after unblocking. **[V]** Refusal at the radio not observed.
- After unblocking, only the Desktop's `Connect` reconnects; it took 0.95 s, with no `br-connection-busy`. **[V]**
- Blocking changes no other property; `Network1` and `MediaControl1` vanish while blocked and return on unblock. **[V]**
