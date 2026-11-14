#!/usr/bin/python3
"""Handoff probe for the real Phone, from the ticket #3 prototype.

Places a Call to the Test Number, brings its audio to the Desktop, then
tries a Handoff back to the Phone while timestamping every D-Bus signal
from org.pipewire.Telephony and the Phone's org.bluez objects, plus
`pactl subscribe`. Findings: the 2026-11-04 research note on the Handoff to the Phone prototype.

Usage:
  /usr/bin/python3 tools/real-phone/handoff_probe.py trial-a [--snapshot OUT.json] [--round-trip]
      Handoff to the Phone by bouncing the hands-free profile (ADR-0005).
  /usr/bin/python3 tools/real-phone/handoff_probe.py trial-b
      Handoff by toggling the card profile off and back.
  /usr/bin/python3 tools/real-phone/handoff_probe.py activate-only
      Only bring the Call's audio to the Desktop.
  /usr/bin/python3 tools/real-phone/handoff_probe.py pbap-snapshot OUT.json
      Capture an obexd PBAP Session1 and PhonebookAccess1 (no Call).
Options:
  --trace      raise WirePlumber's bluez5 log level once the Call is active
  --no-lift    call Activate() without lifting RejectSCO first

Safety, all through real_phone.py: Calls only the Test Number, refuses to
start if any Call exists, hangs up only the Call to the Test Number (with
retries across a WirePlumber restart, and Ctrl-C deferred while it does),
checks that GetCalls is empty at the end, writes properties only through
the typed `set_property`, and redacts the Test Number from all output.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import time
from typing import Any

import dbus
import dbus.mainloop.glib
from gi.repository import GLib

import real_phone as rp
from real_phone import log

HFP_AG_UUID = "0000111f-0000-1000-8000-00805f9b34fb"

NUMBER = rp.read_test_number()
rp.install_redaction(NUMBER)

dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
CTX = GLib.MainContext.default()
TEL = rp.Telephony(NUMBER)
SESSION = TEL.session
SYSTEM = dbus.SystemBus()

# Live state, kept from signals so timings are measured to the signal.
STATE: dict[str, Any] = {"ag": None, "calls": {}, "transport": {}}
PHONE: dict[str, str] = {"address": "", "device_path": "", "card": ""}


# ---------------------------------------------------------------- signals


def on_telephony_signal(*args: Any, **kw: Any) -> None:
    member, path, iface = kw["member"], kw["path"], kw["interface"]
    a = [rp.plain(x) for x in args]
    if member == "InterfacesAdded":
        obj, ifaces = a
        if ifaces.get(rp.AUDIO_GATEWAY_IF, {}).get("Address") == PHONE["address"]:
            STATE["ag"] = obj
            STATE["transport"] = dict(ifaces.get(rp.TRANSPORT_IF, {}))
        if rp.CALL_IF in ifaces:
            STATE["calls"][obj] = dict(ifaces[rp.CALL_IF])
    elif member == "InterfacesRemoved":
        obj, ifaces = a
        if obj == STATE["ag"] and rp.AUDIO_GATEWAY_IF in ifaces:
            STATE["ag"] = None
            STATE["transport"] = {}
        if rp.CALL_IF in ifaces:
            STATE["calls"].pop(obj, None)
    elif member == "PropertiesChanged":
        ifname, changed, _invalidated = a
        if ifname == rp.TRANSPORT_IF and path == STATE["ag"]:
            STATE["transport"].update(changed)
        if ifname == rp.CALL_IF and path in STATE["calls"]:
            STATE["calls"][path].update(changed)
    log("TEL-SIG", f"{path} {iface}.{member} {json.dumps(a)}")


def on_bluez_signal(*args: Any, **kw: Any) -> None:
    path = kw["path"]
    a = [rp.plain(x) for x in args]
    dev = PHONE["device_path"]
    if not (path.startswith(dev) or (a and str(a[0]).startswith(dev))):
        return
    log("BLUEZ-SIG", f"{path} {kw['interface']}.{kw['member']} {json.dumps(a)}")


def on_name_owner(name: str, old: str, new: str) -> None:
    if name == rp.TELEPHONY:
        log("NAME", f"{name} owner {old!r} -> {new!r}")


def subscribe() -> subprocess.Popen[str]:
    SESSION.add_signal_receiver(on_telephony_signal, bus_name=rp.TELEPHONY, path_keyword="path",
                                member_keyword="member", interface_keyword="interface")
    SYSTEM.add_signal_receiver(on_bluez_signal, bus_name=rp.BLUEZ, path_keyword="path",
                               member_keyword="member", interface_keyword="interface")
    SESSION.add_signal_receiver(on_name_owner, signal_name="NameOwnerChanged",
                                dbus_interface="org.freedesktop.DBus")
    # pactl subscribe: timestamps when the hands-free and A2DP nodes and the card change.
    pactl = subprocess.Popen(["pactl", "subscribe"], stdout=subprocess.PIPE,
                             stderr=subprocess.DEVNULL, text=True, bufsize=1)

    def on_pactl(_fd: object, _cond: object) -> bool:
        assert pactl.stdout is not None
        line = pactl.stdout.readline()
        if line and any(w in line for w in ("sink", "source", "card")) and "client" not in line:
            log("PACTL", line.strip())
        return True

    GLib.io_add_watch(pactl.stdout, GLib.PRIORITY_DEFAULT, GLib.IO_IN, on_pactl)
    return pactl


def pump(seconds: float) -> None:
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if not CTX.iteration(False):
            time.sleep(0.005)


def wait_for(pred: Any, timeout: float, what: str) -> bool:
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if pred():
            log("OK", what)
            return True
        if not CTX.iteration(False):
            time.sleep(0.005)
    log("TIMEOUT", f"{what} (after {timeout}s)")
    return False


# ---------------------------------------------------------------- the Phone


def tel_call(path: str, iface: str, member: str, *args: Any, signature: str) -> Any:
    return rp.call(SESSION, rp.TELEPHONY, path, iface, member, *args, signature=signature)


def device1(member: str, *args: Any, signature: str, timeout: float = 30.0) -> Any:
    return rp.call(SYSTEM, rp.BLUEZ, PHONE["device_path"], rp.DEVICE1_IF, member, *args,
                   signature=signature, timeout=timeout)


def set_reject_sco(ag: str, value: bool) -> None:
    rp.set_property(SESSION, rp.TELEPHONY, ag, rp.TRANSPORT_IF, "RejectSCO", dbus.Boolean(value))


def bluez_objects() -> dict[str, dict[str, Any]]:
    return rp.call(SYSTEM, rp.BLUEZ, "/", rp.OBJECT_MANAGER_IF, "GetManagedObjects", signature="")


def find_phone() -> None:
    """Fill PHONE from the single AudioGateway and BlueZ's matching Device1."""
    ag = TEL.audio_gateway()
    if ag is None:
        sys.exit("No single AudioGateway for the Phone; is hands-free connected?")
    PHONE["address"] = TEL.phone_address or ""
    for path, ifaces in bluez_objects().items():
        if ifaces.get(rp.DEVICE1_IF, {}).get("Address") == PHONE["address"]:
            PHONE["device_path"] = path
    if not PHONE["device_path"]:
        sys.exit(f"BlueZ has no Device1 with address {PHONE['address']}.")
    PHONE["card"] = "bluez_card." + PHONE["address"].replace(":", "_")


