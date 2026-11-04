#!/usr/bin/env python3
"""PROTOTYPE -- THROWAWAY. Ticket #3. Never import this into the product.

Question it answers: when PipeWire 1.6.9 has AudioGatewayTransport1.Activate()
but no Deactivate(), how does a live Call's Audio Location move from the
Desktop back to the Phone on the real Pixel 9a?

Usage:
  python3 handoff_probe.py trial-a [--snapshot OUT.json]   # profile bounce
  python3 handoff_probe.py trial-b                         # card profile toggle
  python3 handoff_probe.py pbap-snapshot OUT.json          # obexd session objects

Safety rules baked in:
  * Calls only the Test Number in ~/.config/desktop-calls/test-number.
  * Refuses to start if any Call already exists (never touches real Calls).
  * Hangs up only the Call it placed, matched by LineIdentification.
  * Redacts the Test Number from everything it prints or writes.
"""
import json
import os
import re
import subprocess
import sys
import time

import dbus
import dbus.mainloop.glib
from gi.repository import GLib

PHONE_ADDR = "AA:BB:CC:DD:EE:FF"
DEV_PATH = "/org/bluez/hci0/dev_" + PHONE_ADDR.replace(":", "_")
CARD = "bluez_card." + PHONE_ADDR.replace(":", "_")
HFP_AG_UUID = "0000111f-0000-1000-8000-00805f9b34fb"
TEL = "org.pipewire.Telephony"
TEL_ROOT = "/org/pipewire/Telephony"
AG_IF = TEL + ".AudioGateway1"
TR_IF = TEL + ".AudioGatewayTransport1"
CALL_IF = TEL + ".Call1"
OM_IF = "org.freedesktop.DBus.ObjectManager"
PROPS_IF = "org.freedesktop.DBus.Properties"

NUMBER = open(os.path.expanduser("~/.config/desktop-calls/test-number")).read().strip()
_DIGITS = re.sub(r"\D", "", NUMBER)


def redact(s):
    s = str(s)
    if _DIGITS:
        # any digit run ending in the last 7 digits of the Test Number
        s = re.sub(r"\+?\d*" + re.escape(_DIGITS[-7:]), "<TEST_NUMBER>", s)
    return s


dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
SESSION = dbus.SessionBus()
SYSTEM = dbus.SystemBus()
CTX = GLib.MainContext.default()

T0 = time.monotonic()
EVENTS = []


def log(kind, detail=""):
    t = time.monotonic() - T0
    line = redact(f"+{t:8.3f}s  {kind:<10} {detail}")
    EVENTS.append(line)
    print(line, flush=True)


def py(v):
    """dbus value -> plain python, for logging."""
    if isinstance(v, (dbus.Dictionary, dict)):
        return {str(k): py(x) for k, x in v.items()}
    if isinstance(v, (dbus.Array, list, tuple)):
        return [py(x) for x in v]
    if isinstance(v, dbus.Boolean):
        return bool(v)
    if isinstance(v, (dbus.Byte, dbus.Int16, dbus.Int32, dbus.Int64,
                      dbus.UInt16, dbus.UInt32, dbus.UInt64)):
        return int(v)
    if isinstance(v, (dbus.String, dbus.ObjectPath)):
        return str(v)
    return v


# ---------------------------------------------------------------- live state
STATE = {"ag": None, "calls": {}, "transport": {}, "tel_owner": None}


def on_tel_signal(*args, **kw):
    member, path, iface = kw["member"], kw["path"], kw["interface"]
    a = [py(x) for x in args]
    if member == "InterfacesAdded":
        obj, ifaces = a
        if AG_IF in ifaces and ifaces[AG_IF].get("Address") == PHONE_ADDR:
            STATE["ag"] = obj
            STATE["transport"] = dict(ifaces.get(TR_IF, {}))
        if CALL_IF in ifaces:
            STATE["calls"][obj] = dict(ifaces[CALL_IF])
    elif member == "InterfacesRemoved":
        obj, ifaces = a
        if obj == STATE["ag"] and AG_IF in ifaces:
            STATE["ag"] = None
            STATE["transport"] = {}
        if CALL_IF in ifaces:
            STATE["calls"].pop(obj, None)
    elif member == "PropertiesChanged":
        ifname, changed, _inv = a
        if ifname == TR_IF and path == STATE["ag"]:
            STATE["transport"].update(changed)
        if ifname == CALL_IF and path in STATE["calls"]:
            STATE["calls"][path].update(changed)
    log("TEL-SIG", f"{path} {iface}.{member} {json.dumps(a)}")


