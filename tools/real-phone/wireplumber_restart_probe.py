#!/usr/bin/env python3
"""Restart WirePlumber once during a test Call, then wait for the Call to return.

Run only as the hook of `just test-call`, which hangs the Call up afterwards:
  just test-call --max-seconds 90 'python3 tools/real-phone/wireplumber_restart_probe.py'

The user allows one WirePlumber restart during a short test Call, and only
while no other audio stream is active (docs/wiki/user-preferences.md, Q30).
So before restarting, this probe checks, and skips the restart (exit 3) if:
  * any Call other than the one to the Test Number exists: the user may be
    on a real Call, and every real-Phone check must stop;
  * `pactl` lists any playback stream (sink input) or recording stream
    (source output). The test Call keeps its audio on the Phone
    (RejectSCO=true), so it has none of its own.

Then it runs `systemctl --user restart wireplumber` once and logs how long
org.pipewire.Telephony, the Phone's AudioGateway and the Call to the Test
Number take to come back. Exit 0 once the Call is back, 1 if it did not
return within 30 s; either way `just test-call` then hangs up.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time

import dbus

import real_phone as rp
from real_phone import log

EXIT_BACK, EXIT_NOT_BACK, EXIT_SKIPPED = 0, 1, 3


def audio_streams() -> list[str]:
    """Every playback and recording stream PipeWire's Pulse server lists."""
    streams = []
    for kind in ("sink-inputs", "source-outputs"):
        listed = subprocess.run(
            ["pactl", "-f", "json", "list", kind],
            capture_output=True,
            text=True,
            timeout=10,
            check=True,
        )
        for stream in json.loads(listed.stdout or "[]"):
            props = stream.get("properties", {})
            name = props.get("application.name") or props.get("media.name") or "?"
            streams.append(f"{kind} #{stream.get('index')}: {name}")
    return streams


def telephony_owner(session: dbus.Bus) -> str:
    try:
        return str(
            rp.call(
                session,
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetNameOwner",
                rp.TELEPHONY,
                signature="s",
            )
        )
    except dbus.DBusException:
        return ""


def main() -> int:
    number = rp.read_test_number()
    rp.install_redaction(number)
    tel = rp.Telephony(number, os.environ.get("DESKTOP_CALLS_PHONE_ADDRESS") or None)
    tel.dialled = True
    tel.test_call_path = os.environ.get("DESKTOP_CALLS_CALL")

    calls = tel.all_calls()
    others = [path for path, props in calls.items() if not tel.is_test_call(path, props)]
    if others or len(calls) != 1:
        log("SKIP", f"{len(calls)} Call(s) on the Phone, {len(others)} not ours: "
                    "the user may be on a real Call. Stop all real-Phone work.")
        return EXIT_SKIPPED
    streams = audio_streams()
    if streams:
        log("SKIP", f"other audio is active, so WirePlumber is not restarted: {'; '.join(streams)}")
        return EXIT_SKIPPED
    log("CHECK", "only the test Call exists, and no playback or recording stream is active")

    old_owner = telephony_owner(tel.session)
    old_call = tel.test_call_path
    log("RESTART", f"systemctl --user restart wireplumber (Telephony owner {old_owner})")
    started = time.monotonic()
    subprocess.run(["systemctl", "--user", "restart", "wireplumber"], timeout=30, check=True)
    log("RESTART", f"systemctl returned after {time.monotonic() - started:.3f} s")

    def since() -> str:
        return f"{time.monotonic() - started:.3f} s after the restart began"

    if tel.wait_for(lambda: telephony_owner(tel.session) not in ("", old_owner), 30,
                    "org.pipewire.Telephony has a new owner", quiet=True):
        log("BACK", f"org.pipewire.Telephony owned by {telephony_owner(tel.session)}, {since()}")
    if tel.wait_for(lambda: tel.audio_gateway() is not None, 30, "AudioGateway", quiet=True):
        log("BACK", f"AudioGateway {tel.audio_gateway()}, {since()}")
    tel.test_call_path = None
    if not tel.wait_for(lambda: tel.find_test_call() is not None, 30, "Call", quiet=True):
        log("NOT BACK", f"the Call to the Test Number did not reappear within 30 s ({since()})")
        return EXIT_NOT_BACK
    path, props = tel.find_test_call() or ("?", {})
    log("BACK", f"Call {path} State={props.get('State')} (was {old_call}), {since()}")
    others = [p for p, c in tel.all_calls().items() if p != path]
    if others:
        log("ALERT", f"{len(others)} other Call(s) appeared: stop all real-Phone work")
    return EXIT_BACK


if __name__ == "__main__":
    sys.exit(main())