def a2dp_state() -> dict[str, Any]:
    return {p: i["org.bluez.MediaTransport1"].get("State") for p, i in bluez_objects().items()
            if p.startswith(PHONE["device_path"]) and "org.bluez.MediaTransport1" in i}


def card_profile() -> str | None:
    out = subprocess.run(["pactl", "list", "cards"], capture_output=True, text=True).stdout
    block = out.split(f"Name: {PHONE['card']}")[-1] if PHONE["card"] in out else ""
    m = re.search(r"Active Profile: (\S+)", block)
    return m.group(1) if m else None


def bt_nodes() -> list[str]:
    """PipeWire nodes for the Phone. In the hands-free role they are streams."""
    dump = json.loads(subprocess.run(["pw-dump"], capture_output=True, text=True).stdout or "[]")
    out = []
    for o in dump:
        info = o.get("info") or {}
        props = info.get("props") or {}
        if o.get("type", "").endswith(":Node") and "bluez" in str(props.get("node.name", "")):
            out.append(f'{props.get("node.name")}({props.get("media.class")},{info.get("state")})')
    return out


def media_report(label: str) -> None:
    connected = rp.call(SYSTEM, rp.BLUEZ, PHONE["device_path"], rp.PROPERTIES_IF, "Get",
                        rp.DEVICE1_IF, "Connected", signature="ss")
    log("MEDIA", f"{label}: card profile={card_profile()} a2dp={a2dp_state()} "
                 f"nodes={bt_nodes()} Device1.Connected={connected}")


