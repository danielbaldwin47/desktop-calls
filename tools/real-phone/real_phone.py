"""Shared, safe access to the real Phone over D-Bus, for agents' tools.

Everything an agent does to the real Phone goes through this module, so
that the rules learnt on ticket #3 hold everywhere:

* Every D-Bus method call carries an explicit signature (`call`). Nothing
  is guessed: on 2026-11-04 a guessed `ssb` instead of `ssv` crashed
  WirePlumber mid-Call.
* Properties are written only through `set_property`, which takes a typed
  value, checks it against the property's introspected type and access,
  and always sends signature `ssv`.
* The Test Number is read from outside the repo and never printed: once
  `install_redaction` has run, everything written to standard output or
  standard error, tracebacks included, has it replaced by <TEST_NUMBER>.
* Hanging up retries for 30 s and re-finds the Call each time, because a
  WirePlumber restart or a hands-free reconnect re-creates the objects.
  "No Call left" counts only while the Phone's AudioGateway is present.
* Only the Call to the Test Number is ever hung up. Other Calls on the
  Phone are reported, never touched.
* The app never runs beside the user's installed `desktop-calls.service`
  (ADR-0006): `InstalledService` stops it for a Real-Phone run and starts
  it when the run ends.
* Calls to the app go to the unique name `app_owner` returns, never to its
  well-known name: a call to an unowned well-known name would start the
  installed service through D-Bus activation.

This is a tool for agents, not part of the product. The names of the
external APIs (AudioGateway, Call1, Device1) are used as they are.
"""

from __future__ import annotations

import contextlib
import io
import os
import re
import signal
import sys
import time
import xml.etree.ElementTree as ET
from collections.abc import Callable, Iterator
from pathlib import Path
from typing import Any

import dbus

TELEPHONY = "org.pipewire.Telephony"
TELEPHONY_ROOT = "/org/pipewire/Telephony"
AUDIO_GATEWAY_IF = TELEPHONY + ".AudioGateway1"
TRANSPORT_IF = TELEPHONY + ".AudioGatewayTransport1"
CALL_IF = TELEPHONY + ".Call1"
OFONO_CALL_IF = "org.ofono.VoiceCall"
CALL_MANAGER_IF = "org.ofono.VoiceCallManager"
OBJECT_MANAGER_IF = "org.freedesktop.DBus.ObjectManager"
PROPERTIES_IF = "org.freedesktop.DBus.Properties"
INTROSPECTABLE_IF = "org.freedesktop.DBus.Introspectable"
BLUEZ = "org.bluez"
DEVICE1_IF = "org.bluez.Device1"
SYSTEMD = "org.freedesktop.systemd1"
SYSTEMD_PATH = "/org/freedesktop/systemd1"
SYSTEMD_MANAGER_IF = SYSTEMD + ".Manager"
SYSTEMD_UNIT_IF = SYSTEMD + ".Unit"
BUS = "org.freedesktop.DBus"
BUS_PATH = "/org/freedesktop/DBus"

# The unit `just install` installs, and the bus name the app claims.
INSTALLED_UNIT = "desktop-calls.service"
APP_BUS_NAME = "io.github.danielbaldwin47.DesktopCalls"

# States in which a Call may be the one we dialled. "incoming" and "waiting"
# are someone calling the Phone: never ours, never touched.
OUTGOING_STATES = ("dialing", "alerting", "active", "held")

# Seconds to wait for a D-Bus reply before giving up on that attempt.
DBUS_TIMEOUT = 5.0

T0 = time.monotonic()


# ------------------------------------------------------------------ output


def log(kind: str, detail: str = "") -> None:
    """One timestamped line on standard output (redacted once installed)."""
    print(f"+{time.monotonic() - T0:8.3f}s  {kind:<10} {detail}", flush=True)


