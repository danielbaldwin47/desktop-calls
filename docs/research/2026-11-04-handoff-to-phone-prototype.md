# Handoff to the Phone: prototype on the real Pixel 9a

Date: 2026-11-04, 18:00-18:25 EST. Ticket #3. Phone: Pixel 9a `AA:BB:CC:DD:EE:FF`. Desktop: BlueZ 5.87, PipeWire 1.6.9, WirePlumber 0.5.18, Intel AX201 (as in [HFP stack research](2026-11-04-hfp-stack.md) §0).
Legend: **[V]** = verified on this machine. **[I]** = inferred, not tested.

**Bottom line:** Candidate A, bouncing the hands-free profile, works. `Device1.DisconnectProfile(0000111f-…)` followed at once by `Device1.ConnectProfile(0000111f-…)` moved the Audio Location from the Desktop to the Phone in 5 of 5 trials with no dropped Call. Desktop control was lost for 0.54 to 1.01 s. With `RejectSCO=true` on the re-created AudioGateway, the Phone's attempts to reopen audio to the Desktop were rejected, so the audio stayed off the Desktop. Media (A2DP) was untouched. Candidates B and C were not needed. The decision is in [ADR-0005](../adr/0005-handoff-to-the-phone-bounces-the-hands-free-profile.md).

Two findings change how the app must do Handoff **to the Desktop** as well:

1. **[V] `Activate()` alone never brings audio to the Desktop while `RejectSCO=true`.** PipeWire sends `AT+BCC`, the Phone opens SCO, and PipeWire rejects it. The app must set `RejectSCO=false` first (§2).
2. **[V] `Activate()` is not always honoured.** In 1 of 7 tries the Phone answered `OK` and then did nothing for 10 s. In another it took 2.9 s. The app needs a deadline and a retry (§2.3).

