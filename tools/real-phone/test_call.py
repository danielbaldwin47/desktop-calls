#!/usr/bin/python3
"""Place one real Call to the Test Number, run a hook during it, hang up.

Usage (through just):
  just test-call                          # dial, hold 2 s once active, hang up
  just test-call 'busctl --user ...'      # run the hook once the Call is active
  just test-call --hold 10                # hold 10 s instead of 2
  just test-call --max-seconds 120 HOOK   # let the hook run up to 120 s (default 60)
  just test-call --desktop-audio          # accept the Call's audio on the Desktop

The Test Number is read from $XDG_CONFIG_HOME/desktop-calls/test-number and
never printed: all output, the hook's included, is redacted.

The hook runs as `bash -c HOOK` in its own process group, with standard
input from /dev/null and these variables set:
  DESKTOP_CALLS_AUDIO_GATEWAY  the Phone's AudioGateway object path
  DESKTOP_CALLS_CALL           the Call's object path (it can change if
                               WirePlumber restarts; re-find it by State)
  DESKTOP_CALLS_PHONE_ADDRESS  the Phone's Bluetooth address
The Call is hung up whatever happens: when the hook exits or is killed, on
a D-Bus error, on Ctrl-C, SIGTERM or SIGHUP, and, through a separate guard
process, even if this process is killed outright.

The installed desktop-calls.service never runs beside the run (ADR-0006):
if it is running, it is stopped before dialling, so the hook has the Phone
to itself and may start a dev run of the app. It is started again after
hanging up, in all the cases above, the guard's included. A service that
wasn't running stays stopped.

With --desktop-audio it sets RejectSCO=false before dialling, as the app
does for a Call placed on the Desktop, logs every change of the
transport's State and Codec until the Call is active, and sets RejectSCO
back to true after hanging up (the guard does too).

Refuses to start if any Call already exists, and only ever hangs up the
Call to the Test Number.

Exit status: 0 the Call was placed, the hook succeeded, and GetCalls is
empty afterwards; 1 something failed but no Call remains; 2 a Call may
still be live on the Phone (go and hang it up); 3 no Call remains but the
installed service could not be started again (start it with
`systemctl --user start desktop-calls.service`); 130 interrupted, no Call
remains.
"""

from __future__ import annotations

import argparse
import contextlib
import os
import signal
import subprocess
import sys
import threading
import time

import dbus

import real_phone as rp
from real_phone import log

EXIT_OK, EXIT_FAILED, EXIT_CALL_REMAINS, EXIT_SERVICE_STOPPED, EXIT_INTERRUPTED = 0, 1, 2, 3, 130


class TestCallFailed(Exception):
    """The Call could not be placed or did not become active."""


# ------------------------------------------------------------------ guard
#
# A separate process, in its own session so Ctrl-C and a closed terminal do
# not reach it. It reads lines from a pipe: "service-stopped", "dialled",
# "call <path>", "hook <process group>", "done". If the pipe closes without
# "done", the parent died without finishing, so the guard stops the hook,
# hangs up the Call to the Test Number itself, and starts the installed
# service again if the parent had stopped it. `just run` uses it too.


class Guard:
    def __init__(self) -> None:
        self._proc = subprocess.Popen(
            [sys.executable, os.path.abspath(__file__), "--guard"],
            stdin=subprocess.PIPE,
            start_new_session=True,
            text=True,
        )

    def tell(self, line: str) -> None:
        try:
            assert self._proc.stdin is not None
            self._proc.stdin.write(line + "\n")
            self._proc.stdin.flush()
        except (BrokenPipeError, OSError):
            log("GUARD", "the guard process is gone")

    def done(self) -> None:
        self.tell("done")
        try:
            assert self._proc.stdin is not None
            self._proc.stdin.close()
        except OSError:
            pass
        try:
            self._proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self._proc.kill()


def guard_main() -> int:
    for s in (signal.SIGINT, signal.SIGHUP):
        signal.signal(s, signal.SIG_IGN)
    dialled, call_path, hook_group, desktop_audio = False, None, None, False
    service_stopped = False
    for line in sys.stdin:
        word, _, rest = line.strip().partition(" ")
        if word == "done":
            return EXIT_OK
        if word == "service-stopped":
            service_stopped = True
        elif word == "dialled":
            dialled = True
        elif word == "call":
            call_path = rest
        elif word == "hook":
            hook_group = int(rest)
        elif word == "desktop-audio":
            desktop_audio = True
    if hook_group is not None:
        with contextlib.suppress(ProcessLookupError):
            os.killpg(hook_group, signal.SIGKILL)
    outcome = EXIT_OK
    try:
        if dialled:
            outcome = guard_hang_up(call_path, desktop_audio)
        if service_stopped:
            log("GUARD", "the run stopped without starting the installed service again")
            if not rp.InstalledService().start():
                outcome = max(outcome, EXIT_SERVICE_STOPPED)
    except dbus.DBusException as e:
        log("GUARD", f"lost the session bus: {e.get_dbus_name()}")
        return EXIT_CALL_REMAINS
    return outcome