class Redactor:
    """Replaces every spelling of the Test Number with <TEST_NUMBER>."""

    def __init__(self, test_number: str) -> None:
        digits = re.sub(r"\D", "", test_number)
        if len(digits) < 3:
            raise ValueError("the Test Number has fewer than 3 digits")
        self._digits = digits
        # The last 7 digits (or all, if shorter), with any separators between
        # them, preceded by any country or area prefix.
        tail = digits[-7:]
        sep = r"[\s().-]*"
        self._pattern = re.compile(
            r"(?:\+?\d[\d\s().-]*?)?" + sep.join(re.escape(d) for d in tail)
        )

    def __call__(self, text: str) -> str:
        return self._pattern.sub("<TEST_NUMBER>", text)

    def matches(self, line_identification: str) -> bool:
        """True if a Call's LineIdentification is the Test Number."""
        digits = re.sub(r"\D", "", line_identification)
        return bool(digits) and digits.endswith(self._digits[-7:])


class _RedactingStream(io.TextIOBase):
    """Line-buffers writes and redacts each complete line."""

    def __init__(self, inner: Any, redact: Redactor) -> None:
        self._inner = inner
        self._redact = redact
        self._pending = ""

    def write(self, text: str) -> int:
        self._pending += text
        if "\n" in self._pending:
            done, self._pending = self._pending.rsplit("\n", 1)
            self._inner.write(self._redact(done + "\n"))
            self._inner.flush()
        return len(text)

    def flush(self) -> None:
        if self._pending:
            self._inner.write(self._redact(self._pending))
            self._pending = ""
        self._inner.flush()

    def fileno(self) -> int:
        return self._inner.fileno()

    def isatty(self) -> bool:
        return self._inner.isatty()


def read_test_number() -> str:
    """The Test Number, from $XDG_CONFIG_HOME/desktop-calls/test-number."""
    config = os.environ.get("XDG_CONFIG_HOME") or str(Path.home() / ".config")
    path = Path(config) / "desktop-calls" / "test-number"
    try:
        number = path.read_text().strip()
    except OSError as e:
        sys.exit(f"Cannot read the Test Number from {path}: {e.strerror}.")
    if len(re.sub(r"\D", "", number)) < 3:
        sys.exit(f"{path} does not hold a phone number.")
    return number


def install_redaction(test_number: str) -> Redactor:
    """Redact the Test Number from all further standard output and error."""
    redact = Redactor(test_number)
    sys.stdout = _RedactingStream(sys.__stdout__, redact)
    sys.stderr = _RedactingStream(sys.__stderr__, redact)
    return redact


# ------------------------------------------------------------------ signals


class Interrupted(BaseException):
    """Ctrl-C, SIGTERM or SIGHUP arrived. A BaseException, so that ordinary
    error handling never swallows it."""

    def __init__(self, signum: int) -> None:
        super().__init__(signal.Signals(signum).name)
        self.signum = signum


_HANDLED = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)


def _raise_interrupted(signum: int, _frame: object) -> None:
    raise Interrupted(signum)


def _while_hanging_up(signum: int, _frame: object) -> None:
    log("WAIT", f"{signal.Signals(signum).name} ignored: hanging up the Call first")


def raise_on_interrupt() -> None:
    """Turn Ctrl-C, SIGTERM and SIGHUP into `Interrupted`, even when the
    shell started us with SIGINT ignored (a background job)."""
    for s in _HANDLED:
        signal.signal(s, _raise_interrupted)


@contextlib.contextmanager
def interrupts_deferred() -> Iterator[None]:
    """While hanging up, a second Ctrl-C must not abandon the hang-up."""
    for s in _HANDLED:
        signal.signal(s, _while_hanging_up)
    try:
        yield
    finally:
        raise_on_interrupt()


# ------------------------------------------------------------------ D-Bus


