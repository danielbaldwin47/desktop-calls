#!/usr/bin/python3
"""Check the Handoff to the Phone (ADR-0005's bounce) on the real Phone.

Usage:
  /usr/bin/python3 tools/real-phone/handoff_to_phone.py --bounce-only
      With no Call on the Phone: drop the hands-free profile and connect
      it again once, exactly as the app does, and time it.
  just test-call --max-seconds 90 '/usr/bin/python3 tools/real-phone/handoff_to_phone.py'
      During a Call placed by `just test-call`: start this checkout's
      `target/debug/desktop-calls service`, then move the audio here, to the
      Phone, here and to the Phone again with `desktop-calls audio`, and
      time each step (scenario 12, ticket #14).

Only `Device1.DisconnectProfile` and `Device1.ConnectProfile`, with the
hands-free profile's UUID, are ever sent to BlueZ, and only by
`--bounce-only`; during a Call the app sends them itself. Nothing here
calls `Device1.Disconnect`, `Adapter1.RemoveDevice` or anything to do
with pairing. Both signatures are checked against BlueZ's introspection
before anything is sent.

If hands-free is not back within 30 s, or `ConnectProfile` fails, it says
so and stops at once: no retry. After each step to the Phone it checks
the Call is still active with the same Call id and timer, `RejectSCO` is
true and the Phone is still paired and connected.

No phone number is printed: Calls are shown by id and state only, and
everything goes through the Test Number's redaction.
"""

from __future__ import annotations

import argparse
import os
import signal
import subprocess
import sys
import time
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Any

import dbus

import real_phone as rp

APP = Path(__file__).resolve().parents[2] / "target" / "debug" / "desktop-calls"
APP_NAME = "io.github.danielbaldwin47.DesktopCalls"
APP_PATH = "/io/github/danielbaldwin47/DesktopCalls"
APP_IF = "io.github.danielbaldwin47.DesktopCalls1"
DEVICE_IF = "org.bluez.Device1"
HANDS_FREE_PROFILE = "0000111f-0000-1000-8000-00805f9b34fb"

# ADR-0005: hands-free must be back well within this, or something is wrong.
BACK_WITHIN = 30.0
# How long each Handoff may take before the step counts as failed.
STEP_WITHIN = 8.0
# How long to stay with the audio in each place before the next step.
SETTLE = 3.0


class Stop(Exception):
    """Something the coordinator must hear about at once; no retry."""


def device_path(system: dbus.Bus, address: str) -> str:
    objects = rp.call(system, rp.BLUEZ, "/", rp.OBJECT_MANAGER_IF, "GetManagedObjects",
                      signature="")
    for path, interfaces in objects.items():
        if interfaces.get(DEVICE_IF, {}).get("Address") == address:
            return path
    raise Stop(f"BlueZ has no Device1 for the Phone {address}")


def check_signatures(system: dbus.Bus, path: str) -> None:
    """Both profile methods take one string, by BlueZ's own introspection."""
    xml = rp.call(system, rp.BLUEZ, path, rp.INTROSPECTABLE_IF, "Introspect", signature="")
    for iface in ET.fromstring(xml).findall("interface"):
        if iface.get("name") != DEVICE_IF:
            continue
        found = {
            method.get("name"): "".join(arg.get("type", "") for arg in method.findall("arg")
                                        if arg.get("direction", "in") == "in")
            for method in iface.findall("method")
        }
        for member in ("DisconnectProfile", "ConnectProfile"):
            if found.get(member) != "s":
                raise Stop(f"Device1.{member} takes {found.get(member)!r}, not 's'")
        return
    raise Stop(f"{path} does not introspect {DEVICE_IF}")


def device(system: dbus.Bus, path: str, name: str) -> Any:
    return rp.call(system, rp.BLUEZ, path, rp.PROPERTIES_IF, "Get", DEVICE_IF, name,
                   signature="ss")


def reject_sco(tel: rp.Telephony, gateway: str) -> Any:
    return rp.call(tel.session, rp.TELEPHONY, gateway, rp.PROPERTIES_IF, "Get",
                   rp.TRANSPORT_IF, "RejectSCO", signature="ss")


