#!/usr/bin/env python3
"""Run the tests under CI's limits on the Desktop, to reproduce a CI-only
failure before pushing. No root, no Docker.

Usage (through just):
  just test-ci-like                                   # the whole suite
  just test-ci-like -p test-harness --test scenarios -- incoming_call_surfaces::
  just test-file incoming_call_surfaces --ci-like       # the same

Arguments go to `cargo test --workspace --locked`, as with `just test`.

The limits, calibrated so that main passes and reverting PR #41's fix for
slow caller formatting fails the Banner test (see
docs/research/2026-11-05-diagnosable-ci.md):

  - CPU: a cgroup quota of CPU_QUOTA for every test process together
    (systemd-run --user --scope -p CPUQuota=...), on whichever CPUs the
    kernel picks. Never pinned to one CPU (taskset -c 0): agents copy the
    recipe, and every copy would pile onto CPU 0 and starve the others.
  - TEST_THREADS scenarios at a time.
  - No GPU: an empty /dev/dri. CI's runners have none.
  - /dev/shm of SHM_SIZE, as CI's container has (--shm-size in
    .github/workflows/ci.yml; Docker's default of 64 MB failed every screen
    scenario four at a time, ticket #41).

The GPU and /dev/shm are hidden in a private mount namespace, so nothing
outside the run sees them change.

The Desktop is shared by several agents, so:

  - A machine-wide lock ($XDG_RUNTIME_DIR/desktop-calls-test-ci-like.lock)
    keeps two runs from overlapping and skewing each other's timing. While
    waiting, it prints who holds the lock.
  - The binaries build first, outside the lock and the quota.
  - Each run has its own TMPDIR, removed afterwards. Failed scenarios keep
    their directories: they move to target/ci-like/<time>/, and the last
    line says where.

Exit status: that of cargo test; 2 if the run could not start.
"""

from __future__ import annotations

import datetime
import fcntl
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import tempfile
import time

# The calibrated limits: see the module's documentation. TEST_CI_LIKE_CPU_QUOTA
# overrides the quota, for recalibrating (e.g. TEST_CI_LIKE_CPU_QUOTA=150%).
CPU_QUOTA = os.environ.get("TEST_CI_LIKE_CPU_QUOTA") or "100%"
TEST_THREADS = "4"
SHM_SIZE = "1g"

LOCK_NAME = "desktop-calls-test-ci-like.lock"
# How often to check a held lock, and print its holder if that changed.
LOCK_POLL_SECONDS = 2

# Mounts the limits in a new user and mount namespace, then maps the user
# back to their own ID (as the harness does for the app) and runs the tests.
CONFINE = r"""
mount -t tmpfs -o size="$SHM_SIZE" none /dev/shm &&
if [ -d /dev/dri ]; then mount -t tmpfs none /dev/dri; fi &&
exec unshare --user --map-user="$OUTER_UID" --map-group="$OUTER_GID" -- "$@"
"""


class CannotRun(Exception):
    """The run could not start. The message says why and what to do."""


def take_lock() -> int:
    """Takes the machine-wide lock, waiting for it if needed, and returns
    its file descriptor. The lock goes when the process does."""
    runtime = os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}"
    lock_path = pathlib.Path(runtime) / LOCK_NAME
    holder_path = lock_path.with_suffix(".holder")
    lock = os.open(lock_path, os.O_RDWR | os.O_CREAT, 0o600)
    shown = None
    started = time.monotonic()
    while True:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            break
        except BlockingIOError:
            try:
                holder = holder_path.read_text().strip()
            except OSError:
                holder = "another run (it hasn't said who it is yet)"
            if holder != shown:
                print(f"just test-ci-like: waiting for {holder}", file=sys.stderr, flush=True)
                shown = holder
            time.sleep(LOCK_POLL_SECONDS)
    waited = time.monotonic() - started
    if shown is not None:
        print(f"just test-ci-like: got the lock after {waited:.0f}s", file=sys.stderr)
    branch = subprocess.run(
        ["git", "branch", "--show-current"], capture_output=True, text=True
    ).stdout.strip()
    since = datetime.datetime.now().strftime("%H:%M:%S")
    holder_path.write_text(
        f"process {os.getpid()} in {os.getcwd()} (branch {branch or '?'}), running since "
        f"{since}: just test-ci-like {' '.join(sys.argv[1:])}\n"
    )
    return lock


