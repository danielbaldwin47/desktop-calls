# Simulated Phone conformance, Telephony signals and a WirePlumber restart mid-Call

Date: 2026-11-04. Ticket #6. Desktop and stack as in the [first Phone connection note](2026-11-04-first-phone-connection.md): PipeWire and WirePlumber with PipeWire 1.6.9 Telephony, BlueZ and obexd 5.87, the real Pixel 9a. Phone shapes from the [D-Bus snapshot](2026-11-04-dbus-snapshot/).
Legend: **[V]** = verified on this machine. **[I]** = inferred, not tested.

**Bottom line:** `crates/test-harness/tests/conformance.rs` holds the Simulated Phone to the snapshot, and it fails `just test` naming each difference. Real `InterfacesAdded` and `InterfacesRemoved` name exactly `AudioGateway1` and `AudioGatewayTransport1` for an AudioGateway, and only `Call1` for a Call. `just test-call` still hung up after one WirePlumber restart mid-Call: Telephony was back in 1.07 s, with the Call at the same path.

## 1. The conformance test

- It compares facts, not values: each interface, each method with its argument types, names and directions, each property with its type and access, each signal, and each annotation, from `Introspect`. It also compares which properties `GetAll` serves, with their types, and the reply signature of each method it calls. A difference reads "the Pixel has `X`, the Simulated Phone has `Y`" or "doesn't". **[V]**
- Objects covered: the Telephony manager, the AudioGateway with `AudioGatewayTransport1`, a ringing Call, `Adapter1` with `Media1` and `LEAdvertisingManager1`, `Device1` with `Network1`, `MediaControl1` and `Battery1`, and a PBAP session after `CreateSession` and after `Select("int", "pb")`. Also the `GetManagedObjects`, `GetModems` and `GetCalls` replies. **[V]**
- Mutations it caught, each reverted: renaming `SpeakerVolume` together with changing `SendTones`'s argument to `as` (5 differences, naming the signature and the property in `GetAll` and `GetManagedObjects`); `PairableTimeout` as `q` instead of `u`; renaming `Session1.Target`; letting Telephony's `Introspect` list children. **[V]**
- Not compared: values, by design (the Simulated Phone has its own address, and its Call rings). BlueZ's `GetManagedObjects` has no snapshot reply, so it is not compared. **[V]**

## 2. Simulated Phone bugs the test found

- The manager's `GetManagedObjects` listed Calls as well as AudioGateways. The real one lists only objects whose nearest `ObjectManager` is the manager, and each AudioGateway is the `ObjectManager` for its own Calls. **[V]** (snapshot) Fixed in `objects.rs`.
- The Simulated Phone had no Calls, no obexd and only `Adapter1` and `Device1` on the BlueZ objects. It now rings (control method `Ring(number)`), serves `org.bluez.obex` on a connection of its own, and serves the other BlueZ interfaces with the snapshot's values. **[V]**

## 3. Real Telephony signals

Watched with `tools/real-phone/watch_telephony.py`, which prints interface and property names and never values, during one `just test-call` with a WirePlumber restart.

- **Dial:** on `ag1`, `InterfacesAdded(call1, [Call1{IncomingLine, LineIdentification, Multiparty, Name, State}])`, then `CallAdded`. **[V]**
- **WirePlumber restart, old owner, in order:** on `ag1`, `InterfacesRemoved(call1, [Call1])` and `CallRemoved`; on the manager, `InterfacesRemoved(ag1, [AudioGateway1, AudioGatewayTransport1])`, then `ModemRemoved` at once (during a profile bounce it comes late; see the [prototype note](2026-11-04-handoff-to-phone-prototype.md) §3.2). Then it released `org.pipewire.Telephony`. **[V]**
- **New owner, 0.73 s later:** `InterfacesAdded(ag1, [AudioGateway1{Address, MicrophoneVolume, SpeakerVolume}, AudioGatewayTransport1{Codec, RejectSCO, State}])`, `ModemAdded`, then, 40 ms later, `InterfacesAdded(call1, [Call1{...}])` and `CallAdded`. **[V]**
- **Hang-up:** `InterfacesRemoved(call1, [Call1])`, `CallRemoved`. **[V]**
- So the answer to the [test seam note](2026-11-04-test-seam.md)'s open question is yes: the signals name exactly the interfaces `GetManagedObjects` carries. **[V]**
- After a restart the paths start again at `ag1` and `call1`, so the same Call comes back at the same path under a new unique name. A profile bounce, by contrast, registers the next `agN`. **[V]** for the restart. The app should track the owner of `org.pipewire.Telephony`, not just paths. **[I]**

## 4. WirePlumber restart mid-Call (Q30)

`tools/real-phone/wireplumber_restart_probe.py`, run as the `just test-call` hook. Exactly one restart.

- Before restarting: no Call other than the test Call in `GetCalls`, and `pactl` listed no sink inputs and no source outputs. **[V]**
- `systemctl --user restart wireplumber` returned in 0.064 s; a new owner of `org.pipewire.Telephony` after 0.770 s; `ag1` back after 1.072 s; `ag1/call1` back, `active`, after 1.073 s. **[V]**
- `just test-call` then hung up with `Call1.Hangup` (returned in 0.020 s). The Call was gone 6.048 s after dialling, `GetCalls` was empty, exit status 0. **[V]**
- Not tested: a hang-up sent during the 1 s gap itself, while no Telephony owner exists. **[I]** It would fail with `ServiceUnknown` and need a retry once the Call reappears.

## 5. obexd's client object

- `Introspect` on `/org/bluez/obex` (read-only `busctl`, obexd 5.87): `org.bluez.obex.Client1` (`CreateSession(in s destination, in a{sv} args, out o session)`, `RemoveSession(in o session)`), `org.bluez.obex.AgentManager1` (`RegisterAgent(in o agent)`, `UnregisterAgent(in o agent)`), `Introspectable`; no `Properties` and no child nodes. **[V]** Saved verbatim as `crates/simulated-phone/introspection/obex-client.xml`. It is not in the committed snapshot. **[V]**
- obexd's `/` has `ObjectManager` and lists child `org`; `/org/bluez/obex/client` introspects as an empty node until a session exists. **[V]**

## 6. Known gaps in the Simulated Phone

- `Battery1`, `MediaControl1` and `Network1` are static. The real `Battery1` comes and goes with hands-free (prototype note). **[V]** (real); the Simulated Phone keeps it.
- The A2DP `MediaTransport1` is neither simulated nor compared. "Transport" in #6 was read as `AudioGatewayTransport1`. **[I]** that this is the intended reading.
- `Ring` makes only a ringing Call: answering and hanging up are #7.
- The Simulated Phone emits no `ModemAdded` or `ModemRemoved`.
- PBAP sessions open at once, and nothing can be pulled yet.