def on_bluez_signal(*args, **kw):
    path = kw["path"]
    a = [py(x) for x in args]
    if not (path.startswith(DEV_PATH) or (a and str(a[0]).startswith(DEV_PATH))):
        return
    log("BLUEZ-SIG", f"{path} {kw['interface']}.{kw['member']} {json.dumps(a)}")


def on_owner(name, old, new):
    if name == TEL:
        STATE["tel_owner"] = new
        log("NAME", f"{name} owner {old!r} -> {new!r}")


SESSION.add_signal_receiver(on_tel_signal, bus_name=TEL, path_keyword="path",
                            member_keyword="member", interface_keyword="interface")
SYSTEM.add_signal_receiver(on_bluez_signal, bus_name="org.bluez", path_keyword="path",
                           member_keyword="member", interface_keyword="interface")
SESSION.add_signal_receiver(on_owner, signal_name="NameOwnerChanged",
                            dbus_interface="org.freedesktop.DBus")

# pactl subscribe: timestamps when the HFP/A2DP nodes and card change.
_pactl = subprocess.Popen(["pactl", "subscribe"], stdout=subprocess.PIPE, text=True, bufsize=1)


def _on_pactl(fd, cond):
    line = _pactl.stdout.readline()
    if line and ("sink" in line or "source" in line or "card" in line) and "client" not in line:
        log("PACTL", line.strip())
    return True


GLib.io_add_watch(_pactl.stdout, GLib.PRIORITY_DEFAULT, GLib.IO_IN, _on_pactl)


def pump(seconds):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        CTX.iteration(False) or time.sleep(0.005)


def wait_for(pred, timeout, what):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if pred():
            log("OK", what)
            return True
        CTX.iteration(False) or time.sleep(0.005)
    log("TIMEOUT", f"{what} (after {timeout}s)")
    return False


def tel(path, iface):
    return dbus.Interface(SESSION.get_object(TEL, path, introspect=False), iface)


def find_ag():
    objs = py(tel(TEL_ROOT, OM_IF).GetManagedObjects())
    for p, ifaces in objs.items():
        if ifaces.get(AG_IF, {}).get("Address") == PHONE_ADDR:
            return p, ifaces
    return None, None


def get_calls(ag):
    return py(dbus.Interface(SESSION.get_object(TEL, ag, introspect=False),
                             "org.ofono.VoiceCallManager").GetCalls())


def device():
    return dbus.Interface(SYSTEM.get_object("org.bluez", DEV_PATH), "org.bluez.Device1")


def a2dp_state():
    objs = py(dbus.Interface(SYSTEM.get_object("org.bluez", "/"), OM_IF).GetManagedObjects())
    out = {p: i["org.bluez.MediaTransport1"].get("State") for p, i in objs.items()
           if p.startswith(DEV_PATH) and "org.bluez.MediaTransport1" in i}
    return out


def card_profile():
    out = subprocess.run(["pactl", "list", "cards"], capture_output=True, text=True).stdout
    block = out.split(f"Name: {CARD}")[-1] if CARD in out else ""
    m = re.search(r"Active Profile: (\S+)", block)
    return m.group(1) if m else None


def bt_nodes():
    """PipeWire nodes for the Phone. In the HF role they are streams, not sinks/sources."""
    dump = json.loads(subprocess.run(["pw-dump"], capture_output=True, text=True).stdout or "[]")
    out = []
    for o in dump:
        info = o.get("info") or {}
        props = info.get("props") or {}
        if o.get("type", "").endswith(":Node") and "bluez" in str(props.get("node.name", "")):
            out.append(f'{props.get("node.name")}({props.get("media.class")},{info.get("state")})')
    return out


