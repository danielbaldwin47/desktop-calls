#!/usr/bin/python3
"""Time one Handoff to the Desktop on the real Phone, as a `just test-call` hook.

Usage:
  just test-call --max-seconds 40 '/usr/bin/python3 tools/real-phone/handoff_timing.py'
  just test-call --max-seconds 40 '/usr/bin/python3 tools/real-phone/handoff_timing.py --settle 5'
      Wait 5 s, once the app shows the Call, before the Handoff.
  /usr/bin/python3 tools/real-phone/handoff_timing.py --monitor-only
      Check that the session bus lets this tool watch it, with no Call and
      without starting the app.

During a Call placed by `just test-call` (audio on the Phone, `RejectSCO`
true), it starts this checkout's `target/debug/desktop-calls service`,
waits until the app shows the Call with its audio on the Phone, runs
`desktop-calls audio here`, and watches for 4 s, past the 3 s limit. It
prints one line: when `AudioLocation` read "desktop", and when, from
running the CLI, each `RejectSCO` write, each `Activate` and each change
of the transport's `State` happened. Then it stops the app (ticket #70).

It watches the session bus as a monitor (`BecomeMonitor`), which only
reads. The app does every write itself. The one exception: if the Handoff
failed and the app left `RejectSCO` false, it is set back to true after the
app has stopped, so the next Call's audio stays on the Phone.

The app logs at its default level, so no phone number reaches the journal.
All output goes through `just test-call`, which redacts the Test Number.
"""

from __future__ import annotations

import argparse
import os
import signal
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

import dbus
import dbus.bus
import dbus.lowlevel
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

import real_phone as rp

APP = Path(__file__).resolve().parents[2] / "target" / "debug" / "desktop-calls"
APP_NAME = "io.github.danielbaldwin47.DesktopCalls"
APP_PATH = "/io/github/danielbaldwin47/DesktopCalls"
APP_IF = "io.github.danielbaldwin47.DesktopCalls1"
BUS = "org.freedesktop.DBus"
BUS_PATH = "/org/freedesktop/DBus"

# What the monitor sees: the app's writes and Activates to Telephony, and
# Telephony's property changes. Method calls are matched by interface and
# member only, because a monitor's rules can't name the destination's
# well-known name.
MONITOR_RULES = [
    f"type='method_call',interface='{rp.TRANSPORT_IF}',member='Activate'",
    f"type='method_call',interface='{rp.PROPERTIES_IF}',member='Set'",
    f"type='signal',sender='{rp.TELEPHONY}',interface='{rp.PROPERTIES_IF}',"
    "member='PropertiesChanged'",
]

# How long to watch after running the CLI: past the 3 s limit.
WATCH_SECONDS = 4.0


class Watch:
    """Timestamped events from a monitor connection on the session bus."""

    def __init__(self) -> None:
        self.events: list[tuple[float, str]] = []
        self.monitor = dbus.bus.BusConnection(dbus.bus.BUS_SESSION)
        # Before BecomeMonitor: a filter added after it sees nothing.
        self.monitor.add_message_filter(self._seen)
        rp.call(
            self.monitor,
            BUS,
            BUS_PATH,
            "org.freedesktop.DBus.Monitoring",
            "BecomeMonitor",
            dbus.Array(MONITOR_RULES, signature="s"),
            dbus.UInt32(0),
            signature="asu",
        )

    def _seen(self, _connection: Any, message: Any) -> None:
        now = time.monotonic()
        member = message.get_member()
        args = message.get_args_list()
        if isinstance(message, dbus.lowlevel.MethodCallMessage):
            if member == "Activate":
                self.events.append((now, "Activate"))
            elif (member == "Set" and len(args) == 3 and str(args[0]) == rp.TRANSPORT_IF
                  and str(args[1]) == "RejectSCO"):
                self.events.append((now, f"RejectSCO={str(bool(args[2])).lower()}"))
        elif isinstance(message, dbus.lowlevel.SignalMessage) and len(args) >= 2:
            if str(args[0]) == rp.TRANSPORT_IF:
                changed = rp.plain(args[1])
                if "State" in changed:
                    self.events.append((now, f"State={changed['State']}"))
                if "RejectSCO" in changed:
                    self.events.append((now, f"RejectSCO is {changed['RejectSCO']}"))
        # A monitor must let every message through untouched.
        return None

    def pump(self, seconds: float) -> None:
        """Handles bus traffic for `seconds`."""
        context = GLib.MainContext.default()
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            while context.iteration(False):
                pass
            time.sleep(0.005)