def build(arguments: list[str]) -> None:
    """Builds the binaries and test executables, outside the quota."""
    for command in (
        ["cargo", "build", "--workspace", "--bins", "--locked"],
        ["cargo", "test", "--workspace", "--locked", "--no-run", *arguments],
    ):
        if subprocess.run(command).returncode != 0:
            raise CannotRun(f"`{' '.join(command)}` failed")


def run_confined(arguments: list[str], temporary: pathlib.Path, unit: str) -> int:
    environment = dict(
        os.environ,
        TMPDIR=str(temporary),
        RUST_TEST_THREADS=TEST_THREADS,
        DESKTOP_CALLS_CI_LIKE="1",
        SHM_SIZE=SHM_SIZE,
        OUTER_UID=str(os.getuid()),
        OUTER_GID=str(os.getgid()),
    )
    command = [
        "systemd-run", "--user", "--scope", "--quiet", "--collect", f"--unit={unit}",
        f"--property=CPUQuota={CPU_QUOTA}",
        "--",
        "unshare", "--user", "--map-root-user", "--mount", "--",
        "sh", "-c", CONFINE, "confine",
        "cargo", "test", "--workspace", "--locked", *arguments,
    ]
    print(
        f"just test-ci-like: CPU quota {CPU_QUOTA}, {TEST_THREADS} test threads, no GPU, "
        f"/dev/shm {SHM_SIZE}, TMPDIR {temporary}",
        file=sys.stderr,
        flush=True,
    )
    try:
        return subprocess.run(command, env=environment).returncode
    except FileNotFoundError as missing:
        raise CannotRun(
            f"{missing.filename} is missing: the quota needs systemd's user manager "
            "(systemd-run --user) and the namespaces util-linux's unshare"
        ) from missing


def stop(unit: str) -> None:
    """Stops the run's scope, killing anything of this run still in it.
    A no-op once the tests have exited, since the scope then goes too."""
    subprocess.run(
        ["systemctl", "--user", "stop", "--quiet", f"{unit}.scope"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def keep_failed(temporary: pathlib.Path) -> str | None:
    """Moves the scenario directories failed scenarios kept into
    target/ci-like/<time>/, and returns where, if there were any."""
    kept = sorted(temporary.glob("desktop-calls-test-*"))
    if not kept:
        return None
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    destination = pathlib.Path("target/ci-like") / stamp
    destination.mkdir(parents=True, exist_ok=True)
    for directory in kept:
        # Sockets and FIFOs can't be copied across file systems; skip them.
        shutil.copytree(
            directory,
            destination / directory.name,
            symlinks=True,
            ignore_dangling_symlinks=True,
            copy_function=copy_regular,
        )
    noun = "directory" if len(kept) == 1 else "directories"
    return f"{len(kept)} kept scenario {noun} in {destination}/ (the paths above were under {temporary})"


def copy_regular(source: str, destination: str) -> None:
    if pathlib.Path(source).is_file():
        shutil.copy2(source, destination)


def main() -> int:
    arguments = sys.argv[1:]
    if arguments[:1] in (["-h"], ["--help"]):
        print(__doc__)
        return 0
    # A SIGTERM unwinds like Ctrl-C does, so the cleanup below still runs.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    unit = f"desktop-calls-test-ci-like-{os.getpid()}"
    try:
        build(arguments)
        lock = take_lock()
        temporary = pathlib.Path(tempfile.mkdtemp(prefix="dc-ci-like-", dir="/tmp"))
        try:
            try:
                status = run_confined(arguments, temporary, unit)
            except KeyboardInterrupt:
                status = 130
            kept = keep_failed(temporary)
        finally:
            stop(unit)
            shutil.rmtree(temporary, ignore_errors=True)
            os.close(lock)
        verdict = "passed" if status == 0 else f"failed (exit status {status})"
        print(f"just test-ci-like: {verdict}" + (f"; {kept}" if kept else ""), file=sys.stderr)
        return status
    except CannotRun as failure:
        print(f"just test-ci-like: {failure}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
