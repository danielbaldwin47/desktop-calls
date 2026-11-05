#!/usr/bin/python3
"""PBAP probe for the real Phone, from ticket #9 (Contacts sync).

Opens one obexd PBAP session to the Phone, pulls each PBAP folder the app
reads (`pb`, `cch`, `ich`, `och`, `mch`), and reports only the *shape* of
what came back: which folders exist, the Transfer1 object and its Status
changes, vCard versions, which properties and parameters appear, how
numbers and timestamps are written (every digit replaced by D), and how
many entries each folder has and share with the others. No name, number or
other Contact value is ever printed or written: the pulled files are
deleted as soon as they are read, and the Test Number is redacted from all
output as in every probe.

It then pulls a second page of `cch` (Offset 50, MaxCount 50), because the
Pixel gives at most 50 Call History entries per pull, and reports whether
the Phone honours paging as the Call History sync assumes (#9): page 2 is
not page 1 again, and every entry on it is older than those on page 1.

No Call is placed, and nothing on the Phone changes. obexd writes each
pulled folder to a file in a private directory under
$XDG_CACHE_HOME/desktop-calls (~/.cache/desktop-calls), deleted at the end.

Have the Phone unlocked and in hand: if it has not yet been told whether
this Desktop may read Contacts, it asks ("Allow access to contacts and call
history?"). On 2026-11-04 CreateSession failed twice after about 10 s with
`org.bluez.obex.Error.Failed: Timed out waiting for response` while nobody
was at the Phone (docs/research/2026-11-04-contacts-sync.md).

Usage (through just, which runs it under /usr/bin/python3):
  just pbap-probe [--out OUT.json]
      Print what each pull gave and the paging verdict, and write the shape
      report ("report") with the D-Bus snapshot ("dbus snapshot": obexd's
      client object, a session, a Transfer1 during a pull) to OUT.json, by
      default ~/.cache/desktop-calls/pbap-probe-<time>.json.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
import time
from collections import Counter
from pathlib import Path
from typing import Any

import dbus
import dbus.mainloop.glib
from gi.repository import GLib

import real_phone as rp
from real_phone import log

OBEX = "org.bluez.obex"
CLIENT_PATH = "/org/bluez/obex"
CLIENT_IF = "org.bluez.obex.Client1"
PHONEBOOK_IF = "org.bluez.obex.PhonebookAccess1"
TRANSFER_IF = "org.bluez.obex.Transfer1"
HFP_AG_UUID = "0000111f-0000-1000-8000-00805f9b34fb"
BOOKS = ("pb", "cch", "ich", "och", "mch")
# The second page of Call History: the Pixel caps each pull at 50 entries.
PAGE_2 = {"Offset": 50, "MaxCount": 50}

# Set by connect(), after the arguments are parsed, so that --help needs
# neither the Test Number nor a bus.
REDACT: rp.Redactor
CTX: Any
SESSION: Any
SYSTEM: Any


def connect() -> None:
    global REDACT, CTX, SESSION, SYSTEM
    REDACT = rp.install_redaction(rp.read_test_number())
    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
    CTX = GLib.MainContext.default()
    SESSION = dbus.SessionBus()
    SYSTEM = dbus.SystemBus()


def cache_directory() -> Path:
    """$XDG_CACHE_HOME/desktop-calls, private to the user."""
    cache = os.environ.get("XDG_CACHE_HOME") or str(Path.home() / ".cache")
    directory = Path(cache) / "desktop-calls"
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    return directory

# Transfer1 Status changes by transfer path, with seconds since the pull.
STATUS: dict[str, list[tuple[float, str]]] = {}
PULL_STARTED: dict[str, float] = {}


def on_properties_changed(interface: str, changed: Any, _invalidated: Any, path: str = "") -> None:
    if interface == TRANSFER_IF and "Status" in changed:
        started = PULL_STARTED.get(path, time.monotonic())
        STATUS.setdefault(path, []).append((round(time.monotonic() - started, 3), str(changed["Status"])))


def wait_for_end(transfer: str, timeout: float) -> str:
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        seen = [status for _, status in STATUS.get(transfer, [])]
        if seen and seen[-1] in ("complete", "error"):
            return seen[-1]
        if not CTX.iteration(False):
            time.sleep(0.005)
    return "timeout"


def phone_address() -> str:
    objects = rp.call(SYSTEM, rp.BLUEZ, "/", rp.OBJECT_MANAGER_IF, "GetManagedObjects", signature="")
    for interfaces in objects.values():
        device = interfaces.get(rp.DEVICE1_IF, {})
        uuids = [u.lower() for u in device.get("UUIDs", [])]
        if device.get("Paired") and device.get("Connected") and HFP_AG_UUID in uuids:
            return str(device["Address"])
    sys.exit("No paired, connected Phone offering hands-free; is the Phone connected?")


def introspect(path: str) -> str:
    return rp.call(SESSION, OBEX, path, rp.INTROSPECTABLE_IF, "Introspect", signature="")


def get_all(path: str, interface: str) -> dict[str, Any]:
    return rp.call(SESSION, OBEX, path, rp.PROPERTIES_IF, "GetAll", interface, signature="s")


def typed(properties: dict[str, Any], raw: Any) -> dict[str, Any]:
    """Each property with its D-Bus type, the way busctl's JSON reports it."""
    out = {}
    for name, value in raw.items():
        out[str(name)] = {"type": dbus_type(value), "value": properties.get(str(name))}
    return out