def media_report(label):
    log("MEDIA", f"{label}: card profile={card_profile()} a2dp={a2dp_state()} "
                 f"nodes={bt_nodes()} Device1.Connected={bool(device_props()['Connected'])}")


def device_props():
    return py(dbus.Interface(SYSTEM.get_object("org.bluez", DEV_PATH), PROPS_IF)
              .GetAll("org.bluez.Device1"))


OURS = {"path": None, "dialled": False}


def my_call():
    """The one Call this probe placed. Never returns an incoming/waiting Call."""
    for p, props in STATE["calls"].items():
        if re.sub(r"\D", "", props.get("LineIdentification", "")).endswith(_DIGITS[-7:]):
            return p
    if OURS["path"] in STATE["calls"]:
        return OURS["path"]
    if OURS["dialled"]:
        cand = [p for p, pr in STATE["calls"].items()
                if not pr.get("LineIdentification") and pr.get("State") in ("dialing", "alerting", "active")]
        if len(cand) == 1:
            OURS["path"] = cand[0]
            return cand[0]
    return None


def refresh_calls():
    ag, _ = find_ag()
    if not ag:
        return
    STATE["ag"] = ag
    STATE["calls"] = {p: dict(props) for p, props in get_calls(ag).items()}


def hangup_mine(reason):
    """Retries for 30 s: a WirePlumber restart or HFP reconnect may be in flight."""
    end = time.monotonic() + 30
    while time.monotonic() < end:
        try:
            refresh_calls()
        except dbus.DBusException as e:
            log("HANGUP-ERR", f"refresh: {e.get_dbus_name()}")
        p = my_call()
        if not p:
            log("HANGUP", f"({reason}) no Call of ours present")
            return True
        for iface in (CALL_IF, "org.ofono.VoiceCall"):
            try:
                t = time.monotonic()
                tel(p, iface).Hangup()
                log("HANGUP", f"({reason}) {p} via {iface}.Hangup returned in {time.monotonic()-t:.3f}s")
                break
            except dbus.DBusException as e:
                log("HANGUP-ERR", f"{iface}: {e.get_dbus_name()} {e.get_dbus_message()}")
        if wait_for(lambda: my_call() is None, 5, "our Call removed after Hangup"):
            return True
        pump(1.0)
    return False


# ---------------------------------------------------------------- snapshot
def busctl_json(bus, *args):
    r = subprocess.run(["busctl", f"--{bus}", "--json=short", *args], capture_output=True, text=True)
    return json.loads(r.stdout) if r.returncode == 0 and r.stdout.strip() else {"error": r.stderr.strip()}


def snap_object(bus, service, path):
    r = busctl_json(bus, "call", service, path, "org.freedesktop.DBus.Introspectable", "Introspect")
    xml = r["data"][0] if "data" in r else str(r)
    props = {}
    # PipeWire quotes attributes with ', BlueZ with "
    for m in re.finditer(r"<interface name=[\"']([^\"']+)[\"']>(.*?)</interface>", xml, re.S):
        i, body = m.group(1), m.group(2)
        if i.startswith("org.freedesktop.DBus."):
            continue
        if "<property" in body:
            props[i] = busctl_json(bus, "call", service, path, PROPS_IF, "GetAll", "s", i)
    return {"bus": bus, "service": service, "path": path, "introspection_xml": xml, "properties": props}


def write_snapshot(out, extra_paths=()):
    ag = STATE["ag"]
    objs = [("user", TEL, TEL_ROOT), ("user", TEL, ag)]
    objs += [("user", TEL, p) for p in STATE["calls"]]
    objs += [("system", "org.bluez", "/org/bluez/hci0"), ("system", "org.bluez", DEV_PATH)]
    objs += [("system", "org.bluez", p) for p in a2dp_state()]
    objs += list(extra_paths)
    calls = {
        "GetManagedObjects(/org/pipewire/Telephony)": busctl_json("user", "call", TEL, TEL_ROOT, OM_IF, "GetManagedObjects"),
        f"GetManagedObjects({ag})": busctl_json("user", "call", TEL, ag, OM_IF, "GetManagedObjects"),
        "GetModems(/org/pipewire/Telephony)": busctl_json("user", "call", TEL, TEL_ROOT, "org.ofono.Manager", "GetModems"),
        f"GetCalls({ag})": busctl_json("user", "call", TEL, ag, "org.ofono.VoiceCallManager", "GetCalls"),
    }
    data = {"captured": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "phone": "Pixel 9a",
            "objects": [snap_object(*o) for o in objs], "method_results": calls}
    text = redact(json.dumps(data, indent=2))
    open(out, "w").write(text + "\n")
    log("SNAPSHOT", f"wrote {len(data['objects'])} objects to {out}")