def my_call() -> str | None:
    """The Call to the Test Number, from signal-kept state."""
    for path, props in STATE["calls"].items():
        if TEL.is_test_call(path, props):
            return path
    return None


def adopt_calls() -> None:
    if STATE["ag"]:
        STATE["calls"].update(TEL.calls(STATE["ag"]))


# ---------------------------------------------------------------- snapshot


def busctl_json(bus: str, *args: str) -> Any:
    r = subprocess.run(["busctl", f"--{bus}", "--json=short", *args], capture_output=True, text=True)
    return json.loads(r.stdout) if r.returncode == 0 and r.stdout.strip() else {"error": r.stderr.strip()}


def snap_object(bus: str, service: str, path: str) -> dict[str, Any]:
    r = busctl_json(bus, "call", service, path, rp.INTROSPECTABLE_IF, "Introspect")
    xml = r["data"][0] if "data" in r else str(r)
    props = {}
    # PipeWire quotes attributes with ', BlueZ with "
    for m in re.finditer(r"<interface name=[\"']([^\"']+)[\"']>(.*?)</interface>", xml, re.S):
        i, body = m.group(1), m.group(2)
        if i.startswith("org.freedesktop.DBus."):
            continue
        if "<property" in body:
            props[i] = busctl_json(bus, "call", service, path, rp.PROPERTIES_IF, "GetAll", "s", i)
    return {"bus": bus, "service": service, "path": path, "introspection_xml": xml, "properties": props}


def write_snapshot(out: str) -> None:
    ag = STATE["ag"]
    objs = [("user", rp.TELEPHONY, rp.TELEPHONY_ROOT), ("user", rp.TELEPHONY, ag)]
    objs += [("user", rp.TELEPHONY, p) for p in STATE["calls"]]
    objs += [("system", rp.BLUEZ, "/org/bluez/hci0"), ("system", rp.BLUEZ, PHONE["device_path"])]
    objs += [("system", rp.BLUEZ, p) for p in a2dp_state()]
    root = rp.TELEPHONY_ROOT
    calls = {
        f"GetManagedObjects({root})": busctl_json("user", "call", rp.TELEPHONY, root,
                                                  rp.OBJECT_MANAGER_IF, "GetManagedObjects"),
        f"GetManagedObjects({ag})": busctl_json("user", "call", rp.TELEPHONY, ag,
                                                rp.OBJECT_MANAGER_IF, "GetManagedObjects"),
        f"GetModems({root})": busctl_json("user", "call", rp.TELEPHONY, root,
                                          "org.ofono.Manager", "GetModems"),
        f"GetCalls({ag})": busctl_json("user", "call", rp.TELEPHONY, ag,
                                       rp.CALL_MANAGER_IF, "GetCalls"),
    }
    data = {"captured": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "phone": PHONE["address"],
            "objects": [snap_object(*o) for o in objs], "method_results": calls}
    with open(out, "w") as f:
        f.write(TEL.redact(json.dumps(data, indent=2)) + "\n")
    log("SNAPSHOT", f"wrote {len(data['objects'])} objects to {out}")


# ---------------------------------------------------------------- the trials