def dbus_type(value: Any) -> str:
    for kind, code in ((dbus.Boolean, "b"), (dbus.Byte, "y"), (dbus.UInt16, "q"), (dbus.Int16, "n"),
                       (dbus.UInt32, "u"), (dbus.Int32, "i"), (dbus.UInt64, "t"), (dbus.Int64, "x"),
                       (dbus.ObjectPath, "o"), (dbus.String, "s"), (dbus.Double, "d")):
        if isinstance(value, kind):
            return code
    return "?"


def get_all_typed(path: str, interface: str) -> dict[str, Any]:
    raw = SESSION.call_blocking(OBEX, path, rp.PROPERTIES_IF, "GetAll", "s", (interface,), timeout=5.0)
    return typed(rp.plain(raw), raw)


# ------------------------------------------------------------ vCard shapes


def unfold(text: str) -> list[str]:
    """vCard lines with folded continuations (CRLF + space or tab) joined."""
    lines: list[str] = []
    for line in text.replace("\r\n", "\n").split("\n"):
        if line[:1] in (" ", "\t") and lines:
            lines[-1] += line[1:]
        elif line:
            lines.append(line)
    return lines


def cards(text: str) -> list[list[str]]:
    found, current = [], None
    for line in unfold(text):
        upper = line.upper()
        if upper == "BEGIN:VCARD":
            current = []
        elif upper == "END:VCARD" and current is not None:
            found.append(current)
            current = None
        elif current is not None:
            current.append(line)
    return found


def shape(value: str) -> str:
    """A value with every digit as D and every letter as A: its format only."""
    return re.sub(r"[A-Za-z]", "A", re.sub(r"\d", "D", value))


def split_property(line: str) -> tuple[str, list[str], str]:
    head, _, value = line.partition(":")
    name, *params = head.split(";")
    return name.upper(), [p.upper() for p in params], value


