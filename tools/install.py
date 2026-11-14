#!/usr/bin/python3
"""Install Desktop Calls for this user, or remove it. `just install` and
`just uninstall` run this; it only places files, and they then tell systemd.

Usage:
  /usr/bin/python3 tools/install.py BINARY            install BINARY and its files
  /usr/bin/python3 tools/install.py --uninstall       turn calls back on, then remove them
  /usr/bin/python3 tools/install.py --turn-calls-on   only the first of those
  /usr/bin/python3 tools/install.py --remove          only the second

Removal first runs the installed `desktop-calls calls on --uninstalling`,
because BlueZ keeps `Device1.Blocked` after the app is gone: a Phone the app
blocked for Calls Off is unblocked, and nothing else is (ticket #215). It
asks the running service, if there is one, so `just uninstall` runs
`--turn-calls-on` before it stops the service, then `--remove`. A package's
removal hook must run the same for each user, with `--uninstall`. The step is
skipped when DESTDIR is set, as a package build stages its files, and a
failure of it never stops the removal.

It installs, in the user's own directories:
  ~/.local/bin/desktop-calls                                  the binary
  $XDG_DATA_HOME/systemd/user/desktop-calls.service           the systemd user unit
  $XDG_DATA_HOME/dbus-1/services/<bus name>.service           D-Bus activation
  $XDG_DATA_HOME/applications/<app id>.desktop                launcher entry, tel: links

XDG_DATA_HOME defaults to ~/.local/share. Each file that names the binary
names it by its installed path. With DESTDIR set, the files go under DESTDIR
instead, still naming the installed path, as a package build stages them.

Run it with the system Python: a version manager's `python3` shim (mise's)
can fail when HOME isn't the user's usual one.
"""

from __future__ import annotations

import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parent.parent
APP_ID = "io.github.danielbaldwin47.DesktopCalls"
UNIT = "desktop-calls.service"
# Runs the path given as its first argument with the rest. env would not do:
# it reads any word containing "=" as an assignment, even after "--".
RUN_ARGUMENT = """/bin/sh -c 'exec "$0" "$@"'"""


def systemd_command(path: Path) -> str:
    """Encode the executable for ExecStart, including percent specifiers.

    Environment expansion applies to arguments, not the executable path.
    See systemd.service(5), Command lines, and systemd.syntax(7), Quoting.
    """
    text = str(path)
    # systemd's executable validation rejects quotes, backslashes, globs and
    # controls even when quoted. RUN_ARGUMENT accepts them as an argument.
    indirect = any(
        character in "'\"\\*?[" or ord(character) < 32 or ord(character) == 127
        for character in text
    )
    text = text.replace("%", "%%")
    if indirect:
        text = text.replace("$", "$$")
    if re.fullmatch(r"[\w/%.\-]+", text):
        return text
    text = "".join(
        f"\\x{ord(character):02x}" if ord(character) < 32 or ord(character) == 127
        else "\\" + character if character in '\\"' else character
        for character in text
    )
    prefix = RUN_ARGUMENT.replace("$", "$$") + " " if indirect else ""
    return prefix + '"' + text + '"'


def entry_value(text: str) -> str:
    """Escape the string-value layer shared by desktop and D-Bus files."""
    return text.replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")


def desktop_command(path: Path) -> str:
    """Desktop Entry Exec quoting, then string-value escaping (two layers)."""
    text = str(path).replace("%", "%%")
    if not re.fullmatch(r"[\w/%.\-]+", text):
        text = '"' + quote_exec(text) + '"'
    # Gio checks argv[0] for existence before expanding %% into %.
    if "%" in str(path):
        text = '/bin/sh -c "' + quote_exec("exec \"$0\" \"$@\"") + '" ' + text
    return entry_value(text)


def quote_exec(text: str) -> str:
    """Escape the characters Desktop Entry Exec reserves inside double quotes."""
    for character in ('\\', '"', '`', '$'):
        text = text.replace(character, "\\" + character)
    return text


