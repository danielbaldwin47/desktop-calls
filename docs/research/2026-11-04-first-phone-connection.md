# First Phone connection: the "missing" AudioGateway object

Date: 2026-11-04, 17:39-17:45 EST. Phone: Pixel 9a `AA:BB:CC:DD:EE:FF`. Stack as in [HFP stack research](2026-11-04-hfp-stack.md) §0.
Legend: **[V]** = verified on this machine. **[I]** = inferred, not tested.

**Bottom line:** nothing was broken. The HFP link and the service level connection (SLC) were up, and
`/org/pipewire/Telephony/ag1` existed the whole time. `busctl --user tree` cannot see it, because PipeWire's
`Introspect` reply for `/org/pipewire/Telephony` lists no child `<node>` elements and `busctl tree` discovers
objects only through those. The object is found through `org.freedesktop.DBus.ObjectManager.GetManagedObjects`.
No config change was needed or made.

---

## 1. Symptom

- `busctl --user tree org.pipewire.Telephony` showed only `/org/pipewire/Telephony`.
- `pactl list cards`: `bluez_card.AA_BB_CC_DD_EE_FF`, Active Profile `audio-gateway`, sinks 0, sources 0.
- The user confirmed that on the Phone, this Desktop's Bluetooth device settings have "Phone calls" and all other toggles on. Ruled out.

## 2. Diagnosis

1. **[V] BlueZ side is healthy.** `bluetoothctl info`: paired, bonded, trusted, connected; Phone advertises `0000111f` (HFP AG). Adapter advertises `0000111e` (Handsfree), so PipeWire's HF profile is registered. A2DP transport exists (`/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF/sep4/fd0`, UUID `0000110b`, State `idle`).
2. **[V] An RFCOMM socket is open and owned by WirePlumber.** `/proc/net/rfcomm` lists 3 sockets; inode `365576` is fd 53 of `wireplumber` (PID 114757). The other two are the listening sockets. WirePlumber also holds a SCO socket (inode `361520`, fd 50).
3. **[V] `ConnectProfile(0000111f-…)` returned success instantly with no log activity**, which is consistent with the profile already being connected.
4. **[V] Forced a reconnect with trace logging** to see the full exchange:
   ```
   wpctl set-log-level 'spa.bluez5*:T,*:W'
   busctl call org.bluez /org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF org.bluez.Device1 DisconnectProfile s 0000111f-0000-1000-8000-00805f9b34fb
   busctl call org.bluez /org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF org.bluez.Device1 ConnectProfile    s 0000111f-0000-1000-8000-00805f9b34fb
   journalctl --user -u wireplumber --since 17:43:50
   ```
   The disconnect logged `removing AudioGateway: /org/pipewire/Telephony/ag1`, which proves the AG object existed **before** any intervention. The reconnect (trimmed):
   ```
   17:43:54 NewConnection path=/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF, fd=53, profile /Profile/HFPHF
   adapter /org/bluez/hci0: has msbc/esco transport          (alt6:1)
   RFCOMM >> AT+BRSF=695            << +BRSF: 3951  OK
   RFCOMM >> AT+BAC=1,3,127,2       << OK                     (CVSD, LC3-SWB, LC3-A127, mSBC offered)
   RFCOMM >> AT+CIND=?              << +CIND: ("call",(0,1)),("callsetup",(0-3)),("service",(0-1)),("signal",(0-5)),("roam",(0,1)),("battchg",(0-5)),("callheld",(0-2))
   RFCOMM >> AT+CIND?               << +CIND: 0,0,1,4,0,2,0
   RFCOMM >> AT+CMER=3,0,0,1        << OK
   RFCOMM >> AT+CHLD=?              << +CHLD: (0,1,2,3)  OK
   telephony_ag_register: registered AudioGateway: /org/pipewire/Telephony/ag1
   AT+CLIP=1 OK, AT+CCWA=1 OK, AT+CMEE=1 (+BSIR: 0, +BSIR: 1) OK, AT+CLCC OK, AT+VGS=15 OK, AT+VGM=15 OK
   ```
   - **[V] The AG object is registered right after `AT+CHLD=?` completes, i.e. when the SLC is established.** It is removed on `RequestDisconnection`. So AG objects appear only after the SLC.
   - **[V]** The Phone's `+BRSF: 3951` (0xF6F) includes codec negotiation (bit 9), HF indicators (bit 10) and eSCO S4 (bit 11). `+BSIR: 1` means in-band ringtone is on.
   - **[V]** The HF side also creates a BlueZ `Battery1` for the Phone from the `battchg` indicator (`Created virtual battery for AA:BB:CC:DD:EE:FF`).