def book_report(text: str) -> tuple[dict[str, Any], set[str], list[str]]:
    """The shape of one pulled book, a hash per (number, time) entry, and
    each entry's X-IRMC-CALL-DATETIME (empty if it has none)."""
    raw_lines = text.replace("\r\n", "\n").split("\n")
    report: dict[str, Any] = {
        "bytes": len(text.encode()),
        "crlf_line_endings": "\r\n" in text,
        "folded_lines": sum(1 for line in raw_lines if line[:1] in (" ", "\t")),
        "quoted_printable_properties": 0,
    }
    found = cards(text)
    report["entries"] = len(found)
    versions: Counter[str] = Counter()
    properties: Counter[str] = Counter()
    tel_params: Counter[str] = Counter()
    number_shapes: Counter[str] = Counter()
    datetime_shapes: Counter[str] = Counter()
    datetime_params: Counter[str] = Counter()
    name_params: Counter[str] = Counter()
    without_number = 0
    without_name = 0
    tels_per_entry: Counter[int] = Counter()
    keys: set[str] = set()
    times: list[str] = []
    for card in found:
        tels, has_name, when = [], False, ""
        for line in card:
            name, params, value = split_property(line)
            properties[name] += 1
            if any("QUOTED-PRINTABLE" in p for p in params):
                report["quoted_printable_properties"] += 1
            if name == "VERSION":
                versions[value] += 1
            elif name == "TEL":
                tel_params[";".join(params) or "(none)"] += 1
                if value.strip():
                    tels.append(value.strip())
                    number_shapes[shape(value.strip())] += 1
            elif name in ("FN", "N"):
                name_params[f"{name};{';'.join(params)}" if params else name] += 1
                if value.strip(" ;"):
                    has_name = True
            elif name == "X-IRMC-CALL-DATETIME":
                datetime_params[";".join(params) or "(none)"] += 1
                datetime_shapes[shape(value)] += 1
                when = value
        tels_per_entry[len(tels)] += 1
        without_number += not tels
        without_name += not has_name
        digits = re.sub(r"\D", "", tels[0]) if tels else ""
        keys.add(hashlib.sha256(f"{digits}|{when}".encode()).hexdigest())
        times.append(when)
    report.update({
        "versions": dict(versions),
        "properties": dict(properties),
        "tel_parameters": dict(tel_params),
        "number_shapes": dict(number_shapes.most_common(12)),
        "numbers_per_entry": {str(k): v for k, v in sorted(tels_per_entry.items())},
        "entries_without_number": without_number,
        "entries_without_name": without_name,
        "name_parameters": dict(name_params),
        "call_datetime_parameters": dict(datetime_params),
        "call_datetime_shapes": dict(datetime_shapes),
    })
    return report, keys, times


# --------------------------------------------------------------- the probe


def pull(session: str, book: str, directory: Path, snapshot: dict[str, Any],
         page: dict[str, int] | None = None, name: str = "") -> dict[str, Any]:
    name = name or book
    result: dict[str, Any] = {}
    try:
        rp.call(SESSION, OBEX, session, PHONEBOOK_IF, "Select", "int", book, signature="ss")
    except dbus.DBusException as e:
        result["select_error"] = e.get_dbus_name()
        log("SELECT", f"{book}: {e.get_dbus_name()}")
        return result
    try:
        result["size"] = int(rp.call(SESSION, OBEX, session, PHONEBOOK_IF, "GetSize", signature=""))
    except dbus.DBusException as e:
        result["size_error"] = e.get_dbus_name()
    target = directory / f"{name}.vcf"
    filters = {"Format": dbus.String("vcard30")}
    # Offset and MaxCount are `q` in obexd (obexd/client/pbap.c).
    filters.update({key: dbus.UInt16(value) for key, value in (page or {}).items()})
    started = time.monotonic()
    try:
        transfer, properties = rp.call(SESSION, OBEX, session, PHONEBOOK_IF, "PullAll", str(target),
                                       filters, signature="sa{sv}", timeout=60.0)
    except dbus.DBusException as e:
        result["pull_error"] = e.get_dbus_name()
        log("PULL", f"{name}: {e.get_dbus_name()}")
        return result
    PULL_STARTED[transfer] = started
    result["pull_reply_seconds"] = round(time.monotonic() - started, 3)
    result["pull_reply_properties"] = sorted(properties)
    if "transfer" not in snapshot:
        try:
            snapshot["transfer"] = {
                "bus": "user",
                "service": OBEX,
                "path": transfer,
                "introspection_xml": introspect(transfer),
                "properties": {TRANSFER_IF: get_all_typed(transfer, TRANSFER_IF)},
                "PullAll reply properties": typed(properties, properties),
            }
        except dbus.DBusException as e:
            log("SNAPSHOT", f"Transfer1 gone before it could be captured: {e.get_dbus_name()}")
    result["end"] = wait_for_end(transfer, 60.0)
    result["status_changes"] = STATUS.get(transfer, [])
    result["seconds"] = round(time.monotonic() - started, 3)
    if target.exists():
        mode = target.stat().st_mode & 0o777
        result["file_mode"] = oct(mode)
        text = target.read_bytes().decode("utf-8", errors="replace")
        target.unlink()
        result["shape"], result["_keys"], result["_times"] = book_report(text)
    log("PULL", f"{name}: {result['end']} in {result['seconds']}s, "
                f"{result.get('shape', {}).get('entries', 0)} entries")
    return result


