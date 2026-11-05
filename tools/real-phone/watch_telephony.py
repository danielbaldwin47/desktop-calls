#!/usr/bin/python3
"""Print org.pipewire.Telephony's object signals as they arrive. Read-only.

Usage:
  /usr/bin/python3 tools/real-phone/watch_telephony.py [--seconds N]

Prints one timestamped line per signal from any object under
/org/pipewire/Telephony: ObjectManager.InterfacesAdded and
InterfacesRemoved with the object path and the interface names (and, for
InterfacesAdded, each interface's property names), the oFono ModemAdded,
ModemRemoved, CallAdded and CallRemoved with the object path, and every
change of owner of the org.pipewire.Telephony name. Property values are
never printed, so no phone number can reach the output; the Test Number
is redacted anyway.

It sends no method calls to the Phone's side and stops on Ctrl-C, SIGTERM,
or after N seconds (default 120).
"""

from __future__ import annotations

import argparse
import signal
import sys
from typing import Any

import dbus
import dbus.mainloop.glib
from gi.repository import GLib, GLibUnix

import real_phone as rp
from real_phone import log

OFONO_MANAGER_IF = "org.ofono.Manager"


def describe_interfaces(interfaces: Any) -> str:
    """`Iface{Prop, Prop}` for each interface, names only."""
    if isinstance(interfaces, dict):
        return ", ".join(
            f"{name}{{{', '.join(sorted(props))}}}" for name, props in sorted(interfaces.items())
        )
    return ", ".join(str(name) for name in interfaces)


def main() -> int:
    parser = argparse.ArgumentParser(prog="watch_telephony.py", description=__doc__.split("\n")[0])
    parser.add_argument("--seconds", type=float, default=120.0)
    args = parser.parse_args()
    rp.install_redaction(rp.read_test_number())

    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
    session = dbus.SessionBus()
    loop = GLib.MainLoop()

    def on_signal(*body: Any, **kw: Any) -> None:
        path, member, sender = kw["path"], kw["member"], kw["sender"]
        if not str(path).startswith(rp.TELEPHONY_ROOT):
            return
        if member in ("InterfacesAdded", "InterfacesRemoved"):
            detail = f"{body[0]} [{describe_interfaces(body[1])}]"
        else:
            detail = str(body[0])
        log(member, f"on {path} from {sender}: {detail}")

    for interface, members in (
        (rp.OBJECT_MANAGER_IF, ("InterfacesAdded", "InterfacesRemoved")),
        (OFONO_MANAGER_IF, ("ModemAdded", "ModemRemoved")),
        (rp.CALL_MANAGER_IF, ("CallAdded", "CallRemoved")),
    ):
        for member in members:
            session.add_signal_receiver(
                on_signal,
                signal_name=member,
                dbus_interface=interface,
                path_keyword="path",
                member_keyword="member",
                sender_keyword="sender",
            )

    def on_owner_changed(name: str, old: str, new: str) -> None:
        if name == rp.TELEPHONY:
            log("OWNER", f"{rp.TELEPHONY}: {old or '(none)'} -> {new or '(none)'}")

    session.add_signal_receiver(
        on_owner_changed,
        signal_name="NameOwnerChanged",
        dbus_interface="org.freedesktop.DBus",
        bus_name="org.freedesktop.DBus",
    )

    def stop(*_: Any) -> bool:
        loop.quit()
        return False

    for s in (signal.SIGINT, signal.SIGTERM):
        GLibUnix.signal_add(GLib.PRIORITY_DEFAULT, s, stop)
    GLib.timeout_add(int(args.seconds * 1000), stop)
    log("WATCH", f"signals under {rp.TELEPHONY_ROOT} for up to {args.seconds:g} s")
    loop.run()
    log("WATCH", "stopped")
    return 0


if __name__ == "__main__":
    sys.exit(main())