# ---------------------------------------------------------------- the trial
def place_call_and_activate():
    ag, ifaces = find_ag()
    if not ag:
        sys.exit("No AudioGateway for the Phone; is hands-free connected?")
    STATE["ag"] = ag
    STATE["transport"] = dict(ifaces.get(TR_IF, {}))
    existing = get_calls(ag)
    if existing:
        sys.exit(f"Refusing: {len(existing)} Call(s) already exist; never touch real Calls.")
    log("PRE", f"ag={ag} transport={STATE['transport']}")
    media_report("before")

    log("ACTION", "AudioGateway1.Dial(<TEST_NUMBER>)")
    OURS["dialled"] = True
    tel(ag, AG_IF).Dial(NUMBER)
    if not wait_for(lambda: my_call() is not None, 15, "Call object for our Dial appeared"):
        log("CALLS", f"GetCalls={get_calls(ag)}")
        for p, props in get_calls(ag).items():
            if re.sub(r"\D", "", props.get("LineIdentification", "")).endswith(_DIGITS[-7:]):
                STATE["calls"][p] = props
        if not my_call():
            return False
    if not wait_for(lambda: STATE["calls"].get(my_call(), {}).get("State") == "active", 45,
                    "our Call is active (voicemail answered)"):
        return False
    pump(1.0)
    log("CHECK", f"before Activate: transport={STATE['transport']} (RejectSCO keeps audio on Phone?)")
    if "--trace" in sys.argv:  # only after the Call is active: ATD<number> never reaches the journal
        subprocess.run(["wpctl", "set-log-level", "spa.bluez5*:T,*:W"])
        log("TRACE", "WirePlumber bluez5 log level -> T")
    # PipeWire 1.6.9: Activate() only sends AT+BCC; sco_listen_event still rejects
    # the Phone's SCO while RejectSCO=true. So lift it first.
    if "--no-lift" not in sys.argv:
        tel(ag, PROPS_IF).Set(TR_IF, "RejectSCO", False, signature="ssv")
        log("ACTION", "Set RejectSCO=false")
    t = time.monotonic()
    for attempt in range(1, 4):
        ta = time.monotonic()
        try:
            tel(ag, TR_IF).Activate()
            log("ACTION", f"Activate() #{attempt} returned in {time.monotonic()-ta:.3f}s")
        except dbus.DBusException as e:
            log("ERR", f"Activate() #{attempt}: {e.get_dbus_name()} {e.get_dbus_message()}")
        ok = wait_for(lambda: STATE["transport"].get("State") == "active", 4,
                      f"transport active (audio on Desktop) after Activate #{attempt}")
        if ok:
            break
    log("TIMING", f"Activate -> transport active: {time.monotonic()-t:.3f}s")
    if "--no-lift" not in sys.argv:
        tel(ag, PROPS_IF).Set(TR_IF, "RejectSCO", True, signature="ssv")
        log("ACTION", "Set RejectSCO=true (re-arm for later SCO attempts)")
    media_report("audio on Desktop")
    return ok