def app_property(bus: dbus.Bus, name: str) -> Any:
    """One of the app's properties; None while no copy of the app owns its
    name, which is asked first so the installed service is never started
    through D-Bus activation."""
    try:
        if not rp.app_on_bus(bus):
            return None
        return rp.call(bus, APP_NAME, APP_PATH, rp.PROPERTIES_IF, "Get", APP_IF, name,
                       signature="ss", timeout=1.0)
    except dbus.DBusException:
        return None


def transport_property(bus: dbus.Bus, audio_gateway: str, name: str) -> Any:
    return rp.call(bus, rp.TELEPHONY, audio_gateway, rp.PROPERTIES_IF, "Get",
                   rp.TRANSPORT_IF, name, signature="ss")


def wait_until(watch: Watch, seconds: float, done: Any) -> float | None:
    """Pumps the bus every 20 ms until `done()`; the time it held, or None."""
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if done():
            return time.monotonic()
        watch.pump(0.02)
    return None


def stop(app: subprocess.Popen[bytes]) -> None:
    if app.poll() is not None:
        return
    app.send_signal(signal.SIGTERM)
    try:
        app.wait(timeout=5)
    except subprocess.TimeoutExpired:
        app.kill()
        app.wait()


def time_handoff(audio_gateway: str, settle: float) -> int:
    bus = dbus.SessionBus()
    watch = Watch()
    if rp.app_on_bus(bus):
        print("another desktop-calls already owns the bus name; not starting one")
        return 1
    app = subprocess.Popen([str(APP), "service"], stdin=subprocess.DEVNULL,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                           start_new_session=True)
    try:
        if wait_until(watch, 15, lambda: app_property(bus, "AudioLocation") == "phone") is None:
            print(f"the app never showed the Call with its audio on the Phone "
                  f"(AudioLocation {app_property(bus, 'AudioLocation')!r})")
            return 1
        watch.pump(settle)
        watch.events.clear()
        asked = time.monotonic()
        cli = subprocess.run([str(APP), "audio", "here"], capture_output=True, text=True,
                             timeout=10)
        here = wait_until(watch, WATCH_SECONDS,
                          lambda: app_property(bus, "AudioLocation") == "desktop")
        watch.pump(max(0.0, asked + WATCH_SECONDS - time.monotonic()))
        reject_sco = transport_property(bus, audio_gateway, "RejectSCO")
        state = transport_property(bus, audio_gateway, "State")
        codec = transport_property(bus, audio_gateway, "Codec")
    finally:
        stop(app)

    def ms(at: float) -> int:
        return round((at - asked) * 1000)

    events = ", ".join(f"{what} {ms(at)}" for at, what in watch.events)
    activates = sum(1 for _, what in watch.events if what == "Activate")
    result = f"desktop after {ms(here)} ms" if here is not None else "audio never came"
    print(f"RESULT {result}; Activates {activates}; CLI exit {cli.returncode}; "
          f"ms from CLI: {events}; at end RejectSCO {reject_sco}, State {state}, Codec {codec}")
    if cli.returncode != 0:
        print(f"CLI said: {cli.stderr.strip()}")
    if not reject_sco:
        rp.set_property(bus, rp.TELEPHONY, audio_gateway, rp.TRANSPORT_IF, "RejectSCO",
                        dbus.Boolean(True))
        print("set RejectSCO back to true, as the app left it false")
    return 0 if here is not None else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--monitor-only", action="store_true",
                        help="check the monitor works, with no Call and no app")
    parser.add_argument("--settle", type=float, default=0.0,
                        help="seconds to wait, once the app shows the Call, before the Handoff")
    args = parser.parse_args()
    DBusGMainLoop(set_as_default=True)
    if args.monitor_only:
        watch = Watch()
        watch.pump(1.0)
        print(f"monitor works; saw {len(watch.events)} matching messages in 1 s")
        return 0
    audio_gateway = os.environ.get("DESKTOP_CALLS_AUDIO_GATEWAY")
    if not audio_gateway:
        print("run this as a `just test-call` hook", file=sys.stderr)
        return 2
    return time_handoff(audio_gateway, args.settle)


if __name__ == "__main__":
    sys.exit(main())