def guard_hang_up(call_path: str | None, desktop_audio: bool) -> int:
    number = rp.read_test_number()
    rp.install_redaction(number)
    log("GUARD", "test-call stopped without hanging up; the guard is hanging up")
    tel = rp.Telephony(number)
    tel.dialled = True
    tel.test_call_path = call_path
    tel.hang_up_test_call()
    if desktop_audio:
        reject_desktop_audio(tel)
    return EXIT_OK if rp.report_remaining(tel.remaining_calls()) else EXIT_CALL_REMAINS


# ------------------------------------------------------------------ hook


class Hook:
    def __init__(self, command: str, env: dict[str, str]) -> None:
        log("HOOK", f"starting: {command}")
        self.started = time.monotonic()
        self.proc = subprocess.Popen(
            ["bash", "-c", command],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            env={**os.environ, **env},
            text=True,
            errors="replace",
        )
        self._reader = threading.Thread(target=self._copy_output, daemon=True)
        self._reader.start()

    def _copy_output(self) -> None:
        assert self.proc.stdout is not None
        for line in self.proc.stdout:
            print(f"           hook | {line.rstrip()}", flush=True)

    def wait(self, max_seconds: float) -> int | None:
        """The hook's exit status, or None if it ran past `max_seconds`."""
        try:
            return self.proc.wait(timeout=max_seconds)
        except subprocess.TimeoutExpired:
            return None

    def stop(self) -> None:
        """Stop the hook and everything it started."""
        if self.proc.poll() is None:
            log("HOOK", "stopping the hook")
            for sig, grace in ((signal.SIGTERM, 2.0), (signal.SIGKILL, 2.0)):
                try:
                    os.killpg(self.proc.pid, sig)
                except ProcessLookupError:
                    break
                try:
                    self.proc.wait(timeout=grace)
                    break
                except subprocess.TimeoutExpired:
                    continue
        self._reader.join(timeout=1)


def describe_status(status: int) -> str:
    if status < 0:
        return f"killed by {signal.Signals(-status).name}"
    return f"exited with status {status}"


# ------------------------------------------------------------------ audio


def set_reject_sco(tel: rp.Telephony, gateway: str, value: bool) -> None:
    rp.set_property(
        tel.session, rp.TELEPHONY, gateway, rp.TRANSPORT_IF, "RejectSCO", dbus.Boolean(value)
    )
    log("AUDIO", f"Set RejectSCO={str(value).lower()}")


def reject_desktop_audio(tel: rp.Telephony) -> None:
    """Set RejectSCO back to true, so the next Call's audio stays on the Phone."""
    try:
        gateway = tel.audio_gateway()
        if gateway is None:
            log("AUDIO", "no AudioGateway to set RejectSCO=true on")
            return
        set_reject_sco(tel, gateway, True)
    except dbus.DBusException as e:
        log("AUDIO", f"could not set RejectSCO=true: {e.get_dbus_name()}")


class TransportLog:
    """Logs each change of the transport's State and Codec, read with Get."""

    def __init__(self, tel: rp.Telephony, gateway: str) -> None:
        self.tel, self.gateway, self.last = tel, gateway, None

    def poll(self) -> None:
        values = tuple(
            rp.call(self.tel.session, rp.TELEPHONY, self.gateway, rp.PROPERTIES_IF, "Get",
                    rp.TRANSPORT_IF, name, signature="ss")
            for name in ("State", "Codec", "RejectSCO")
        )
        if values != self.last:
            self.last = values
            log("AUDIO", f"transport State={values[0]} Codec={values[1]} RejectSCO={values[2]}")


# ------------------------------------------------------------------ main


def place_test_call(
    tel: rp.Telephony, gateway: str, guard: Guard, transport: TransportLog | None
) -> str:
    """Dial the Test Number and wait until the Call is active. Returns its path."""

    def appeared() -> bool:
        if transport is not None:
            transport.poll()
        return tel.find_test_call() is not None

    log("DIAL", "AudioGateway1.Dial(<TEST_NUMBER>)")
    guard.tell("dialled")
    tel.dial_test_number(gateway)
    log("DIAL", "Dial returned")
    if not tel.wait_for(appeared, 15, "the Call appeared"):
        raise TestCallFailed("no Call appeared within 15 s of dialling")
    assert tel.test_call_path is not None
    guard.tell(f"call {tel.test_call_path}")
    state = {"last": ""}

    def active() -> bool:
        if transport is not None:
            transport.poll()
        found = tel.find_test_call()
        current = found[1].get("State", "") if found else "gone"
        if current != state["last"]:
            state["last"] = current
            log("STATE", f"Call {current}")
        if current == "gone":
            raise TestCallFailed("the Call ended before it became active")
        return current == "active"

    if not tel.wait_for(active, 45, "the Call is active"):
        raise TestCallFailed("the Call did not become active within 45 s")
    return tel.test_call_path