5. **[V] Why `busctl tree` misses it:**
   ```
   $ busctl --user call org.pipewire.Telephony /org/pipewire/Telephony org.freedesktop.DBus.Introspectable Introspect | grep -o '<node[^>]*>'
   <node>                       # no <node name="ag1"/> child
   $ busctl --user call org.pipewire.Telephony /org/pipewire/Telephony org.freedesktop.DBus.ObjectManager GetManagedObjects
   a{oa{sa{sv}}} 1 "/org/pipewire/Telephony/ag1" 2 "org.pipewire.Telephony.AudioGateway1" 3 "Address" s "AA:BB:CC:DD:EE:FF" ...
   ```
   The check suggested in the HFP stack research ("run `busctl --user tree` and confirm `ag*` appears") is therefore wrong for this service. Use `GetManagedObjects`.

Log level was restored with `wpctl set-log-level W`.

## 3. The live AudioGateway object (17:44, idle, no call)

`busctl --user introspect org.pipewire.Telephony /org/pipewire/Telephony/ag1`:

| Interface | Member | Kind | Signature | Value / flags |
|---|---|---|---|---|
| `org.pipewire.Telephony.AudioGateway1` | `Address` | property | `s` | `"AA:BB:CC:DD:EE:FF"`, const |
| | `SpeakerVolume` | property | `y` | `15`, emits-change, writable |
| | `MicrophoneVolume` | property | `y` | `15`, emits-change, writable |
| | `Dial` | method | `s` → | |
| | `SendTones` | method | `s` → | |
| | `CreateMultiparty`, `HangupAll`, `HoldAndAnswer`, `ReleaseAndAnswer`, `ReleaseAndSwap`, `SwapCalls` | methods | → | |
| `org.pipewire.Telephony.AudioGatewayTransport1` | `Activate` | method | → | |
| | `Codec` | property | `y` | `0` (no codec selected while idle), emits-change |
| | `State` | property | `s` | `"idle"`, emits-change |
| | `RejectSCO` | property | `b` | `true` (from `bluez5.telephony.default-reject-sco = true`), emits-change, writable |
| `org.ofono.VoiceCallManager` | same methods as `AudioGateway1` plus `GetCalls` (`→ a{oa{sv}}`) | | | signals `CallAdded(oa{sv})`, `CallRemoved(o)` |
| `org.freedesktop.DBus.ObjectManager` | `GetManagedObjects` | method | `→ a{oa{sa{sv}}}` | signals `InterfacesAdded`, `InterfacesRemoved` |
| `org.freedesktop.DBus.Properties` | `Get`, `GetAll`, `Set` | | | signal `PropertiesChanged(sa{sv}as)` |
| `org.freedesktop.DBus.Introspectable` | `Introspect` | | | |

Other calls at idle:
```
GetModems  (manager, org.ofono.Manager)          → a{oa{sv}} 1 "/org/pipewire/Telephony/ag1" 0     # empty property dict
GetCalls   (ag1, org.ofono.VoiceCallManager)     → a{oa{sv}} 0
GetManagedObjects (ag1)                          → a{oa{sa{sv}}} 0
```
The manager `/org/pipewire/Telephony` exposes `org.ofono.Manager`, `ObjectManager` and `Introspectable` only (no `Properties`).

## 4. Consequences for the build

1. **Discover AGs only through `ObjectManager`**: `GetManagedObjects` on `/org/pipewire/Telephony`, then `InterfacesAdded`/`InterfacesRemoved`. Never walk `Introspect` children. [V]
2. **Do not hard-code `ag0`.** This Phone's first object was `ag1`; it was `ag1` again after a reconnect. [V] How the index is chosen was not checked. Match AGs by the `Address` property. [I]
3. **AG lifetime = SLC lifetime.** The object disappears on HFP disconnect and returns on reconnect. The app's "Phone connected" state should follow it. [V]
4. **`GetModems` carries no properties**, so read `Address` from `GetManagedObjects` or `Properties.GetAll`. [V]
5. **`Codec` is `0` while idle.** Expect it to change when SCO opens (`Activate()` or a call); not yet observed. [I]

## 5. Current state (17:44)

HFP connected, SLC complete, `/org/pipewire/Telephony/ag1` present with `RejectSCO = true`. No files changed.