The prototype is a throwaway Python probe on branch [`prototype/3-handoff-to-phone`](https://github.com/danielbaldwin47/desktop-calls/tree/prototype/3-handoff-to-phone) (`prototypes/PROTOTYPE-handoff-to-phone/handoff_probe.py`). None of it is in the product workspace.

---

## 1. Method

- The probe places a Call to the Test Number with `AudioGateway1.Dial`. The Test Number is the user's own mobile, so the Call goes to voicemail and becomes `active` 1.6 to 2.8 s after dialling. It never prints or writes the number; every output passes through a redactor, and the logs were grepped afterwards (0 hits).
- It refuses to start if any Call already exists, and only hangs up the Call whose `LineIdentification` matches the Test Number.
- It subscribes to every signal from `org.pipewire.Telephony` (session bus) and `org.bluez` (system bus, filtered to the Phone's paths), plus `pactl subscribe`, and timestamps each one with a monotonic clock. Timings below are relative to the action named.
- WirePlumber trace logging was switched on only in one diagnostic run, after the Call was `active` and before hang-up, so the `ATD<number>` command never reached the journal. The journal was grepped afterwards (0 hits).
- 10 Calls were placed in total, each under 20 s of probe time. The probe hung up 9 of them. One ended on its own (§5.1).

## 2. Handoff to the Desktop: `Activate()`

### 2.1 With `RejectSCO=true`, `Activate()` does nothing audible [V]

Two runs called `Activate()` without touching `RejectSCO`. The transport stayed `idle` for 10 s each time. The trace (redacted, Call already active) shows why:

```
RFCOMM >> AT+BCC                 << OK
RFCOMM << +BCS: 3                >> AT+BCS=3      (Phone picks LC3-SWB)
sco_listen_event: rejecting SCO, AudioGatewayTransport1.RejectSCO=true   (x2)
RFCOMM << +BCS: 1                >> AT+BCS=1      (Phone falls back to CVSD)
sco_listen_event: rejecting SCO, AudioGatewayTransport1.RejectSCO=true
... the 3 -> 1 pair repeats once more, then the Phone gives up.
```

Source: PipeWire 1.6.9 `backend-native.c:1888` (`hfp_hf_transport_activate` only queues `AT+BCC` when codec negotiation is on) and `:3015` (`sco_listen_event` rejects whenever `rejectSCO` is set; `Activate()` does not override it). The same `Codec` 3 → 1 → 3 → 1 pattern appears on the D-Bus `AudioGatewayTransport1.Codec` property during **every** Call setup: that is the Phone trying to bring the audio to the Desktop and being refused, which is exactly what `default-reject-sco` is for.

Without a Call, the Phone answers `AT+BCC` with `OK` and never sends `+BCS` [V].

### 2.2 The working sequence [V]

```
Properties.Set(AudioGatewayTransport1, RejectSCO, <false>)   signature ssv
AudioGatewayTransport1.Activate()                             returns in 0.02-0.04 s
  +0.10 s  PropertiesChanged(AudioGatewayTransport1, {Codec: 3, RejectSCO: false})
  +0.18 s  PropertiesChanged(AudioGatewayTransport1, {State: "pending"})
  +0.21 s  PropertiesChanged(AudioGatewayTransport1, {State: "active"})
           PipeWire nodes bluez_input.<addr>.0 (Stream/Output/Audio) and
           bluez_output.<addr>.1 (Stream/Input/Audio) appear, state running
Properties.Set(AudioGatewayTransport1, RejectSCO, <true>)    re-arm; does not close the open SCO
```

The negotiated codec was LC3-SWB (`Codec = 3`, 32 kHz) every time. The Phone also sets `SpeakerVolume` 15 → 12 shortly after audio opens.

### 2.3 Timing and reliability [V]

`Activate()` → `State = "active"`, with `RejectSCO` lifted:

| Run | Result |
|---|---|
| 1 | 0.209 s (hung up before Candidate A because of a probe bug) |
| 2 | 0.247 s (trial A1) |
| 3 | 0.183 s (A2) |
| 4 | **no audio within 10 s** (single attempt; no `+BCS` at all) |
| 5 | **2.927 s** (A3; `Codec: 3` at +0.13 s, then `pending` only at +2.85 s) |
| 6 | 0.212 s (A4) |
| 7 | 0.188 s (A5), then 0.233 s on a second `Activate()` after the bounce (§3.4) |

So 5 of 7 within 0.25 s, 1 slow, 1 ignored. [I] The ignored one looks like the Phone dropping `AT+BCC` while it is still finishing its own refused SCO attempts from Call setup; run 4 called `Activate()` 1.0 s after `active`, like the others, so this is not proven. The app should wait about 3 s for `State = "active"`, then call `Activate()` once more, and only then report a failure.

## 3. Candidate A: hands-free profile bounce

### 3.1 Results [V]

All five trials: Call placed, audio on the Desktop, wait 2 s, then `DisconnectProfile(HFP AG)` and, as soon as the AudioGateway was gone, `ConnectProfile(HFP AG)`.

| Trial | Call survived | Audio left the Desktop | `ConnectProfile` took | Desktop control lost (AG + Call back) | Phone SCO retries after reconnect | Transport 6 s later |
|---|---|---|---|---|---|---|
| A1 | yes, `active` | ≤ 0.002 s | 0.137 s | 0.847 s | 3→1→3→1, all rejected | `idle`, `RejectSCO=true` |
| A2 | yes, `active` | ≤ 0.002 s | 0.182 s | 1.010 s | 3→1, rejected | `idle`, `RejectSCO=true` |
| A3 | yes, `active` | ≤ 0.002 s | 0.488 s | 1.004 s | 3→1→3→1, rejected | `idle`, `RejectSCO=true` |
| A4 | yes, `active` | ≤ 0.002 s | 0.106 s | 0.542 s | 3→1→3→1, rejected | `idle`, `RejectSCO=true` |
| A5 | yes, `active` | ≤ 0.002 s | 0.153 s | 0.964 s | 3→1→3→1, rejected | `idle`, `RejectSCO=true` |

- **Audio left the Desktop** is the time from `DisconnectProfile` returning to the AudioGateway's removal. The PipeWire hands-free nodes were gone by the next check, 0.02 s later. That the audio then plays from the Phone's earpiece is [I] (Android Telecom's documented behaviour on a hands-free-initiated audio disconnect, [HFP research](2026-11-04-hfp-stack.md) §5.2); confirming it by ear belongs to Real-Phone Checklist group 3.
- **AudioGateway and Call re-adoption:** both came back every time, at the **same paths** (`/org/pipewire/Telephony/ag1`, `.../ag1/call1`), with the Call already `active` and `LineIdentification` intact. Path reuse is [I] for the general case; the app must not rely on it.
- **`RejectSCO` afterwards:** the new AudioGateway starts with `RejectSCO = true` from `bluez5.telephony.default-reject-sco`, whatever the old one had. The Phone tries to reopen audio to the Desktop 0.4 to 0.7 s after reconnecting (one or two `+BCS: 3` then `+BCS: 1` rounds) and gives up once refused. Without the reject default, the bounce would bounce the audio straight back.
- **Media audio:** the A2DP transport (`/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF/fd0`, `0000110b`) emitted no signal during any bounce. `Device1.Connected` stayed `true`. The card profile stayed `audio-gateway`. AVRCP (`MediaControl1`) was untouched. Nothing was playing during the trials, so whether music playback glitches is not tested.
- `Battery1` on the Phone's BlueZ device is removed with the hands-free link and re-added about 0.5-0.9 s later, because PipeWire derives it from the hands-free battery indicator.

### 3.2 The observable D-Bus sequence [V]

Identical ordering in all five trials (times from the `DisconnectProfile` call; ranges cover A1-A5):

```
+0.000        org.bluez Device1.DisconnectProfile("0000111f-0000-1000-8000-00805f9b34fb")   returns ≤ 0.002 s
+0.001-0.002  session  /org/pipewire/Telephony/ag1  ObjectManager.InterfacesRemoved(.../ag1/call1, [Call1])
+0.002        session  /org/pipewire/Telephony/ag1  org.ofono.VoiceCallManager.CallRemoved(.../ag1/call1)
+0.002        session  /org/pipewire/Telephony      ObjectManager.InterfacesRemoved(.../ag1, [AudioGateway1, AudioGatewayTransport1])
+0.002        system   /                            ObjectManager.InterfacesRemoved(<dev>, [org.bluez.Battery1])
              (Battery1 and the AG removal arrive in either order)
              PipeWire hands-free nodes removed
+0.020        org.bluez Device1.ConnectProfile("0000111f-…")                                returns 0.11-0.49 s later
+0.13-0.51    session  /org/pipewire/Telephony      org.ofono.Manager.ModemRemoved(.../ag1)   <- late, after ConnectProfile returns
+0.48-0.89    system   /                            ObjectManager.InterfacesAdded(<dev>, {org.bluez.Battery1: {Percentage}})
+0.50-0.96    session  /org/pipewire/Telephony      ObjectManager.InterfacesAdded(.../ag1, {AudioGateway1: {Address, SpeakerVolume 15, MicrophoneVolume 15},
                                                                                         AudioGatewayTransport1: {Codec 0, State "idle", RejectSCO true}})
              session  /org/pipewire/Telephony      org.ofono.Manager.ModemAdded(.../ag1, {})
+0.03-0.06 after the AG:
              session  /org/pipewire/Telephony/ag1  ObjectManager.InterfacesAdded(.../ag1/call1, {Call1: {LineIdentification, IncomingLine "", Name "",
                                                                                                        Multiparty false, State "active"}})
              session  /org/pipewire/Telephony/ag1  org.ofono.VoiceCallManager.CallAdded(.../ag1/call1, {...same...})
              session  /org/pipewire/Telephony/ag1  PropertiesChanged(AudioGateway1, {SpeakerVolume 15, MicrophoneVolume 15})
+0.4-0.7 after the AG:
              session  /org/pipewire/Telephony/ag1  PropertiesChanged(AudioGatewayTransport1, {Codec 3, RejectSCO true})
              then Codec 1, and often Codec 3, Codec 1 again, 50-250 ms apart; State stays "idle"
```

What the Simulated Phone must reproduce, and what the app must tolerate:

1. **The Call vanishes without ever reaching `State = "disconnected"`**, then reappears as a new object, already `active`. A Call that disappears this way during a Handoff is not an ended Call. The app must correlate the re-added Call with the one it had (same AudioGateway address, same `LineIdentification`, `active`) and keep its Call id and timer.
2. **`ModemRemoved` arrives late**, after `InterfacesRemoved` and after `ConnectProfile` has returned. Track AudioGateways with `ObjectManager` signals, not the oFono ones.
3. A Call genuinely ended by hang-up goes `State = "disconnected"` first, then `InterfacesRemoved` and `CallRemoved` in the same millisecond (§4).
4. The re-created AudioGateway always has `RejectSCO = true`, and the Phone makes one or two refused audio attempts shortly after.

### 3.3 Repeatability

5 of 5 Calls survived; the ticket asked for at least 3. No `ConnectProfile` retry was ever needed, and the Phone never reconnected the hands-free link on its own first.

### 3.4 Round trip [V]

In trial A5, after the bounce, `Set RejectSCO=false` plus `Activate()` on the new AudioGateway brought the audio back to the Desktop in 0.233 s. Handoff works in both directions, repeatedly, on the same Call.

## 4. Other sequences recorded [V]

**Dial (outgoing Call):**
```
AudioGateway1.Dial(<number>)                           returns at once
+0.36-0.90 s  InterfacesAdded(.../ag1/call1, {Call1: {LineIdentification, State "alerting", ...}}), then CallAdded
+0.5-1.4 s    Codec 3/1 flips on AudioGatewayTransport1 (Phone's refused SCO attempts)
+1.6-2.8 s    PropertiesChanged(Call1, {State: "active"}), then org.ofono.VoiceCall.PropertyChanged("State", "active")
```
The Call went straight to `alerting`, never `dialing`.

**Hang-up from the Desktop:**
```
Call1.Hangup()                                          returns in 0.01-0.53 s
(if audio was on the Desktop) +1.8 s PropertiesChanged(AudioGatewayTransport1, {State: "idle", ...})
+1.4-2.0 s    PropertiesChanged(Call1, {State: "disconnected"}), VoiceCall.PropertyChanged,
              InterfacesRemoved(.../call1, [Call1]), CallRemoved(.../call1)   all in the same millisecond
```
`Call1.Hangup` worked every time the service was up (9 of 9), so the Foner report of "Method doesn't exist" ([quirks](../wiki/telephony-api-quirks.md)) did not reproduce in normal operation. See §5.1 for the one exception, after a WirePlumber crash.

**`GetManagedObjects` on the AudioGateway lists the live Call** (snapshot, `method_results`). That contradicts the Foner report that it does not. Tracking Calls by signals is still right.

**`RejectSCO` change notifications are deferred:** `Set(RejectSCO)` emits no `PropertiesChanged` of its own. The new value rides along with the next transport change, e.g. `{Codec: 3, RejectSCO: false}` after `Activate()`. Read it back with `Get` if it matters.

## 5. Failures and hazards found

### 5.1 WirePlumber crashes on a malformed `Set` [V]

The probe first sent `Properties.Set` with D-Bus signature `ssb` instead of `ssv` (a dbus-python proxy without introspection guesses the signature). WirePlumber 0.5.18 died with `SIGSEGV` in `libdbus` called from `libspa-bluez5.so` (core dump `coredumpctl info 114757`, 18:09:57; Arch publishes no symbols for these frames). PipeWire 1.6.9 `telephony.c:862` (`ag_properties_set`) reads only the two strings with `dbus_message_get_args`, then calls `dbus_message_iter_recurse` on the third argument without checking that it is a variant. [I] That recursion on a non-container is the crash.

Consequences, all observed:
- `org.pipewire.Telephony` lost its owner, every A2DP, AVRCP and hands-free object for the Phone was torn down, and all of the Desktop's other audio devices were re-created. WirePlumber restarted in 0.8 s, and the AudioGateway and the still-active Call were re-adopted 1.3 s after the crash.
- The probe's hang-up landed in that 0.8 s gap (`ServiceUnknown`) and was not retried (the probe now retries for 30 s). When checked by hand shortly afterwards, `GetCalls` still listed the Call as `active`, but `Properties.Get` and `GetAll` on `.../ag1/call1` returned "Method … doesn't exist". By the next check, about two minutes after dialling, the Call was gone. Nothing on the Desktop hung it up; voicemail's own time-out is the likely cause [I]. This is the only Call the probe did not hang up itself.

For the product: always send `Set` with an explicit `ssv` signature (zbus does this by construction), and treat a WirePlumber restart (`NameOwnerChanged` on `org.pipewire.Telephony`) as a re-adoption, exactly like the bounce. The missing signature check is worth reporting upstream; nothing was submitted.

### 5.2 Hazard: Desktop control is lost for up to about 1 s

During a bounce the Desktop cannot hang up, mute or send tones. A hang-up pressed in that window has to be queued until the Call is re-adopted. If the reconnect ever failed, the Call would carry on on the Phone with no Desktop control at all; it never failed in these trials.

### 5.3 Not tested

- Music playing during a bounce.
- Earbuds connected at the same time: Android might move the Call's audio to them on reconnect instead of the earpiece [I].
- Held Calls, waiting Calls and three-way Calls during a bounce.
- Incoming Calls; every trial was an outgoing Call to voicemail.

## 6. Candidates B and C

Not pursued. The ticket orders B only if A fails, and C (a local PipeWire patch adding `AudioGatewayTransport1.Deactivate()`) would need the user's OK to replace the system PipeWire. Neither is needed now. A `Deactivate()` upstream would still be the cleaner long-term fix, since it would avoid the control gap in §5.2. No merge request text was drafted or submitted.

## 7. D-Bus object snapshot

Committed for the Simulated Phone conformance ticket in [`2026-11-04-dbus-snapshot/`](2026-11-04-dbus-snapshot/):

- `live-call.json`: captured at A1 with a live Call whose audio was on the Desktop. For each object it holds the bus, service, path, the full `Introspect` XML and `GetAll` for every interface with properties, with D-Bus types (from `busctl --json=short`). Objects: `/org/pipewire/Telephony`, `/org/pipewire/Telephony/ag1`, `/org/pipewire/Telephony/ag1/call1`, BlueZ `/org/bluez/hci0` (`Adapter1` and siblings), the Phone's `Device1` (with `Network1`, `MediaControl1`, `Battery1`) and its A2DP `MediaTransport1`. It also records the results of `GetManagedObjects` (manager and AG), `GetModems` and `GetCalls`.
- `pbap-session.json`: an obexd PBAP session (`Client1.CreateSession(addr, {Target: "pbap"})`), captured right after creation and after `Select("int", "pb")`: `Session1` and `PhonebookAccess1` introspection and properties. No Contacts were pulled; the contact count is left out.

Redactions: the Test Number appears as `<TEST_NUMBER>` in `LineIdentification`. Nothing else was changed.

Notable values: `Codec = 3` (LC3-SWB) while audio is on the Desktop; the manager object has no `Properties` interface; `GetModems` returns an empty property dictionary; PBAP `Session1.Channel = 0` and `PSM = 0` (L2CAP), `Folder = "/telecom/pb"` after `Select`, and all-zero `DatabaseIdentifier`, `PrimaryCounter` and `SecondaryCounter`.