def run(args: argparse.Namespace) -> int:
    number = rp.read_test_number()
    rp.install_redaction(number)
    rp.raise_on_interrupt()
    tel = rp.Telephony(number, args.phone_address)

    try:
        gateways = tel.audio_gateways()
        gateway = tel.audio_gateway()
        existing = tel.all_calls()
    except dbus.DBusException as e:
        log("ERROR", f"cannot reach {rp.TELEPHONY}: {e.get_dbus_name()}. Is WirePlumber running?")
        return EXIT_FAILED
    if gateway is None:
        if not gateways:
            log("ERROR", "no AudioGateway: the Phone's hands-free link is not connected.")
        else:
            log("ERROR", f"{len(gateways)} AudioGateways; choose one with --phone-address.")
        return EXIT_FAILED
    if existing:
        log("ERROR", f"refusing: {len(existing)} Call(s) already on the Phone. "
                     "test-call never touches real Calls; try again once they end.")
        return EXIT_FAILED
    log("PHONE", f"{tel.phone_address}, AudioGateway {gateway}, no Calls")

    service = rp.InstalledService(tel.session)
    guard = Guard()
    hook: Hook | None = None
    outcome = EXIT_FAILED
    service_stopped = False
    transport = TransportLog(tel, gateway) if args.desktop_audio else None
    try:
        if service.running():
            guard.tell("service-stopped")
            service_stopped = True
            service.stop()
        if transport is not None:
            guard.tell("desktop-audio")
            transport.poll()
            set_reject_sco(tel, gateway, False)
        call_path = place_test_call(tel, gateway, guard, transport)
        if transport is not None:
            transport.poll()
        if args.hook:
            hook = Hook(args.hook, {
                "DESKTOP_CALLS_AUDIO_GATEWAY": gateway,
                "DESKTOP_CALLS_CALL": call_path,
                "DESKTOP_CALLS_PHONE_ADDRESS": tel.phone_address or "",
            })
            guard.tell(f"hook {hook.proc.pid}")
            status = hook.wait(args.max_seconds)
            if status is None:
                log("HOOK", f"still running after {args.max_seconds:g} s; stopping it")
            else:
                log("HOOK", describe_status(status))
                outcome = EXIT_OK if status == 0 else EXIT_FAILED
        else:
            held_until = time.monotonic() + args.hold
            while time.monotonic() < held_until:
                if transport is not None:
                    transport.poll()
                time.sleep(0.1)
            outcome = EXIT_OK
    except rp.Interrupted as e:
        log("STOP", f"{e}: hanging up")
        outcome = EXIT_INTERRUPTED
    except TestCallFailed as e:
        log("ERROR", str(e))
    except dbus.DBusException as e:
        log("ERROR", f"{e.get_dbus_name()}: {e.get_dbus_message()}")
    except rp.ServiceError as e:
        log("ERROR", str(e))
    finally:
        with rp.interrupts_deferred():
            if hook is not None:
                hook.stop()
            tel.hang_up_test_call()
            if transport is not None:
                reject_desktop_audio(tel)
            no_call_remains = rp.report_remaining(tel.remaining_calls())
            service_started = not service_stopped or service.start()
            guard.done()
    if not no_call_remains:
        return EXIT_CALL_REMAINS
    if not service_started:
        return EXIT_SERVICE_STOPPED
    return outcome


def main() -> int:
    if sys.argv[1:] == ["--guard"]:
        return guard_main()
    parser = argparse.ArgumentParser(
        prog="just test-call",
        description="Place one real Call to the Test Number, run HOOK during it, hang up.",
    )
    parser.add_argument("hook", nargs="?", help="shell command to run once the Call is active")
    parser.add_argument("--hold", type=float, default=2.0,
                        help="seconds to stay in the Call when there is no hook (default 2)")
    parser.add_argument("--max-seconds", type=float, default=60.0,
                        help="stop the hook after this many seconds (default 60)")
    parser.add_argument("--desktop-audio", action="store_true",
                        help="set RejectSCO=false before dialling and log the transport")
    parser.add_argument("--phone-address", help="the Phone's Bluetooth address, if several")
    args = parser.parse_args()
    try:
        return run(args)
    except rp.Interrupted:
        # Arrived before anything was dialled: nothing to hang up.
        return EXIT_INTERRUPTED


if __name__ == "__main__":
    sys.exit(main())