def paging(page_1: dict[str, Any], page_2: dict[str, Any]) -> dict[str, Any]:
    """Whether the Phone honours Offset on `cch`, from its first two pages.

    A Phone that ignores Offset gives page 1 again; one that counts Offset
    from the newest Call, as PBAP says and the app assumes, gives entries
    all older than page 1's. A Call made between the two pulls can shift
    one entry onto both pages, so a page 2 that is mostly new still counts.
    """
    keys_1, keys_2 = page_1.get("_keys", set()), page_2.get("_keys", set())
    entries_1 = page_1.get("shape", {}).get("entries", 0)
    entries_2 = page_2.get("shape", {}).get("entries", 0)
    times_1 = [t for t in page_1.get("_times", []) if t]
    times_2 = [t for t in page_2.get("_times", []) if t]
    result: dict[str, Any] = {
        "page 2 filters": PAGE_2,
        "cch size": page_1.get("size"),
        "page 1 entries": entries_1,
        "page 2 entries": entries_2,
        "page 2 entries also on page 1": len(keys_1 & keys_2),
        # X-IRMC-CALL-DATETIME is YYYYMMDDTHHMMSS, so it sorts as text.
        "page 2 older than page 1": (max(times_2) <= min(times_1)) if times_1 and times_2 else None,
    }
    failed = next((page_2[k] for k in ("select_error", "pull_error") if k in page_2), None)
    if failed or page_2.get("end") not in (None, "complete"):
        result["honours offset"] = None
        result["verdict"] = f"Inconclusive: the page 2 pull failed ({failed or page_2.get('end')})."
    elif isinstance(result["cch size"], int) and result["cch size"] <= PAGE_2["Offset"]:
        result["honours offset"] = None
        result["verdict"] = (f"Inconclusive: cch holds only {result['cch size']} entries, "
                             "so there is no page 2 to read.")
    elif entries_2 == 0:
        result["honours offset"] = False
        result["verdict"] = "The Phone gave an empty page 2, though cch holds more than 50 entries."
    elif result["page 2 entries also on page 1"] * 2 > entries_2:
        result["honours offset"] = False
        result["verdict"] = "The Phone ignores Offset: page 2 is page 1 again."
    elif result["page 2 older than page 1"] is False:
        result["honours offset"] = True
        result["verdict"] = ("The Phone honours Offset, but page 2 is not all older than page 1, "
                             "so Offset doesn't count from the newest Call.")
    else:
        result["honours offset"] = True
        result["verdict"] = (f"The Phone honours Offset: page 2 gave {entries_2} entries, "
                             f"{result['page 2 entries also on page 1']} of them on page 1 too, "
                             + ("all older than page 1's." if result["page 2 older than page 1"]
                                else "with no Call times to order them by."))
    return result


def scrub(value: Any, address: str) -> Any:
    """The snapshot with the Phone's and Desktop's addresses and paths generalised."""
    text = json.dumps(value)
    text = text.replace(address, "FC:41:16:00:00:01")
    text = re.sub(r"\b([0-9A-F]{2}:){5}[0-9A-F]{2}\b", "00:00:00:00:00:00", text)
    text = re.sub(r'"/[^"]*/pbap-probe-[^"/]*/([a-z0-9-]+\.vcf)"', r'"<targetfile \1>"', text)
    return json.loads(REDACT(text))