def directories() -> tuple[Path, Path]:
    """The binary's directory and the data directory, as installed."""
    home = Path(os.environ["HOME"])
    data = os.environ.get("XDG_DATA_HOME") or ""
    # The XDG spec ignores a relative XDG_DATA_HOME.
    data_home = Path(data) if os.path.isabs(data) else home / ".local/share"
    return home / ".local/bin", data_home


def staged(path: Path) -> Path:
    """Where `path` is written: under DESTDIR, if set."""
    destination = os.environ.get("DESTDIR")
    return Path(destination) / path.relative_to("/") if destination else path


def write_atomically(path: Path, write) -> None:
    """Writes `path` through a temporary file beside it, so a reader, or the
    running binary being replaced, never sees a half-written file."""
    path.parent.mkdir(parents=True, exist_ok=True)
    handle, temporary = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.")
    try:
        with os.fdopen(handle, "wb") as file:
            write(file)
        os.replace(temporary, path)
    except BaseException:
        Path(temporary).unlink(missing_ok=True)
        raise


def installed_files() -> dict[Path, Path | None]:
    """Each installed path, with its template in data/ (None for the binary)."""
    bin_directory, data_home = directories()
    return {
        bin_directory / "desktop-calls": None,
        data_home / "systemd/user" / UNIT: REPOSITORY / "data/systemd" / f"{UNIT}.in",
        data_home / "dbus-1/services" / f"{APP_ID}.service": (
            REPOSITORY / "data/dbus-1" / f"{APP_ID}.service.in"
        ),
        data_home / "applications" / f"{APP_ID}.desktop": REPOSITORY / "data" / f"{APP_ID}.desktop",
    }


def install(binary: Path) -> None:
    bin_directory, _ = directories()
    installed_binary = bin_directory / "desktop-calls"
    for path, template in installed_files().items():
        if template is None:
            with binary.open("rb") as source:
                write_atomically(staged(path), lambda file: shutil.copyfileobj(source, file))
            staged(path).chmod(0o755)
        else:
            text = template.read_text()
            if template.parent.name == "systemd":
                command = systemd_command(installed_binary)
            else:
                # D-Bus unescapes the entry value before its shell-style
                # argument parser. It does not expand variables or percent.
                command = entry_value(shlex.quote(str(installed_binary)))
            text = text.replace("@COMMAND@", command)
            # The desktop file runs the binary by its path, not through PATH,
            # which a launcher may not share with the shell.
            text = text.replace("Exec=desktop-calls ", f"Exec={desktop_command(installed_binary)} ")
            write_atomically(staged(path), lambda file: file.write(text.encode()))
            staged(path).chmod(0o644)
        print(f"Installed {staged(path)}")


def turn_calls_on() -> None:
    """Has the installed app turn calls back on, so that no Phone it blocked
    stays blocked once it is gone. Says what went wrong, and carries on."""
    if os.environ.get("DESTDIR"):
        return
    bin_directory, _ = directories()
    binary = bin_directory / "desktop-calls"
    if not binary.exists():
        return
    try:
        # The app prints its own sentence, and its own failure.
        subprocess.run([binary, "calls", "on", "--uninstalling"], check=False, timeout=60)
    except (OSError, subprocess.SubprocessError) as error:
        print(
            f"Couldn't run {binary} to turn calls back on ({error}). If your phone refuses "
            "this computer, run `bluetoothctl unblock` with its address.",
            file=sys.stderr,
        )


def remove() -> None:
    for path in installed_files():
        if staged(path).exists():
            staged(path).unlink()
            print(f"Removed {staged(path)}")


def uninstall() -> None:
    turn_calls_on()
    remove()


def main(arguments: list[str]) -> int:
    actions = {"--uninstall": uninstall, "--turn-calls-on": turn_calls_on, "--remove": remove}
    if len(arguments) == 1 and arguments[0] in actions:
        actions[arguments[0]]()
        return 0
    if len(arguments) != 1 or arguments[0].startswith("-"):
        print(__doc__, file=sys.stderr)
        return 2
    binary = Path(arguments[0])
    if not binary.is_file():
        print(f"install.py: no binary at {binary}; build it first", file=sys.stderr)
        return 1
    install(binary)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