def candidate_a():
    log("ACTION", "Device1.DisconnectProfile(HFP AG)")
    t_disc = time.monotonic()
    try:
        device().DisconnectProfile(HFP_AG_UUID)
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
            device().ConnectProfile(HFP_AG_UUID)
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
        for p, props in get_calls(STATE["ag"]).items():
            STATE["calls"][p] = props
        log("CALLS", f"GetCalls after reconnect={get_calls(STATE['ag'])}")
        readopt = my_call() is not None
    state = STATE["calls"].get(my_call(), {}).get("State")
    log("RESULT", f"Call survived={readopt} state={state} "
                  f"audio-left-Desktop<= {t_gone-t_disc:.3f}s control-lost={t_ctrl-t_disc:.3f}s")
    if STATE["ag"]:
        rs = tel(STATE["ag"], PROPS_IF).Get(TR_IF, "RejectSCO")
        log("CHECK", f"new AG RejectSCO={bool(rs)}; watching transport 6s")
        pump(6.0)
        log("CHECK", f"transport after 6s={STATE['transport']}")
        if "--round-trip" in sys.argv and readopt:
            ag = STATE["ag"]
            tel(ag, PROPS_IF).Set(TR_IF, "RejectSCO", False, signature="ssv")
            t = time.monotonic()
            tel(ag, TR_IF).Activate()
            log("ACTION", "round trip: Set RejectSCO=false + Activate() on the new AG")
            wait_for(lambda: STATE["transport"].get("State") == "active", 8, "round trip: audio back on Desktop")
            log("TIMING", f"round trip Activate -> active: {time.monotonic()-t:.3f}s")
            tel(ag, PROPS_IF).Set(TR_IF, "RejectSCO", True, signature="ssv")
    media_report("after reconnect")
    return readopt


def candidate_b():
    before = card_profile()
    log("ACTION", f"pactl set-card-profile {CARD} off (was {before})")
    t = time.monotonic()
    subprocess.run(["pactl", "set-card-profile", CARD, "off"], check=False)
    wait_for(lambda: STATE["transport"].get("State") != "active" or STATE["ag"] is None, 10,
             "transport no longer active / AG gone")
    media_report("profile off")
    pump(1.0)
    log("ACTION", f"pactl set-card-profile {CARD} {before}")
    subprocess.run(["pactl", "set-card-profile", CARD, before], check=False)
    wait_for(lambda: STATE["ag"] is not None, 20, "AudioGateway present")
    readopt = wait_for(lambda: my_call() is not None, 10, "our Call present")
    log("RESULT", f"Call survived={readopt} elapsed={time.monotonic()-t:.3f}s transport={STATE['transport']}")
    pump(4.0)
    media_report("after toggle")
    return readopt


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "pbap-snapshot":
        pbap_snapshot(sys.argv[2])
        return
    if cmd not in ("trial-a", "trial-b", "activate-only"):
        sys.exit(__doc__)
    snap = sys.argv[sys.argv.index("--snapshot") + 1] if "--snapshot" in sys.argv else None
    try:
        if place_call_and_activate():
            if snap:
                write_snapshot(snap)
            pump(2.0)
            if cmd != "activate-only":
                (candidate_a if cmd == "trial-a" else candidate_b)()
    finally:
        if "--trace" in sys.argv:
            subprocess.run(["wpctl", "set-log-level", "W"])
            log("TRACE", "WirePlumber log level -> W")
        if not hangup_mine("end of trial") and my_call():
            log("ALERT", "our Call may still be live; hang it up on the Phone")
        pump(1.0)
        media_report("end")
        _pactl.terminate()


def pbap_snapshot(out):
    client = dbus.Interface(SESSION.get_object("org.bluez.obex", "/org/bluez/obex"), "org.bluez.obex.Client1")
    session = client.CreateSession(PHONE_ADDR, {"Target": "pbap"})
    log("PBAP", f"session {session}")
    try:
        pb = dbus.Interface(SESSION.get_object("org.bluez.obex", session), "org.bluez.obex.PhonebookAccess1")
        snaps = {"after CreateSession": snap_object("user", "org.bluez.obex", session)}
        pb.Select("int", "pb")
        snaps["after Select(int, pb)"] = snap_object("user", "org.bluez.obex", session)
        size = int(pb.GetSize())
        log("PBAP", f"pb GetSize={size}")
        data = {"captured": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
                "objects": snaps, "GetSize(pb)": "redacted (count of the user's contacts)"}
        open(out, "w").write(redact(json.dumps(data, indent=2)) + "\n")
        log("SNAPSHOT", f"wrote {out}")
    finally:
        client.RemoveSession(session)
        log("PBAP", "session removed")


if __name__ == "__main__":
    main()