def wait(seconds: float, done: Any) -> float | None:
    """Polls every 20 ms until `done()`; when it held, or None."""
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        try:
            if done():
                return time.monotonic()
        except dbus.DBusException:
            pass
        time.sleep(0.02)
    return None


def still_paired(system: dbus.Bus, path: str) -> str:
    paired, connected = device(system, path, "Paired"), device(system, path, "Connected")
    if not paired:
        raise Stop("the Phone is no longer paired")
    return f"Paired {paired}, Connected {connected}"


def bounce_only() -> int:
    tel = rp.Telephony(rp.read_test_number())
    system = dbus.SystemBus()
    gateway = tel.audio_gateway()
    if gateway is None:
        print("no single AudioGateway: is the Phone connected?")
        return 1
    if tel.all_calls():
        print("refusing: there is a Call on the Phone; --bounce-only needs none")
        return 1
    path = device_path(system, tel.phone_address or "")
    check_signatures(system, path)
    print(f"before: {still_paired(system, path)}, RejectSCO {reject_sco(tel, gateway)}")

    started = time.monotonic()
    rp.call(system, rp.BLUEZ, path, DEVICE_IF, "DisconnectProfile", HANDS_FREE_PROFILE,
            signature="s", timeout=BACK_WITHIN)
    dropped = time.monotonic()
    gone = wait(5.0, lambda: tel.audio_gateway() is None)
    try:
        rp.call(system, rp.BLUEZ, path, DEVICE_IF, "ConnectProfile", HANDS_FREE_PROFILE,
                signature="s", timeout=BACK_WITHIN)
    except dbus.DBusException as e:
        raise Stop(f"ConnectProfile failed: {e.get_dbus_name()}: {e.get_dbus_message()}") from e
    reconnected = time.monotonic()
    back = wait(BACK_WITHIN, lambda: tel.audio_gateway() is not None)
    if back is None:
        raise Stop(f"hands-free is not back {BACK_WITHIN:g} s after ConnectProfile")
    new_gateway = tel.audio_gateway() or ""

    def ms(at: float | None) -> str:
        return "never" if at is None else f"{round((at - started) * 1000)} ms"

    print(f"RESULT DisconnectProfile replied {ms(dropped)}, AudioGateway gone {ms(gone)}, "
          f"ConnectProfile replied {ms(reconnected)}, AudioGateway back {ms(back)} "
          f"({'same' if new_gateway == gateway else 'new'} path)")
    time.sleep(1.0)
    print(f"after: {still_paired(system, path)}, RejectSCO {reject_sco(tel, new_gateway)}")
    return 0


# ------------------------------------------------------------------ during a Call


def app_property(bus: dbus.Bus, name: str) -> Any:
    """One of the app's properties; None while no copy of the app is on the
    bus. Sent to the copy's unique name, so it never starts the installed
    service through D-Bus activation."""
    try:
        owner = rp.app_owner(bus)
        if owner is None:
            return None
        return rp.call(bus, owner, APP_PATH, rp.PROPERTIES_IF, "Get", APP_IF, name,
                       signature="ss", timeout=1.0)
    except dbus.DBusException:
        return None


def app_calls(bus: dbus.Bus) -> list[tuple[int, str, int]]:
    """The app's Calls as (id, state, active since): never the number."""
    return [(call[0], call[2], call[5]) for call in app_property(bus, "Calls") or []]


def stop_app(app: subprocess.Popen[bytes]) -> None:
    if app.poll() is not None:
        return
    app.send_signal(signal.SIGTERM)
    try:
        app.wait(timeout=5)
    except subprocess.TimeoutExpired:
        app.kill()
        app.wait()


def phone_calls(tel: rp.Telephony) -> str:
    gateway = tel.audio_gateway()
    if gateway is None:
        return "AudioGateway absent"
    return ", ".join(str(props.get("State")) for props in tel.calls(gateway).values()) or "none"