def plain(v: Any) -> Any:
    """A dbus-python value as plain Python, for logging and comparison."""
    if isinstance(v, dict):
        return {str(k): plain(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [plain(x) for x in v]
    if isinstance(v, dbus.Boolean):
        return bool(v)
    if isinstance(v, int):
        return int(v)
    if isinstance(v, float):
        return float(v)
    if isinstance(v, str):
        return str(v)
    return v


def call(
    bus: dbus.Bus,
    service: str,
    path: str,
    interface: str,
    member: str,
    *args: Any,
    signature: str,
    timeout: float = DBUS_TIMEOUT,
) -> Any:
    """One D-Bus method call with an explicit input signature, never guessed."""
    return plain(
        bus.call_blocking(service, path, interface, member, signature, args, timeout=timeout)
    )


_BASIC_SIGNATURES: dict[type, str] = {
    dbus.Boolean: "b",
    dbus.Byte: "y",
    dbus.Int16: "n",
    dbus.UInt16: "q",
    dbus.Int32: "i",
    dbus.UInt32: "u",
    dbus.Int64: "x",
    dbus.UInt64: "t",
    dbus.Double: "d",
    dbus.String: "s",
    dbus.ObjectPath: "o",
}


def introspected_property(
    bus: dbus.Bus, service: str, path: str, interface: str, name: str
) -> tuple[str, str] | None:
    """(type, access) of a property, from the object's own introspection."""
    xml = call(bus, service, path, INTROSPECTABLE_IF, "Introspect", signature="")
    root = ET.fromstring(xml)
    for iface in root.findall("interface"):
        if iface.get("name") != interface:
            continue
        for prop in iface.findall("property"):
            if prop.get("name") == name:
                return prop.get("type", ""), prop.get("access", "")
    return None


def set_property(
    bus: dbus.Bus, service: str, path: str, interface: str, name: str, value: Any
) -> None:
    """The one way tools write a D-Bus property.

    `value` must be a typed dbus-python value, e.g. `dbus.Boolean(False)`.
    Its type is checked against the property's introspected type before
    anything is sent, and the call always carries signature `ssv`.
    """
    value_type = _BASIC_SIGNATURES.get(type(value))
    if value_type is None:
        raise TypeError(f"set_property needs a typed dbus value, not {type(value).__name__}")
    declared = introspected_property(bus, service, path, interface, name)
    if declared is None:
        raise TypeError(f"{path} does not introspect {interface}.{name}")
    declared_type, access = declared
    if declared_type != value_type:
        raise TypeError(
            f"{interface}.{name} has type {declared_type!r}, but the value has {value_type!r}"
        )
    if "write" not in access:
        raise TypeError(f"{interface}.{name} is {access}, not writable")
    props = dbus.Interface(bus.get_object(service, path, introspect=False), PROPERTIES_IF)
    props.Set(interface, name, value, signature="ssv")


def wait_for(
    predicate: Callable[[], bool], timeout: float, what: str, quiet: bool = False
) -> bool:
    """Poll `predicate` until it holds or `timeout` passes. D-Bus errors
    (e.g. WirePlumber restarting) count as "not yet"."""
    deadline = time.monotonic() + timeout
    last_error = ""
    while True:
        try:
            if predicate():
                if not quiet:
                    log("OK", what)
                return True
        except dbus.DBusException as e:
            if e.get_dbus_name() != last_error:
                last_error = e.get_dbus_name() or ""
                log("RETRY", f"{what}: {last_error}")
        if time.monotonic() >= deadline:
            if not quiet:
                log("TIMEOUT", f"{what} (after {timeout:g} s)")
            return False
        time.sleep(0.1)


# ------------------------------------------------------------------ Telephony


class Telephony:
    """The Phone's Calls as PipeWire's org.pipewire.Telephony shows them.

    Every query goes to the bus afresh (no cached object paths), so a
    WirePlumber restart or a hands-free reconnect cannot leave it stale.
    """

    def __init__(self, test_number: str | None, phone_address: str | None = None) -> None:
        """Without a Test Number it only reads: no Call is ever its own."""
        self.session = dbus.SessionBus()
        self.test_number = test_number
        self.redact = Redactor(test_number) if test_number else None
        self.phone_address = phone_address
        # The Call path we dialled, once known. Trusted only while that
        # path still holds an outgoing Call to the Test Number.
        self.test_call_path: str | None = None
        self.dialled = False

    # -- discovery

    def audio_gateways(self) -> dict[str, dict[str, Any]]:
        """Every AudioGateway, by path, with its properties."""
        objects = call(
            self.session,
            TELEPHONY,
            TELEPHONY_ROOT,
            OBJECT_MANAGER_IF,
            "GetManagedObjects",
            signature="",
        )
        return {p: i for p, i in objects.items() if AUDIO_GATEWAY_IF in i}

    def audio_gateway(self) -> str | None:
        """The Phone's AudioGateway path, or None while it is absent.

        Without a phone address, there must be exactly one AudioGateway,
        and its address is remembered from then on.
        """
        gateways = self.audio_gateways()
        if self.phone_address is None:
            if len(gateways) != 1:
                return None
            path, ifaces = next(iter(gateways.items()))
            self.phone_address = ifaces[AUDIO_GATEWAY_IF].get("Address")
            return path
        for path, ifaces in gateways.items():
            if ifaces[AUDIO_GATEWAY_IF].get("Address") == self.phone_address:
                return path
        return None

    def calls(self, audio_gateway: str) -> dict[str, dict[str, Any]]:
        return call(
            self.session, TELEPHONY, audio_gateway, CALL_MANAGER_IF, "GetCalls", signature=""
        )

    def all_calls(self) -> dict[str, dict[str, Any]]:
        """Every Call on every AudioGateway."""
        out: dict[str, dict[str, Any]] = {}
        for gateway in self.audio_gateways():
            out.update(self.calls(gateway))
        return out

    def is_test_call(self, path: str, props: dict[str, Any]) -> bool:
        """Is this the Call we dialled? Never true for an incoming Call."""
        if self.redact is None or props.get("State") not in OUTGOING_STATES:
            return False
        line = props.get("LineIdentification", "")
        if line:
            return self.redact.matches(line)
        # No LineIdentification yet: ours only if it is the path we already
        # matched, or the only outgoing Call since we dialled.
        return path == self.test_call_path or self.dialled

    def find_test_call(self) -> tuple[str, dict[str, Any]] | None:
        """The Call to the Test Number, re-found from the bus."""
        gateway = self.audio_gateway()
        if gateway is None:
            return None
        calls = self.calls(gateway)
        matches = [(p, c) for p, c in calls.items() if self.is_test_call(p, c)]
        if self.dialled and len(matches) > 1:
            # Ambiguous unnamed Calls: keep only those with our number.
            matches = [(p, c) for p, c in matches if c.get("LineIdentification")]
        if len(matches) != 1:
            return None
        self.test_call_path = matches[0][0]
        return matches[0]

    # -- actions

    def dial_test_number(self, audio_gateway: str) -> None:
        if self.test_number is None:
            raise ValueError("no Test Number to dial")
        self.dialled = True
        call(
            self.session,
            TELEPHONY,
            audio_gateway,
            AUDIO_GATEWAY_IF,
            "Dial",
            self.test_number,
            signature="s",
        )

    def wait_for(
        self, predicate: Callable[[], bool], timeout: float, what: str, quiet: bool = False
    ) -> bool:
        return wait_for(predicate, timeout, what, quiet)

    def hang_up_test_call(self, timeout: float = 30.0) -> bool:
        """Hang up the Call to the Test Number. True once it is confirmed gone.

        Retries until `timeout`, re-finding the Call each time. "Gone" is
        believed only while the Phone's AudioGateway is present, because
        during a WirePlumber restart or hands-free reconnect the Call is
        invisible but still live.
        """
        deadline = time.monotonic() + timeout
        last_error = ""
        while time.monotonic() < deadline:
            try:
                if self.audio_gateway() is None:
                    if last_error != "no AudioGateway":
                        last_error = "no AudioGateway"
                        log("HANGUP", "the Phone's AudioGateway is absent; waiting for it")
                    time.sleep(0.25)
                    continue
                found = self.find_test_call()
                if found is None:
                    log("HANGUP", "no Call to the Test Number remains")
                    return True
                path, props = found
                self._hang_up(path, props)
                if self.wait_for(
                    lambda: self.audio_gateway() is not None and self.find_test_call() is None,
                    5,
                    "the Call to the Test Number is gone",
                ):
                    return True
            except dbus.DBusException as e:
                name = e.get_dbus_name() or ""
                if name != last_error:
                    last_error = name
                    log("HANGUP", f"retrying after {name}")
                time.sleep(0.25)
        log("HANGUP", f"could not confirm the Call ended within {timeout:g} s")
        return False

    def _hang_up(self, path: str, props: dict[str, Any]) -> None:
        # Call1.Hangup first; org.ofono.VoiceCall.Hangup is the fallback for
        # Phones where Call1.Hangup is missing (Foner's report).
        for interface in (CALL_IF, OFONO_CALL_IF):
            try:
                started = time.monotonic()
                call(self.session, TELEPHONY, path, interface, "Hangup", signature="")
                log(
                    "HANGUP",
                    f"{path} ({props.get('State')}) via {interface}.Hangup, "
                    f"returned in {time.monotonic() - started:.3f} s",
                )
                return
            except dbus.DBusException as e:
                log("HANGUP", f"{interface}.Hangup failed: {e.get_dbus_name()}")

    def remaining_calls(self, timeout: float = 10.0) -> dict[str, dict[str, Any]] | None:
        """Last resort: wait for `GetCalls` to be empty on every AudioGateway.

        Returns {} once it is, the Calls still listed after `timeout`, or
        None if the Phone's AudioGateway never came back to ask.
        """
        result: dict[str, dict[str, Any]] | None = None

        def empty() -> bool:
            nonlocal result
            if self.audio_gateway() is None:
                result = None
                return False
            result = self.all_calls()
            return not result

        self.wait_for(empty, timeout, "GetCalls is empty", quiet=True)
        return result


def report_remaining(remaining: dict[str, dict[str, Any]] | None) -> bool:
    """Print the last-resort verdict. True if no Call remains."""
    if remaining == {}:
        log("CHECK", "GetCalls is empty: no Call remains on the Phone")
        return True
    if remaining is None:
        log("ALERT", "cannot confirm that no Call remains: the Phone's AudioGateway is gone.")
        log("ALERT", "Look at the Phone now and hang up any Call there.")
        return False
    for path, props in remaining.items():
        log("ALERT", f"a Call remains: {path} State={props.get('State')}")
    log("ALERT", "Look at the Phone now and hang up any Call to the Test Number there.")
    return False


# ------------------------------------------------------------------ the installed service


class ServiceError(Exception):
    """The installed service could not be stopped or started."""


def die_with_parent() -> None:
    """For `preexec_fn`: the child gets SIGTERM when the process that started
    it dies, even by SIGKILL, so a dev run never outlives its tool."""
    import ctypes

    pr_set_pdeathsig = 1
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(pr_set_pdeathsig, signal.SIGTERM, 0, 0, 0) != 0:
        os._exit(126)
    if os.getppid() == 1:  # the parent died before the prctl
        os._exit(126)


def app_owner(session: dbus.Bus) -> str | None:
    """The unique bus name of the copy of the app on the bus, or None. Asks
    the bus, never the app, and a call to a unique name never activates
    anything, so calls sent to it never start the installed service."""
    try:
        return str(call(session, BUS, BUS_PATH, BUS, "GetNameOwner", APP_BUS_NAME,
                        signature="s"))
    except dbus.DBusException as e:
        if e.get_dbus_name() == "org.freedesktop.DBus.Error.NameHasNoOwner":
            return None
        raise


def app_on_bus(session: dbus.Bus) -> bool:
    """Whether a copy of the app owns its bus name now."""
    return app_owner(session) is not None


class InstalledService:
    """The user's installed `desktop-calls.service`, through the systemd
    user manager on the session bus.

    A Real-Phone run never runs the app beside it (ADR-0006): two apps would
    both react to Calls, both move audio, and fight over the control
    interface. So the run stops the service first, if it runs, and starts it
    when the run ends, whether or not it ran before (the user's choice on
    2026-11-05); a guard process starts it even if the run is killed. Where
    there is no systemd on the bus, or the unit isn't installed, there is
    nothing to stop or start.
    """

    _ABSENT = (
        SYSTEMD + ".NoSuchUnit",
        "org.freedesktop.DBus.Error.ServiceUnknown",
        "org.freedesktop.DBus.Error.NameHasNoOwner",
    )

    def __init__(self, session: dbus.Bus | None = None) -> None:
        self.session = session if session is not None else dbus.SessionBus()

    def active_state(self) -> str:
        """The unit's ActiveState; "inactive" if it isn't installed."""
        try:
            path = call(
                self.session, SYSTEMD, SYSTEMD_PATH, SYSTEMD_MANAGER_IF, "GetUnit",
                INSTALLED_UNIT, signature="s",
            )
        except dbus.DBusException as e:
            if e.get_dbus_name() in self._ABSENT:
                return "inactive"
            raise
        return call(
            self.session, SYSTEMD, path, PROPERTIES_IF, "Get", SYSTEMD_UNIT_IF, "ActiveState",
            signature="ss",
        )

    def running(self) -> bool:
        return self.active_state() in ("active", "activating", "reloading")

    def start_job(self) -> str:
        """The path of the unit's start job still running, or "/"."""
        path = call(self.session, SYSTEMD, SYSTEMD_PATH, SYSTEMD_MANAGER_IF, "GetUnit",
                    INSTALLED_UNIT, signature="s")
        _, job = call(self.session, SYSTEMD, path, PROPERTIES_IF, "Get", SYSTEMD_UNIT_IF, "Job",
                      signature="ss")
        return str(job)

    def app_on_bus(self) -> bool:
        """Whether any copy of the app owns the control interface now."""
        return app_on_bus(self.session)

    def stop(self, timeout: float = 30.0) -> None:
        """Stop the service and wait until it has left the bus."""
        log("SERVICE", f"stopping {INSTALLED_UNIT} for this run; it starts again afterwards")
        call(self.session, SYSTEMD, SYSTEMD_PATH, SYSTEMD_MANAGER_IF, "StopUnit",
             INSTALLED_UNIT, "replace", signature="ss")
        if not wait_for(
            lambda: self.active_state() in ("inactive", "failed") and not self.app_on_bus(),
            timeout, f"{INSTALLED_UNIT} stopped",
        ):
            raise ServiceError(f"{INSTALLED_UNIT} did not stop within {timeout:g} s")

    def start(self, timeout: float = 30.0) -> bool:
        """Start the service at the end of a run. True once it runs, if it
        already runs, if it isn't installed, or if another run's copy of the
        app is still on the bus: the service is never started beside it, and
        that run starts the service when it ends. Keeps no state, so nothing
        a failed start leaves behind affects a later run."""
        try:
            if self.active_state() == "active":
                return True
            if not wait_for(lambda: not self.app_on_bus(), 10,
                            "no other copy of the app on the bus", quiet=True):
                log("SERVICE", f"another copy of the app is still on the bus, so "
                               f"{INSTALLED_UNIT} isn't started beside it. If a `just run` "
                               "or `just test-call` started it, that run starts the service "
                               "when it ends. If none is running, stop that copy, then run "
                               f"`systemctl --user start {INSTALLED_UNIT}`.")
                return True
            log("SERVICE", f"starting {INSTALLED_UNIT}")
            # StartUnit answers before the start ends, while ActiveState may
            # still say how the last start went ("failed"), so the outcome is
            # read once the start job has ended (#155).
            job = str(call(self.session, SYSTEMD, SYSTEMD_PATH, SYSTEMD_MANAGER_IF, "StartUnit",
                           INSTALLED_UNIT, "replace", signature="ss"))
            wait_for(lambda: self.start_job() != job, timeout,
                     f"{INSTALLED_UNIT} start finished", quiet=True)
            started = self.active_state() == "active" and self.app_on_bus()
            if started:
                log("OK", f"{INSTALLED_UNIT} running")
        except dbus.DBusException as e:
            if e.get_dbus_name() in self._ABSENT:
                return True
            log("SERVICE", f"could not start {INSTALLED_UNIT}: {e.get_dbus_name()}")
            started = False
        if not started:
            log("ALERT", f"{INSTALLED_UNIT} is not running. Start it with "
                         f"`systemctl --user start {INSTALLED_UNIT}`.")
        return started

