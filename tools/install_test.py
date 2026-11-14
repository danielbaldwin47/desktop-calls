#!/usr/bin/env python3
"""The public installer produces files their real launchers can consume (#191).

Each test uses a temporary HOME and only its own private D-Bus daemon. No
systemd service is started. Missing optional launcher tools skip that check.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from xml.sax.saxutils import escape

REPOSITORY = Path(__file__).resolve().parent.parent
APP_ID = "io.github.danielbaldwin47.DesktopCalls"
HOMES = (
    "home", "home with spaces", "home%u", "home$HOME${HOME}",
    "home'quote\"\\backtick`", "home*?[glob]",
    "home %u $HOME ${HOME} 'quote\" \\backtick`",
    "home%u=1", "home'quote=1", "home é ü",
)


class Install(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="desktop-calls-install-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.marker = self.root / "executed.json"
        self.binary = self.root / "fixture"
        self.binary.write_text(
            f"#!{sys.executable}\nimport json, sys\nfrom pathlib import Path\n"
            f"Path({str(self.marker)!r}).write_text(json.dumps(sys.argv))\n"
        )

    def install(self, name):
        home = self.root / name
        data = home / ".local/share"
        environment = os.environ | {
            "HOME": str(home), "XDG_DATA_HOME": str(data), "DESTDIR": "",
            "DBUS_SESSION_BUS_ADDRESS": "unix:path=/nonexistent",
        }
        subprocess.run(
            [sys.executable, str(REPOSITORY / "tools/install.py"), str(self.binary)],
            env=environment, check=True, capture_output=True,
        )
        self.marker.unlink(missing_ok=True)
        return home, data, environment

    def ran(self, binary, arguments):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                actual = json.loads(self.marker.read_text())
            except (FileNotFoundError, json.JSONDecodeError):
                time.sleep(0.01)
                continue
            self.assertEqual(actual, [str(binary), *arguments])
            return
        self.fail("the launcher did not invoke the installed executable")

    @unittest.skipUnless(shutil.which("systemd-analyze"), "systemd-analyze is not installed")
    def test_systemd_resolves_the_installed_executable(self):
        for name in HOMES:
            with self.subTest(home=name):
                _, data, environment = self.install(name)
                result = subprocess.run(
                    ["systemd-analyze", "verify", "--man=no", "--generators=no",
                     str(data / "systemd/user/desktop-calls.service")],
                    env=environment, capture_output=True, text=True, timeout=10,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertNotIn("desktop-calls.service:", result.stderr)

    @unittest.skipUnless(shutil.which("gio"), "gio is not installed")
    def test_desktop_launcher_runs_the_binary_and_passes_a_tel_link(self):
        for name in HOMES:
            with self.subTest(home=name):
                home, data, environment = self.install(name)
                result = subprocess.run(
                    ["gio", "launch", str(data / "applications" / f"{APP_ID}.desktop"),
                     "tel:+15555550100"],
                    env=environment, capture_output=True, text=True, timeout=10,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.ran(home / ".local/bin/desktop-calls", ["tel:+15555550100"])

    @unittest.skipUnless(shutil.which("dbus-daemon") and shutil.which("busctl"),
                         "dbus-daemon and busctl are not installed")
    def test_dbus_activation_runs_the_binary_with_the_service_argument(self):
        for name in HOMES:
            with self.subTest(home=name):
                home, data, environment = self.install(name)
                configuration = self.root / "bus.conf"
                configuration.write_text(
                    '<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen>'
                    '<auth>EXTERNAL</auth><servicedir>'
                    + escape(str(data / "dbus-1/services"))
                    + '</servicedir><policy context="default">'
                    '<allow send_destination="*"/><allow receive_sender="*"/>'
                    '<allow own="*"/></policy></busconfig>'
                )
                daemon = subprocess.Popen(
                    ["dbus-daemon", "--nofork", "--print-address=1",
                     f"--config-file={configuration}"],
                    env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                )
                try:
                    address = daemon.stdout.readline().strip()
                    self.assertTrue(address, "private D-Bus daemon did not start")
                    # The fixture records invocation and exits without claiming a bus
                    # name. Activation's reply therefore is not the pass/fail signal.
                    request = subprocess.Popen(
                        ["busctl", f"--address={address}", "--timeout=5s", "call",
                         APP_ID, "/", "org.freedesktop.DBus.Peer", "Ping"],
                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                    )
                    try:
                        self.ran(home / ".local/bin/desktop-calls", ["service"])
                    finally:
                        request.terminate()
                        request.wait(timeout=5)
                finally:
                    daemon.terminate()
                    daemon.communicate(timeout=5)

    def test_staged_install_and_uninstall_keep_paths_outside_the_staging_directory(self):
        home = self.root / "home"
        stage = self.root / "stage"
        environment = os.environ | {
            "HOME": str(home), "XDG_DATA_HOME": "relative-ignored", "DESTDIR": str(stage),
        }
        command = [sys.executable, str(REPOSITORY / "tools/install.py")]
        subprocess.run(command + [str(self.binary)], env=environment, check=True, capture_output=True)
        installed = stage / home.relative_to("/")
        binary = installed / ".local/bin/desktop-calls"
        self.assertEqual(binary.read_bytes(), self.binary.read_bytes())
        self.assertEqual(binary.stat().st_mode & 0o777, 0o755)
        files = list(installed.rglob("*"))
        for path in files:
            if path.is_file() and path != binary:
                self.assertIn(str(home / ".local/bin/desktop-calls"), path.read_text())
                self.assertNotIn(str(stage), path.read_text())
        self.assertEqual(sum(path.is_file() for path in files), 4)
        self.assertFalse(home.exists())
        subprocess.run(command + ["--uninstall"], env=environment, check=True, capture_output=True)
        self.assertFalse(any(path.is_file() for path in installed.rglob("*")))

    def installed_with_recorder(self):
        """A temporary home with the fixture installed, whose runs of the
        installed binary go to `calls.log`, one JSON line each, and whose
        exit status is `exit_status.txt`'s number."""
        home = self.root / "home-for-removal"
        log = self.root / "calls.log"
        status = self.root / "exit_status.txt"
        status.write_text("0")
        self.binary.write_text(
            f"#!{sys.executable}\nimport json, os, sys\nfrom pathlib import Path\n"
            f"with open({str(log)!r}, 'a') as file:\n"
            "    file.write(json.dumps([sys.argv[1:], os.path.exists(os.path.expanduser("
            "'~/.local/share/applications'))]) + '\\n')\n"
            f"sys.exit(int(Path({str(status)!r}).read_text()))\n"
        )
        environment = os.environ | {
            "HOME": str(home), "XDG_DATA_HOME": str(home / ".local/share"), "DESTDIR": "",
        }
        subprocess.run(
            [sys.executable, str(REPOSITORY / "tools/install.py"), str(self.binary)],
            env=environment, check=True, capture_output=True,
        )
        return home, environment, log, status

    def uninstall(self, environment, *arguments):
        return subprocess.run(
            [sys.executable, str(REPOSITORY / "tools/install.py"), *(arguments or ("--uninstall",))],
            env=environment, capture_output=True, text=True,
        )

    def test_uninstall_turns_calls_on_while_the_binary_is_still_installed(self):
        home, environment, log, _ = self.installed_with_recorder()
        result = self.uninstall(environment)
        self.assertEqual(result.returncode, 0, result.stderr)
        # One run, with the files still installed when it ran: the second
        # item says the launcher entry existed.
        self.assertEqual(
            [json.loads(line) for line in log.read_text().splitlines()],
            [[["calls", "on", "--uninstalling"], True]],
        )
        self.assertFalse((home / ".local/bin/desktop-calls").exists())

    def test_uninstall_carries_on_when_turning_calls_on_fails(self):
        home, environment, log, status = self.installed_with_recorder()
        status.write_text("1")
        result = self.uninstall(environment)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(log.read_text().splitlines()), 1)
        self.assertFalse(any(path.is_file() for path in home.rglob("*")))

    def test_uninstall_carries_on_when_the_binary_cannot_run(self):
        home, environment, log, _ = self.installed_with_recorder()
        (home / ".local/bin/desktop-calls").write_text("not a program")
        (home / ".local/bin/desktop-calls").chmod(0o755)
        result = self.uninstall(environment)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("bluetoothctl unblock", result.stderr)
        self.assertFalse((home / ".local/bin/desktop-calls").exists())

    def test_uninstall_without_the_binary_removes_what_is_left(self):
        home, environment, log, _ = self.installed_with_recorder()
        (home / ".local/bin/desktop-calls").unlink()
        result = self.uninstall(environment)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(log.exists())
        self.assertFalse(any(path.is_file() for path in home.rglob("*")))

    def test_staged_uninstall_never_runs_the_binary(self):
        home, environment, log, _ = self.installed_with_recorder()
        result = self.uninstall(environment | {"DESTDIR": str(self.root / "stage")})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(log.exists())
        self.assertTrue((home / ".local/bin/desktop-calls").exists())

    def test_the_two_halves_of_removal_run_alone(self):
        home, environment, log, _ = self.installed_with_recorder()
        self.uninstall(environment, "--turn-calls-on")
        self.assertEqual(len(log.read_text().splitlines()), 1)
        self.assertTrue((home / ".local/bin/desktop-calls").exists())
        self.uninstall(environment, "--remove")
        self.assertEqual(len(log.read_text().splitlines()), 1)
        self.assertFalse((home / ".local/bin/desktop-calls").exists())


if __name__ == "__main__":
    unittest.main()
