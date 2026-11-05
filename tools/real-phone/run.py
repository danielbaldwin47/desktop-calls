#!/usr/bin/python3
"""Run this checkout's app on the real Phone, in place of the installed service.

Usage (through just):
  just run                   # build, then run the app until Ctrl-C

  /usr/bin/python3 tools/real-phone/run.py BINARY

It runs `BINARY service`, a dev run of the app, until the app exits or you
press Ctrl-C. The app never runs beside the installed desktop-calls.service
(ADR-0006): if the service is running, it is stopped first, and it is
started when the run ends, whether or not it ran before: when the app
exits, on Ctrl-C, SIGTERM or SIGHUP, and, through test_call.py's guard
process, even if this process is killed outright.

It refuses to start without the Phone's hands-free link, or while the Phone
has a Call, so it never takes a Call away from the installed service. The dev run keeps its own runtime state
($XDG_RUNTIME_DIR/desktop-calls-dev), apart from the service's, and logs to
the journal like the service.

Agents: only the Test Number may be Called from the dev run, and no real
incoming Call may be answered or declined.

Exit status: the app's own; 1 refused or failed; 3 the installed service
could not be started again (start it with
`systemctl --user start desktop-calls.service`); 130 interrupted.
"""

from __future__ import annotations

import os
import signal
import subprocess
import sys
from pathlib import Path

import dbus

import real_phone as rp
from real_phone import log
from test_call import (
    EXIT_FAILED,
    EXIT_INTERRUPTED,
    EXIT_SERVICE_STOPPED,
    Guard,
    start_installed_service,
)

USAGE = "usage: just run (or /usr/bin/python3 tools/real-phone/run.py BINARY)"


def stop_app(app: subprocess.Popen[bytes]) -> None:
    """Stop the dev run, and anything it started, as systemd would."""
    if app.poll() is not None:
        return
    log("APP", "stopping the dev run")
    for sig, grace in ((signal.SIGTERM, 10.0), (signal.SIGKILL, 2.0)):
        try:
            os.killpg(app.pid, sig)
        except ProcessLookupError:
            return
        try:
            app.wait(timeout=grace)
            return
        except subprocess.TimeoutExpired:
            continue


def run(binary: Path) -> int:
    rp.raise_on_interrupt()
    tel = rp.Telephony(None)
    try:
        gateway = tel.audio_gateway()
        calls = tel.all_calls()
    except dbus.DBusException as e:
        log("ERROR", f"cannot reach {rp.TELEPHONY}: {e.get_dbus_name()}. Is WirePlumber running?")
        return EXIT_FAILED
    if gateway is None:
        log("ERROR", "refusing: no AudioGateway, so the Phone's hands-free link is not "
                     "connected, or there is more than one.")
        return EXIT_FAILED
    if calls:
        log("ERROR", f"refusing: {len(calls)} Call(s) on the Phone. Try again once they end.")
        return EXIT_FAILED

    service = rp.InstalledService(tel.session)
    guard = Guard()
    app: subprocess.Popen[bytes] | None = None
    outcome = EXIT_FAILED
    try:
        guard.tell("service")
        if service.running():
            service.stop()
        log("APP", f"starting a dev run: {binary} service (Ctrl-C to stop)")
        app = subprocess.Popen([str(binary), "service"], start_new_session=True)
        guard.tell(f"hook {app.pid}")
        outcome = app.wait()
        log("APP", f"the dev run exited with status {outcome}")
    except rp.Interrupted as e:
        log("STOP", str(e))
        outcome = EXIT_INTERRUPTED
    except (dbus.DBusException, rp.ServiceError, OSError) as e:
        log("ERROR", str(e))
    finally:
        with rp.interrupts_deferred():
            if app is not None:
                stop_app(app)
            service_started = start_installed_service(service)
            guard.done()
    return outcome if service_started else EXIT_SERVICE_STOPPED


def main() -> int:
    arguments = sys.argv[1:]
    if "-h" in arguments or "--help" in arguments:
        print(__doc__)
        return 0
    if len(arguments) != 1 or arguments[0].startswith("-"):
        print(USAGE, file=sys.stderr)
        return 2
    binary = Path(arguments[0])
    if not binary.is_file():
        print(f"run.py: no app at {binary}; build it first", file=sys.stderr)
        return EXIT_FAILED
    try:
        return run(binary)
    except rp.Interrupted:
        return EXIT_INTERRUPTED


if __name__ == "__main__":
    sys.exit(main())