def main() -> int:
    parser = argparse.ArgumentParser(
        prog="just pbap-probe",
        description=__doc__.split("\n\n")[0].replace("\n", " "),
        epilog="Have the Phone unlocked and in hand: it may ask to allow access to contacts "
               "and call history.",
    )
    parser.add_argument("--out", type=Path, metavar="OUT.json",
                        help="where to write the report and the D-Bus snapshot "
                             "(default: ~/.cache/desktop-calls/pbap-probe-<time>.json)")
    args = parser.parse_args()
    connect()
    out = args.out or cache_directory() / f"pbap-probe-{time.strftime('%Y%m%d-%H%M%S')}.json"
    address = phone_address()
    SESSION.add_signal_receiver(on_properties_changed, "PropertiesChanged", rp.PROPERTIES_IF, OBEX,
                                path_keyword="path")
    snapshot: dict[str, Any] = {
        "client": {
            "bus": "user", "service": OBEX, "path": CLIENT_PATH,
            "introspection_xml": introspect(CLIENT_PATH), "properties": {},
        },
        "GetManagedObjects(/) before a session": rp.call(
            SESSION, OBEX, "/", rp.OBJECT_MANAGER_IF, "GetManagedObjects", signature=""),
    }
    started = time.monotonic()
    try:
        session = rp.call(SESSION, OBEX, CLIENT_PATH, CLIENT_IF, "CreateSession", address,
                          {"Target": dbus.String("pbap")}, signature="sa{sv}", timeout=60.0)
    except dbus.DBusException as e:
        log("SESSION", f"CreateSession failed after {time.monotonic() - started:.2f}s: "
                       f"{e.get_dbus_name()}: {e.get_dbus_message()}")
        return 1
    log("SESSION", f"opened in {time.monotonic() - started:.2f}s")
    books: dict[str, Any] = {}
    try:
        with tempfile.TemporaryDirectory(prefix="pbap-probe-", dir=cache_directory()) as directory:
            for book in BOOKS:
                books[book] = pull(session, book, Path(directory), snapshot)
            page_2 = pull(session, "cch", Path(directory), snapshot, PAGE_2, "cch-page-2")
        try:
            rp.call(SESSION, OBEX, session, PHONEBOOK_IF, "Select", "int", "nosuchbook", signature="ss")
            books["select an unknown book"] = "accepted"
        except dbus.DBusException as e:
            books["select an unknown book"] = e.get_dbus_name()
    finally:
        rp.call(SESSION, OBEX, CLIENT_PATH, CLIENT_IF, "RemoveSession", dbus.ObjectPath(session),
                signature="o")
        log("SESSION", "removed")
    call_history_paging = paging(books.get("cch", {}), page_2)
    log("PAGING", call_history_paging["verdict"])
    for pulled in (*books.values(), page_2):
        if isinstance(pulled, dict):
            pulled.pop("_times", None)
    history = {b: books[b].pop("_keys", set()) for b in ("cch", "ich", "och", "mch")
               if isinstance(books.get(b), dict)}
    books.get("pb", {}).pop("_keys", None)
    page_2.pop("_keys", None)
    books["cch page 2"] = page_2
    split = set().union(*(history.get(b, set()) for b in ("ich", "och", "mch")))
    overlap = {
        "cch entries also in ich/och/mch": len(history.get("cch", set()) & split),
        "ich/och/mch entries not in cch": len(split - history.get("cch", set())),
        "entries in more than one of ich/och/mch": sum(
            1 for key in split
            if sum(key in history.get(b, set()) for b in ("ich", "och", "mch")) > 1),
    }
    snapshot["captured"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    report = scrub({"books": books, "history overlap": overlap,
                    "call history paging": call_history_paging}, address)
    out.parent.mkdir(parents=True, exist_ok=True)
    with open(out, "w") as f:
        f.write(json.dumps({"report": report, "dbus snapshot": scrub(snapshot, address)}, indent=2) + "\n")
    log("REPORT", f"wrote {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