def place_call_and_activate() -> bool:
    ag = TEL.audio_gateway()
    assert ag is not None
    STATE["ag"] = ag
    STATE["transport"] = rp.call(SESSION, rp.TELEPHONY, ag, rp.PROPERTIES_IF, "GetAll",
                                 rp.TRANSPORT_IF, signature="s")
    existing = TEL.all_calls()
    if existing:
        sys.exit(f"Refusing: {len(existing)} Call(s) already exist; never touch real Calls.")
    log("PRE", f"ag={ag} transport={STATE['transport']}")
    media_report("before")

    log("ACTION", "AudioGateway1.Dial(<TEST_NUMBER>)")
    TEL.dial_test_number(ag)
    if not wait_for(lambda: my_call() is not None, 15, "Call object for our Dial appeared"):
        adopt_calls()
        if not my_call():
            return False
    if not wait_for(lambda: STATE["calls"].get(my_call(), {}).get("State") == "active", 45,
                    "our Call is active (voicemail answered)"):
        return False
    pump(1.0)
    log("CHECK", f"before Activate: transport={STATE['transport']}")
    if "--trace" in sys.argv:  # only after the Call is active: ATD<number> never reaches the journal
        subprocess.run(["wpctl", "set-log-level", "spa.bluez5*:T,*:W"])
        log("TRACE", "WirePlumber bluez5 log level -> T")
    # PipeWire 1.6.9: Activate() only sends AT+BCC; sco_listen_event still rejects
    # the Phone's SCO while RejectSCO=true. So lift it first.
    if "--no-lift" not in sys.argv:
        set_reject_sco(ag, False)
        log("ACTION", "Set RejectSCO=false")
    t = time.monotonic()
    ok = False
    for attempt in range(1, 4):
        ta = time.monotonic()
        try:
            tel_call(ag, rp.TRANSPORT_IF, "Activate", signature="")
            log("ACTION", f"Activate() #{attempt} returned in {time.monotonic()-ta:.3f}s")
        except dbus.DBusException as e:
            log("ERR", f"Activate() #{attempt}: {e.get_dbus_name()} {e.get_dbus_message()}")
        ok = wait_for(lambda: STATE["transport"].get("State") == "active", 4,
                      f"transport active (audio on Desktop) after Activate #{attempt}")
        if ok:
            break
    log("TIMING", f"Activate -> transport active: {time.monotonic()-t:.3f}s")
    if "--no-lift" not in sys.argv:
        set_reject_sco(ag, True)
        log("ACTION", "Set RejectSCO=true (re-arm for later SCO attempts)")
    media_report("audio on Desktop")
    return ok


def candidate_a() -> bool:
    log("ACTION", "Device1.DisconnectProfile(HFP AG)")
    t_disc = time.monotonic()
    try:
        device1("DisconnectProfile", HFP_AG_UUID, signature="s")
        log("RETURN", f"DisconnectProfile returned in {time.monotonic()-t_disc:.3f}s")
    except dbus.DBusException as e:
        log("ERR", f"DisconnectProfile: {e.get_dbus_name()} {e.get_dbus_message()}")
    wait_for(lambda: STATE["ag"] is None, 10, "AudioGateway removed")
    t_gone = time.monotonic()
    media_report("after disconnect")

    log("ACTION", "Device1.ConnectProfile(HFP AG)")
    t_conn = time.monotonic()
    for attempt in range(3):
        try:
            device1("ConnectProfile", HFP_AG_UUID, signature="s")
            log("RETURN", f"ConnectProfile returned in {time.monotonic()-t_conn:.3f}s")
            break
        except dbus.DBusException as e:
            log("ERR", f"ConnectProfile try {attempt+1}: {e.get_dbus_name()} {e.get_dbus_message()}")
            if STATE["ag"]:
                break
            pump(1.0)
    wait_for(lambda: STATE["ag"] is not None, 20, "AudioGateway re-added")
    readopt = wait_for(lambda: my_call() is not None, 10, "our Call re-adopted")
    t_ctrl = time.monotonic()
    if not readopt and STATE["ag"]:
        adopt_calls()
        readopt = my_call() is not None
    state = STATE["calls"].get(my_call(), {}).get("State")
    log("RESULT", f"Call survived={readopt} state={state} "
                  f"audio-left-Desktop<= {t_gone-t_disc:.3f}s control-lost={t_ctrl-t_disc:.3f}s")
    ag = STATE["ag"]
    if ag:
        rs = rp.call(SESSION, rp.TELEPHONY, ag, rp.PROPERTIES_IF, "Get",
                     rp.TRANSPORT_IF, "RejectSCO", signature="ss")
        log("CHECK", f"new AG RejectSCO={rs}; watching transport 6s")
        pump(6.0)
        log("CHECK", f"transport after 6s={STATE['transport']}")
        if "--round-trip" in sys.argv and readopt:
            set_reject_sco(ag, False)
            t = time.monotonic()
            tel_call(ag, rp.TRANSPORT_IF, "Activate", signature="")
            log("ACTION", "round trip: Set RejectSCO=false + Activate() on the new AG")
            wait_for(lambda: STATE["transport"].get("State") == "active", 8,
                     "round trip: audio back on Desktop")
            log("TIMING", f"round trip Activate -> active: {time.monotonic()-t:.3f}s")
            set_reject_sco(ag, True)
    media_report("after reconnect")
    return readopt