def step(bus: dbus.Bus, tel: rp.Telephony, system: dbus.Bus, device_at: str,
         location: str, before: list[tuple[int, str, int]]) -> str:
    """One Handoff by the CLI; a line on how it went."""
    word = {"desktop": "here", "phone": "phone"}[location]
    asked = time.monotonic()
    cli = subprocess.run([str(APP), "audio", word], capture_output=True, text=True, timeout=10)
    if cli.returncode != 0:
        raise Stop(f"`desktop-calls audio {word}` failed: {cli.stderr.strip()}")
    gone_at: float | None = None

    def arrived() -> bool:
        nonlocal gone_at
        if gone_at is None and tel.audio_gateway() is None:
            gone_at = time.monotonic()
        return app_property(bus, "AudioLocation") == location

    at = wait(STEP_WITHIN, arrived)
    if at is None:
        if tel.audio_gateway() is None and wait(BACK_WITHIN, lambda: tel.audio_gateway()
                                                is not None) is None:
            raise Stop(f"hands-free is not back {BACK_WITHIN:g} s after the Handoff")
        raise Stop(f"the audio did not reach the {location} within {STEP_WITHIN:g} s "
                   f"(AudioLocation {app_property(bus, 'AudioLocation')!r}, "
                   f"Surfaces notice in the app log)")
    line = f"{word} after {round((at - asked) * 1000)} ms"
    if location == "phone":
        back = wait(BACK_WITHIN, lambda: tel.audio_gateway() is not None and
                    phone_calls(tel) == "active")
        if back is None:
            raise Stop(f"the Call is not back {BACK_WITHIN:g} s after the Handoff")
        gateway = tel.audio_gateway() or ""
        line += (f" (AudioGateway gone at "
                 f"{'never' if gone_at is None else round((gone_at - asked) * 1000)} ms, "
                 f"Call back at {round((back - asked) * 1000)} ms); "
                 f"RejectSCO {reject_sco(tel, gateway)}, {still_paired(system, device_at)}")
    after = app_calls(bus)
    if after != before:
        raise Stop(f"the app's Call changed from {before} to {after}")
    return line


def during_call() -> int:
    tel = rp.Telephony(rp.read_test_number(), os.environ.get("DESKTOP_CALLS_PHONE_ADDRESS"))
    bus = dbus.SessionBus()
    system = dbus.SystemBus()
    if rp.app_on_bus(bus):
        print("another desktop-calls already owns the bus name; not starting one")
        return 1
    path = device_path(system, tel.phone_address or "")
    check_signatures(system, path)
    app = subprocess.Popen([str(APP), "service"], stdin=subprocess.DEVNULL,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                           start_new_session=True)
    try:
        if wait(15, lambda: app_property(bus, "AudioLocation") == "phone") is None:
            print("the app never showed the Call with its audio on the Phone")
            return 1
        before = app_calls(bus)
        print(f"the app shows Calls {before}")
        for location in ("desktop", "phone", "desktop", "phone"):
            time.sleep(SETTLE)
            print(f"STEP {step(bus, tel, system, path, location, before)}")
        time.sleep(SETTLE)
        print(f"RESULT here, phone, here, phone in one Call; the app shows Calls "
              f"{app_calls(bus)}, the Phone's Calls {phone_calls(tel)}")
        return 0
    except Stop as e:
        print(f"ALERT {e}")
        return 1
    finally:
        stop_app(app)
        gateway = tel.audio_gateway()
        if gateway is not None and not reject_sco(tel, gateway):
            rp.set_property(tel.session, rp.TELEPHONY, gateway, rp.TRANSPORT_IF, "RejectSCO",
                            dbus.Boolean(True))
            print("set RejectSCO back to true, as the app left it false")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bounce-only", action="store_true",
                        help="bounce hands-free once with no Call on the Phone")
    args = parser.parse_args()
    rp.install_redaction(rp.read_test_number())
    if args.bounce_only:
        try:
            return bounce_only()
        except Stop as e:
            print(f"ALERT {e}")
            return 1
    if not os.environ.get("DESKTOP_CALLS_AUDIO_GATEWAY"):
        print("run this as a `just test-call` hook, or with --bounce-only", file=sys.stderr)
        return 2
    return during_call()


if __name__ == "__main__":
    sys.exit(main())
