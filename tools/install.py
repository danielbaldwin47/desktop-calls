#!/usr/bin/python3
"""Install Desktop Calls for this user, or remove it. `just install` and
`just uninstall` run this; it only places files, and they then tell systemd.

Usage:
  /usr/bin/python3 tools/install.py BINARY      install BINARY and its files
  /usr/bin/python3 tools/install.py --uninstall remove them

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
import shutil
import sys
import tempfile
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parent.parent
APP_ID = "io.github.danielbaldwin47.DesktopCalls"
UNIT = "desktop-calls.service"


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
            text = text.replace("@BINDIR@", str(bin_directory))
            # The desktop file runs the binary by its path, not through PATH,
            # which a launcher may not share with the shell.
            text = text.replace("Exec=desktop-calls ", f"Exec={installed_binary} ")
            write_atomically(staged(path), lambda file: file.write(text.encode()))
            staged(path).chmod(0o644)
        print(f"Installed {staged(path)}")


def uninstall() -> None:
    for path in installed_files():
        if staged(path).exists():
            staged(path).unlink()
            print(f"Removed {staged(path)}")


def main(arguments: list[str]) -> int:
    if arguments == ["--uninstall"]:
        uninstall()
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