def candidate_b() -> bool:
    before = card_profile() or "audio-gateway"
    log("ACTION", f"pactl set-card-profile {PHONE['card']} off (was {before})")
    t = time.monotonic()
    subprocess.run(["pactl", "set-card-profile", PHONE["card"], "off"], check=False)
    wait_for(lambda: STATE["transport"].get("State") != "active" or STATE["ag"] is None, 10,
             "transport no longer active / AG gone")
    media_report("profile off")
    pump(1.0)
    log("ACTION", f"pactl set-card-profile {PHONE['card']} {before}")
    subprocess.run(["pactl", "set-card-profile", PHONE["card"], before], check=False)
    wait_for(lambda: STATE["ag"] is not None, 20, "AudioGateway present")
    readopt = wait_for(lambda: my_call() is not None, 10, "our Call present")
    log("RESULT", f"Call survived={readopt} elapsed={time.monotonic()-t:.3f}s transport={STATE['transport']}")
    pump(4.0)
    media_report("after toggle")
    return readopt


def pbap_snapshot(out: str) -> None:
    obex = "org.bluez.obex"
    session = rp.call(SESSION, obex, "/org/bluez/obex", "org.bluez.obex.Client1", "CreateSession",
                      PHONE["address"], {"Target": "pbap"}, signature="sa{sv}", timeout=40.0)
    log("PBAP", f"Session1 {session}")
    try:
        snaps = {"after CreateSession": snap_object("user", obex, session)}
        rp.call(SESSION, obex, session, "org.bluez.obex.PhonebookAccess1", "Select",
                "int", "pb", signature="ss")
        snaps["after Select(int, pb)"] = snap_object("user", obex, session)
        data = {"captured": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "objects": snaps,
                "GetSize(pb)": "redacted (count of the user's Contacts)"}
        with open(out, "w") as f:
            f.write(TEL.redact(json.dumps(data, indent=2)) + "\n")
        log("SNAPSHOT", f"wrote {out}")
    finally:
        rp.call(SESSION, obex, "/org/bluez/obex", "org.bluez.obex.Client1", "RemoveSession",
                dbus.ObjectPath(session), signature="o")
        log("PBAP", "Session1 removed")


def main() -> int:
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd not in ("trial-a", "trial-b", "activate-only", "pbap-snapshot"):
        sys.exit(__doc__)
    find_phone()
    if cmd == "pbap-snapshot":
        pbap_snapshot(sys.argv[2])
        return 0
    snap = sys.argv[sys.argv.index("--snapshot") + 1] if "--snapshot" in sys.argv else None
    pactl = subscribe()
    rp.raise_on_interrupt()
    try:
        if place_call_and_activate():
            if snap:
                write_snapshot(snap)
            pump(2.0)
            if cmd == "trial-a":
                candidate_a()
            elif cmd == "trial-b":
                candidate_b()
    except rp.Interrupted as e:
        log("STOP", f"{e}: hanging up")
    finally:
        with rp.interrupts_deferred():
            if "--trace" in sys.argv:
                subprocess.run(["wpctl", "set-log-level", "W"])
                log("TRACE", "WirePlumber log level -> W")
            TEL.hang_up_test_call()
            no_call_remains = rp.report_remaining(TEL.remaining_calls())
            pactl.terminate()
    return 0 if no_call_remains else 2


if __name__ == "__main__":
    sys.exit(main())
