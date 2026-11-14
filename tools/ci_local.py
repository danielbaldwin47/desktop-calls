#!/usr/bin/env python3
"""Run CI's jobs on this machine, and post "CI passed" on the commit if they pass.

Usage (through just):
  just ci-local                  # every job on HEAD: lint, test, scenarios, timings
  just ci-local lint             # one or more jobs; nothing is posted
  just ci-local --full           # every job, even for a change that touches only docs
  just ci-local --cancel         # stop the run started from this worktree, queued or running
  just ci-local --rebuild-image  # throw the image away and build it again

GitHub runs the full CI only on pushes to main (ticket #158). Everywhere
else this is CI: run it before every push. It runs the same `just` recipes
as .github/workflows/ci.yml, in the same base image (archlinux:latest, with
the packages the workflow installs), unprivileged: a user namespace and a
chroot, no sudo, no container engine.

What it does, in order:
  1. Refuses unless the working tree is clean: it tests the commit HEAD, and
     the status it posts says that commit passed.
  2. Takes a lock, so runs from several agents take turns. A run waiting
     for it tests HEAD as it is when its turn comes, so committing while
     queued is fine. A worktree has one run at a time: a second is refused,
     naming the first, and --cancel stops the first (and only it).
  3. Removes /tmp/desktop-calls-test-* folders of yours that are over an
     hour old (the harness leaks them).
  4. Builds the image the first time (a few minutes, 2 GB), and again once it
     is two weeks old, as CI's is new each run. Refuses to run with
     DESKTOP_CALLS_OFFLINE set, which would skip lint's ticket check.
  5. Checks out HEAD into a scratch directory and runs each job in the
     image, under nice (the timings, below, excepted), with a private /tmp. The jobs that run tests have
     no network, and nothing reaches the host's D-Bus, PipeWire, Bluetooth
     or home, so a run can't touch the real Phone. Before each job that runs
     tests in parallel it measures how much of the Desktop other programs
     (agents' builds) are using, and runs that many tests at a time and no
     more: all the CPUs on a quiet Desktop, down to MIN_THREADS on a busy one
     (CI's runners have two CPUs). The first-frame timings wait, for up to
     CALM_WAIT_SECONDS, until others use few CPUs, then run at normal
     priority, since under nice they wait behind every build. Each job's last
     line says how much others used while it ran, and a failure says whether
     that was enough to blame (ticket #167).
  6. If every job passed and HEAD is still the commit tested, pushes the
     branch if GitHub doesn't have the commit yet (a status needs a commit
     to sit on), and posts the commit status "CI passed" (state success).
     If a job failed, posts "CI passed" with state failure. Branch
     protection requires that context, and `just merge` reads it.

Two shortcuts, both said in the log and the status:
  - Docs only: when HEAD contains origin/main and tools/changes.py says the
    change from origin/main to HEAD touches only docs that no code names, the
    run is lint alone, and a pass posts "CI passed" as usual. GitHub CI skips
    the test jobs for such a push through the same script. It is safe because
    a squash merge keeps the tree of the head that passed. If main has moved
    with code since, the head no longer contains origin/main: `just merge`
    sends the PR back to be brought up to date, and the new run, whose change
    from origin/main is docs only again, is lint alone as well, while that
    code is already tested on main. --full runs every job regardless.
  - A failed lint ends the run: the other jobs would test a commit that must
    change before it can pass.

The cache, image and logs are under ~/.cache/desktop-calls-ci, never /tmp,
which has a per-user quota. Each run's log is in logs/. A failed run keeps
its /tmp (the failed scenarios' directories) in runs/.

Exit status: 0 every job passed and the status was posted; 1 a job failed;
2 refused, or could not run or post.
"""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import pathlib
import re
import resource
import shutil
import signal
import subprocess
import sys
import time
import urllib.request
from datetime import datetime, timezone

import changes  # the docs-only rule that CI's "Code changed?" job and `just merge` share

EXIT_PASSED, EXIT_FAILED, EXIT_REFUSED = 0, 1, 2

REPO = "danielbaldwin47/desktop-calls"
# The context branch protection requires, with no app pinned.
CONTEXT = "CI passed"

HERE = pathlib.Path(__file__).resolve().parent
LOCAL_CI = HERE / "local-ci"
CACHE_HOME = pathlib.Path(os.environ.get("XDG_CACHE_HOME") or pathlib.Path.home() / ".cache")
ROOT = CACHE_HOME / "desktop-calls-ci"
ROOTFS = ROOT / "rootfs"
LAYERS = ROOT / "layers"
CACHE = ROOT / "cache"
LOGS = ROOT / "logs"
RUNS = ROOT / "runs"
LOCK = ROOT / "lock"
BUILT = ROOT / "image-built"
# One file per worktree with a run queued or running: its process ID and
# state. A file whose process is gone is stale and ignored.
CLAIMS = ROOT / "claims"
# How long --cancel waits for the stopped run to exit.
CANCEL_SECONDS = 20

# How old the image may get before a run builds it again. CI installs the
# newest packages on every run, so an old image drifts from it.
IMAGE_STALE_DAYS = 14
# Set, it makes the ticket check in lint skip itself (tools/checklist_tickets.py),
# which a run that posts "CI passed" must not.
OFFLINE = "DESKTOP_CALLS_OFFLINE"
# How long logs and failed runs' /tmp are kept.
KEEP_DAYS = 14
# A /tmp/desktop-calls-test-* folder this old belongs to no running test.
STALE_TEST_FOLDER_MINUTES = 60
STALE_TEST_FOLDER = "desktop-calls-test-*"

# Each job is the command CI's job runs, from a `just` recipe, and whether it
# runs with the host's network. tools/ci_local_test.py checks that
# .github/workflows/ci.yml runs every one of these commands.
#
#   lint       CI job "lint"
#   test       CI job "check": the tests but the harness's scenarios, and the
#              scorecard (the gallery test renders the gallery on the way)
#   scenarios  CI job "scenarios": every scenario, in one run rather than
#              four shards
#   timings    the rest of CI job "scenarios": the first-frame timings one at
#              a time, so that no other scenario's app slows a first paint
JOBS: dict[str, str] = {
    "lint": "just lint",
    "test": "just test --exclude test-harness && just scorecard",
    "scenarios": "just ci-scenarios",
    "timings": "just ci-first-frame-timings",
}
# Jobs that run tests in parallel, so are held to the Desktop's spare CPUs.
PARALLEL_JOBS = {"test", "scenarios"}
# A failure here ends the run: the fix is a new commit and a new run, so the
# other jobs would only spend minutes on a commit that will not be posted.
FAIL_FAST_JOBS = {"lint"}
# Jobs whose timings are the test: they run at normal priority, one at a time.
UNHURRIED_JOBS = {"timings"}
# Lint's `cargo deny` fetches the advisory database, and its ticket check asks
# GitHub, anonymously (the repository is public). The others build with the crates fetched beforehand, and run
# scenarios that must not see the host's network.
JOBS_WITH_NETWORK = {"lint"}
# Never fewer tests at a time than this, which is twice what a CI runner runs
# (it has two CPUs): the least that still finishes in a few minutes.
MIN_THREADS = 4
# How long to watch what others are using before a job.
SAMPLE_SECONDS = 3
# Others using at least this share of the CPUs while a job ran make a timing
# failure likely to be theirs.
BUSY_SHARE = 0.4
# The timings judge milliseconds of one app's first paint and keystrokes, which
# a build's bursts stretch (a Dialer keystroke is 12 to 25 ms, and 50 to 56 ms
# beside a build). They start once others use at most this share of the CPUs,
# or after this long, if the Desktop doesn't calm down before.
CALM_SHARE = 0.2
CALM_WAIT_SECONDS = 120
# A build comes in bursts with quiet between: calm is this many looks in a row,
# each CALM_LOOK_SECONDS long.
CALM_LOOKS = 3
CALM_LOOK_SECONDS = 2
# Before any job, with the network: the pinned toolchain (rust-toolchain.toml)
# and the crates.
PREPARE = "rustup toolchain install && cargo fetch --locked"


class Refused(Exception):
    """The run can't start, or its result can't be posted. The message says
    what to do next."""


def git(*args: str) -> str:
    result = subprocess.run(["git", *args], capture_output=True, text=True)
    if result.returncode != 0:
        raise Refused(f"`git {' '.join(args)}` failed: {result.stderr.strip()}")
    return result.stdout.strip()


def head_and_cleanliness() -> tuple[str, str]:
    """The commit checked out, and `git status --porcelain` (empty if clean)."""
    return git("rev-parse", "HEAD"), git("status", "--porcelain")


def require_clean_head() -> str:
    sha, dirty = head_and_cleanliness()
    if dirty:
        raise Refused(
            "The working tree has uncommitted changes, so a run would test "
            f"{sha[:7]} while the status described something else:\n{dirty}\n"
            "Next: commit or discard them, then just ci-local"
        )
    return sha


# -- The lock, the folders, the log ------------------------------------------


def worktree() -> str:
    """The root of the checkout this run was started from."""
    return os.path.realpath(git("rev-parse", "--show-toplevel"))


def is_ci_local(pid: int) -> bool:
    """Whether `pid` is a live ci-local run: a process ID alone may have been
    reused by anything since the run that held it ended."""
    try:
        command = pathlib.Path(f"/proc/{pid}/cmdline").read_bytes()
    except OSError:
        return False
    return b"ci_local.py" in command


class Claim:
    """A worktree's one queued or running ci-local run, in CLAIMS."""

    def __init__(self, directory: str) -> None:
        self.directory = directory
        digest = hashlib.sha256(directory.encode()).hexdigest()[:16]
        self.path = CLAIMS / f"{digest}.json"

    def read(self) -> dict | None:
        """The live run's record, or None if there is none (a stale file is removed)."""
        try:
            record = json.loads(self.path.read_text())
            if is_ci_local(int(record["pid"])):
                return record
        except (OSError, ValueError, KeyError, TypeError):
            pass
        self.path.unlink(missing_ok=True)
        return None

    def write(self, state: str, sha: str) -> None:
        record = {
            "pid": os.getpid(), "worktree": self.directory, "sha": sha, "state": state,
            "since": datetime.now(timezone.utc).strftime("%H:%M:%S UTC"),
        }
        temporary = self.path.with_suffix(f".{os.getpid()}")
        temporary.write_text(json.dumps(record))
        temporary.replace(self.path)

    def take(self, sha: str) -> None:
        """Records this run as the worktree's, or refuses naming the one that is."""
        CLAIMS.mkdir(parents=True, exist_ok=True)
        first = self.read()
        if first and first["pid"] != os.getpid():
            raise Refused(
                f"This worktree already has a ci-local run: process {first['pid']}, "
                f"{first['state']} since {first['since']}, on {first['sha'][:7]}. "
                "It tests HEAD as it is when its turn comes, so commit what you like.\n"
                "Next: leave it to finish, or just ci-local --cancel to stop it"
            )
        self.write("waiting", sha)

    def release(self) -> None:
        record = self.read()
        if record and record["pid"] == os.getpid():
            self.path.unlink(missing_ok=True)


def cancel(directory: str) -> int:
    """Stops the run started from this worktree, and no other."""
    record = Claim(directory).read()
    if not record:
        print("No ci-local run is queued or running from this worktree.")
        return EXIT_PASSED
    pid = int(record["pid"])
    # SIGTERM takes the run's whole tree down with it (stop_on_sigterm).
    os.kill(pid, signal.SIGTERM)
    deadline = time.monotonic() + CANCEL_SECONDS
    while is_ci_local(pid) and time.monotonic() < deadline:
        time.sleep(0.2)
    if is_ci_local(pid):
        print(f"Sent the run (process {pid}) SIGTERM, but it is still stopping.", file=sys.stderr)
        return EXIT_REFUSED
    print(f"Stopped the {record['state']} run (process {pid}) on {record['sha'][:7]}.")
    return EXIT_PASSED


def running_elsewhere() -> str:
    """Who has the lock, for the message of a run that must wait."""
    for path in sorted(CLAIMS.glob("*.json")) if CLAIMS.is_dir() else []:
        try:
            record = json.loads(path.read_text())
            if record["state"] == "running" and is_ci_local(int(record["pid"])):
                return f" (the run from {record['worktree']}, process {record['pid']})"
        except (OSError, ValueError, KeyError, TypeError):
            continue
    return ""


def take_turn() -> object:
    """Holds the lock until the process exits, and says so if it must wait."""
    ROOT.mkdir(parents=True, exist_ok=True)
    handle = open(LOCK, "w")
    try:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        print(
            f"Another ci-local run has the lock{running_elsewhere()}; waiting for it to finish. "
            "This run tests HEAD as it is when its turn comes, so committing while it waits is "
            "fine. To stop it: just ci-local --cancel, in this worktree.",
            flush=True,
        )
        fcntl.flock(handle, fcntl.LOCK_EX)
    return handle


def remove_tree(path: pathlib.Path) -> None:
    """Removes a directory a run filled. Files made by other IDs in the
    user namespace belong to the subuid range, which only a namespace that
    maps it can remove."""
    if path.exists():
        subprocess.run(
            ["unshare", "--map-auto", "--map-root-user", "rm", "-rf", str(path)],
            check=False,
        )


def clear_stale_test_folders(
    now: float | None = None, parent: pathlib.Path = pathlib.Path("/tmp")
) -> list[pathlib.Path]:
    """Removes this user's /tmp/desktop-calls-test-* folders over an hour old."""
    now = time.time() if now is None else now
    removed = []
    for folder in parent.glob(STALE_TEST_FOLDER):
        try:
            info = folder.lstat()
        except OSError:
            continue
        old = now - info.st_mtime > STALE_TEST_FOLDER_MINUTES * 60
        if old and info.st_uid == os.getuid() and folder.is_dir() and not folder.is_symlink():
            shutil.rmtree(folder, ignore_errors=True)
            removed.append(folder)
    return removed


def prune_old_runs(now: float | None = None) -> None:
    now = time.time() if now is None else now
    for parent in (LOGS, RUNS):
        for entry in parent.glob("*") if parent.is_dir() else []:
            if now - entry.stat().st_mtime > KEEP_DAYS * 86400:
                if entry.is_dir():
                    remove_tree(entry)
                else:
                    entry.unlink()


class Log:
    """Everything the run prints, on the screen and in the run's log file."""

    def __init__(self, sha: str) -> None:
        LOGS.mkdir(parents=True, exist_ok=True)
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        self.name = f"{stamp}-{sha[:7]}"
        self.path = LOGS / f"{self.name}.log"
        self._file = open(self.path, "w")

    def line(self, text: str) -> None:
        print(text, flush=True)
        self._file.write(text + "\n")
        self._file.flush()

    def stream(self, command: list[str], env: dict[str, str] | None = None) -> int:
        """Runs a command, copying its output as it comes. Returns its exit status."""
        process = subprocess.Popen(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            env=env,
            stdin=subprocess.DEVNULL,
        )
        assert process.stdout is not None
        try:
            for raw in process.stdout:
                self.line(raw.decode(errors="replace").rstrip("\n"))
        except KeyboardInterrupt:
            # SIGINT or SIGTERM (see main). unshare's --kill-child takes the
            # whole tree of the run down with it.
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            process.stdout.close()
            raise
        return process.wait()

    def display_path(self) -> str:
        """The log's path with ~ for the home directory: a status is public."""
        try:
            return "~/" + str(self.path.relative_to(pathlib.Path.home()))
        except ValueError:
            return str(self.path)


# -- The image ----------------------------------------------------------------


def registry_get(url: str, token: str, accept: str) -> bytes:
    request = urllib.request.Request(
        url, headers={"Authorization": f"Bearer {token}", "Accept": accept}
    )
    with urllib.request.urlopen(request, timeout=120) as response:
        return response.read()


def pull_base_layers(destination: pathlib.Path, log: Log) -> None:
    """Downloads archlinux:latest's layers from Docker Hub (CI's base image),
    in order, as 00.tar, 01.tar, ..."""
    registry = "https://registry-1.docker.io/v2/library/archlinux"
    token_url = (
        "https://auth.docker.io/token?service=registry.docker.io"
        "&scope=repository:library/archlinux:pull"
    )
    with urllib.request.urlopen(token_url, timeout=60) as response:
        token = json.load(response)["token"]
    index = json.loads(
        registry_get(f"{registry}/manifests/latest", token, "application/vnd.oci.image.index.v1+json")
    )
    digest = next(
        entry["digest"]
        for entry in index["manifests"]
        if entry["platform"]["architecture"] == "amd64" and entry["platform"]["os"] == "linux"
    )
    manifest = json.loads(
        registry_get(f"{registry}/manifests/{digest}", token, "application/vnd.oci.image.manifest.v1+json")
    )
    destination.mkdir(parents=True)
    for number, layer in enumerate(manifest["layers"]):
        log.line(f"Pulling archlinux:latest layer {number + 1} of {len(manifest['layers'])}...")
        request = urllib.request.Request(
            f"{registry}/blobs/{layer['digest']}", headers={"Authorization": f"Bearer {token}"}
        )
        with urllib.request.urlopen(request, timeout=300) as response:
            with open(destination / f"{number:02}.tar", "wb") as out:
                shutil.copyfileobj(response, out)


def userns(*flags: str, polite: bool = True) -> list[str]:
    """`nice` (unless not `polite`), then `unshare` with a user namespace
    mapped onto the subuid range."""
    return [
        *(["nice", "-n", "19", "ionice", "-c3"] if polite else []),
        "unshare", "--map-auto", "--map-root-user", "--mount", "--pid", "--fork", "--kill-child",
        *flags, "--",
    ]


def image_age_days(now: float | None = None) -> float:
    now = time.time() if now is None else now
    return (now - BUILT.stat().st_mtime) / 86400 if BUILT.exists() else 0


def build_image(log: Log, rebuild: bool) -> None:
    """Builds the image if there is none, when asked to, or when it is more
    than IMAGE_STALE_DAYS old: CI's image is new each run."""
    if ROOTFS.exists() and image_age_days() > IMAGE_STALE_DAYS:
        log.line(f"The image is {image_age_days():.0f} days old; building it again.")
        rebuild = True
    if ROOTFS.exists() and rebuild:
        log.line("Removing the old image...")
        remove_tree(ROOTFS)
    if ROOTFS.exists():
        return
    started = time.monotonic()
    log.line("Building the image (first run, or an old one; a few minutes)...")
    partial = ROOT / "rootfs.partial"
    remove_tree(partial)
    remove_tree(LAYERS)
    pull_base_layers(LAYERS, log)
    status = log.stream(userns() + [str(LOCAL_CI / "build-rootfs.sh"), str(partial), str(LAYERS)])
    if status != 0:
        raise Refused(
            f"Building the image failed (exit {status}); the log is {log.display_path()}.\n"
            "Next: check that /etc/subuid has a range for you and the network works, "
            "then just ci-local --rebuild-image"
        )
    partial.rename(ROOTFS)
    BUILT.touch()
    remove_tree(LAYERS)
    log.line(f"== image: {time.monotonic() - started:.0f} s")


# -- The jobs -----------------------------------------------------------------


def checkout(sha: str, scratch: pathlib.Path) -> None:
    """A clone of this repository at the commit, so git works inside the run
    as in CI, and the run tests exactly that commit's files."""
    git("clone", "--quiet", "--no-checkout", git("rev-parse", "--show-toplevel"), str(scratch))
    # The clone's origin is this checkout; the checks that ask GitHub which
    # repository this is (the Real-Phone Checklist's tickets) need the real one.
    subprocess.run(
        ["git", "-C", str(scratch), "remote", "set-url", "origin", git("remote", "get-url", "origin")],
        check=True,
    )
    subprocess.run(["git", "-C", str(scratch), "checkout", "--quiet", "--detach", sha], check=True)


def owner_and_name(url: str) -> str:
    """`owner/name` from a GitHub remote's URL, as GITHUB_REPOSITORY has it."""
    match = re.search(r"github\.com[:/]([^/]+/[^/]+?)(?:\.git)?$", url)
    if not match:
        raise Refused(f"origin ({url}) isn't a GitHub repository.\nNext: git remote set-url origin <its URL>")
    return match.group(1)


def run_in_image(
    log: Log,
    scratch: pathlib.Path,
    tmp: pathlib.Path,
    command: str,
    network: bool,
    threads: int | None = None,
    polite: bool = True,
) -> int:
    """Runs `command` in the image. With `threads`, the test runners (cargo
    test, nextest, the gallery) run that many tests at a time."""
    flags = [] if network else ["--net"]
    # GitHub Actions sets this in every step. The checkout there belongs to
    # another user, so git refuses it and a check can't read origin's URL
    # (enter.sh hands the checkout to another user here too).
    command = f"export GITHUB_REPOSITORY={owner_and_name(git('remote', 'get-url', 'origin'))} && {command}"
    if threads is not None:
        command = f"export RUST_TEST_THREADS={threads} NEXTEST_TEST_THREADS={threads} && {command}"
    CACHE.mkdir(parents=True, exist_ok=True)
    return log.stream(
        userns(*flags, polite=polite)
        + [str(LOCAL_CI / "enter.sh"), str(ROOTFS), str(CACHE), str(scratch), str(tmp), command],
    )


# -- What the Desktop has to spare --------------------------------------------


def cpu_seconds() -> tuple[float, float]:
    """The CPU time every CPU has spent working, and in all, since boot
    (/proc/stat's first line: user nice system idle iowait irq softirq steal)."""
    fields = [int(field) for field in pathlib.Path("/proc/stat").read_text().split()[1:9]]
    ticks = os.sysconf("SC_CLK_TCK")
    total = sum(fields)
    return (total - fields[3] - fields[4]) / ticks, total / ticks


def children_cpu_seconds() -> float:
    """The CPU time this run's finished jobs spent."""
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    return usage.ru_utime + usage.ru_stime


def watch_others(seconds: float = SAMPLE_SECONDS) -> float:
    """How many CPUs' worth other programs use, over `seconds`. Run it
    between jobs, when this run uses none."""
    busy, total = cpu_seconds()
    time.sleep(seconds)
    busy_after, total_after = cpu_seconds()
    return (busy_after - busy) / (total_after - total) * (os.cpu_count() or 1)


def threads_for(others: float, cpus: int) -> int:
    """How many tests to run at a time: the CPUs left once others have what
    they use and the same again as headroom, at least MIN_THREADS and at
    most one per CPU. A quiet Desktop gets all its CPUs, as before; a build
    using half of them gets MIN_THREADS."""
    return min(cpus, max(MIN_THREADS, int(cpus - 2 * others)))


class Usage:
    """The CPU time spent by everything on the machine and by this run's
    jobs from its creation: what is left over is what others used."""

    def __init__(self) -> None:
        self.busy, _ = cpu_seconds()
        self.ours = children_cpu_seconds()
        self.started = time.monotonic()

    def others_cpus(self) -> float:
        """How many CPUs' worth others used, on average, since creation."""
        busy, _ = cpu_seconds()
        elapsed = max(time.monotonic() - self.started, 0.001)
        return max(0.0, busy - self.busy - (children_cpu_seconds() - self.ours)) / elapsed


def wait_for_calm(log: Log, cpus: int, give_up_after: float = CALM_WAIT_SECONDS) -> None:
    """Returns once others have used at most CALM_SHARE of the CPUs for
    CALM_LOOKS looks in a row, or after `give_up_after` seconds, saying which."""
    started = time.monotonic()
    announced = False
    calm_looks = 0
    while True:
        others = watch_others(CALM_LOOK_SECONDS)
        calm_looks = calm_looks + 1 if others <= CALM_SHARE * cpus else 0
        if calm_looks == CALM_LOOKS:
            if announced:
                log.line(f"   calm after {time.monotonic() - started:.0f} s")
            return
        if calm_looks:
            continue
        if time.monotonic() - started >= give_up_after:
            log.line(
                f"   not calm after {give_up_after:.0f} s (others use about {others:.0f} of "
                f"{cpus} CPUs): running anyway"
            )
            return
        if not announced:
            log.line(
                f"   waiting for others to use at most {CALM_SHARE * cpus:.0f} of {cpus} CPUs "
                f"(now about {others:.0f}), for up to {give_up_after:.0f} s"
            )
            announced = True


def blame(others: float, cpus: int, failed: bool) -> str:
    """What a job's last line says about the load it ran under."""
    used = f"other programs used about {others:.0f} of {cpus} CPUs while it ran"
    if not failed:
        return used
    if others >= BUSY_SHARE * cpus:
        return (
            f"{used}: a timeout or a late reply may be the machine's, not the change's. "
            "Read the failing test's log first; rerunning until it passes would hide a real flake"
        )
    return f"{used}: not enough to blame, so treat the failure as the change's"


def run_jobs(log: Log, sha: str, jobs: list[str]) -> tuple[list[str], pathlib.Path]:
    """Runs the jobs in order, and returns the names of those that failed,
    and the directory that was their /tmp."""
    RUNS.mkdir(parents=True, exist_ok=True)
    scratch = RUNS / f"{log.name}-src"
    tmp = RUNS / log.name
    tmp.mkdir()
    checkout(sha, scratch)
    failed = []
    try:
        started = time.monotonic()
        log.line(f"== prepare: {PREPARE}")
        if run_in_image(log, scratch, tmp, PREPARE, network=True) != 0:
            raise Refused(
                f"Preparing the toolchain and crates failed; the log is {log.display_path()}.\n"
                "Next: check the network, then just ci-local"
            )
        log.line(f"== prepare: {time.monotonic() - started:.0f} s")
        cpus = os.cpu_count() or 1
        for job in jobs:
            threads = None
            log.line(f"== {job}: {JOBS[job]}")
            if job in PARALLEL_JOBS:
                others = watch_others()
                threads = threads_for(others, cpus)
            if job in UNHURRIED_JOBS:
                wait_for_calm(log, cpus)
            if threads is not None:
                log.line(
                    f"   {threads} at a time: other programs are using about {others:.0f} of "
                    f"{cpus} CPUs"
                )
            started = time.monotonic()
            usage = Usage()
            status = run_in_image(
                log, scratch, tmp, JOBS[job],
                network=job in JOBS_WITH_NETWORK,
                threads=threads,
                polite=job not in UNHURRIED_JOBS,
            )
            if status != 0:
                failed.append(job)
            verdict = "passed" if status == 0 else f"FAILED (exit {status})"
            log.line(f"== {job}: {verdict}, {time.monotonic() - started:.0f} s")
            log.line(f"   {blame(usage.others_cpus(), cpus, status != 0)}")
            if status != 0 and job in FAIL_FAST_JOBS:
                skipped = jobs[jobs.index(job) + 1:]
                if skipped:
                    log.line(f"== {job} failed, so {', '.join(skipped)} did not run.")
                break
    finally:
        remove_tree(scratch)
    return failed, tmp


# -- The status ---------------------------------------------------------------


def github_has(sha: str) -> bool:
    result = subprocess.run(
        ["gh", "api", f"repos/{REPO}/commits/{sha}", "--silent"], capture_output=True, text=True
    )
    return result.returncode == 0


def push_branch(sha: str, log: Log) -> None:
    """A status can only sit on a commit GitHub has, so push the branch, as
    `git push` would. Never main, never a force."""
    branch = git("branch", "--show-current")
    if not branch or branch == "main":
        raise Refused(
            f"{sha[:7]} passed, but GitHub doesn't have it and "
            f"{'HEAD is detached' if not branch else 'main is only pushed by a merge'}, "
            "so there is nowhere to push it.\n"
            "Next: switch to a branch, push it, then just ci-local"
        )
    log.line(f"GitHub doesn't have {sha[:7]} yet: pushing {branch}.")
    # The commit that was tested, not whatever the branch points at now.
    result = subprocess.run(
        ["git", "push", "origin", f"{sha}:refs/heads/{branch}"], capture_output=True, text=True
    )
    log.line((result.stdout + result.stderr).strip())
    if result.returncode != 0:
        raise Refused(
            f"{sha[:7]} passed, but pushing {branch} failed, so no status was posted.\n"
            "Next: merge origin/main into the branch (or fix the push), then just ci-local"
        )
    # So `git status` and a bare `git push` know the branch, as after `git push -u`.
    subprocess.run(
        ["git", "branch", "--set-upstream-to", f"origin/{branch}", branch], capture_output=True
    )


def status_description(
    state: str, seconds: float, failed: list[str], log: Log, docs_only: bool = False
) -> str:
    what = "passed" if state == "success" else f"failed: {', '.join(failed)}"
    if docs_only:
        what += " (docs only, lint alone)"
    return f"just ci-local {what} in {seconds:.0f} s; log {log.display_path()}"[:140]


def post_status(sha: str, state: str, description: str) -> None:
    result = subprocess.run(
        [
            "gh", "api", f"repos/{REPO}/statuses/{sha}",
            "-f", f"state={state}", "-f", f"context={CONTEXT}", "-f", f"description={description}",
        ],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        raise Refused(
            f"Posting the status on {sha[:7]} failed: {result.stderr.strip()}\n"
            "Next: gh auth status, then just ci-local"
        )


def require_online() -> None:
    if os.environ.get(OFFLINE):
        raise Refused(
            f"{OFFLINE} is set, which makes lint skip its ticket check, so this run "
            "can't say \"CI passed\".\n"
            f"Next: unset {OFFLINE}, then just ci-local"
        )


def settle(sha: str, failed: list[str], seconds: float, log: Log, docs_only: bool = False) -> int:
    """Posts the result, if HEAD is still the commit that was tested."""
    require_online()
    now, dirty = head_and_cleanliness()
    if now != sha or dirty:
        raise Refused(
            f"HEAD or the working tree changed during the run (tested {sha[:7]}, now {now[:7]}"
            f"{', with uncommitted changes' if dirty else ''}), so nothing was posted.\n"
            "Next: just ci-local"
        )
    state = "failure" if failed else "success"
    if not github_has(sha):
        if failed:
            log.line(f"{', '.join(failed)} failed, and GitHub doesn't have {sha[:7]}: nothing posted.")
            return EXIT_FAILED
        push_branch(sha, log)
    post_status(sha, state, status_description(state, seconds, failed, log, docs_only))
    if failed:
        log.line(f"{CONTEXT}: failure posted on {sha[:7]} ({', '.join(failed)} failed).")
        return EXIT_FAILED
    log.line(f"{CONTEXT}: success posted on {sha[:7]}.")
    return EXIT_PASSED


# -- Main ---------------------------------------------------------------------


def is_docs_only(sha: str, log: Log) -> bool:
    """Whether the change from origin/main to `sha` touches only docs that no
    code names, which lint alone tests. Anything unclear says no, and says why."""
    if subprocess.run(["git", "fetch", "--quiet", "origin", "main"], capture_output=True).returncode != 0:
        log.line("Couldn't fetch origin/main, so every job runs.")
        return False
    base = git("rev-parse", "origin/main")
    contains = subprocess.run(["git", "merge-base", "--is-ancestor", base, sha], capture_output=True)
    if contains.returncode != 0:
        log.line(f"{sha[:7]} doesn't contain origin/main ({base[:7]}), so every job runs.")
        return False
    try:
        code, reason = changes.code_changes(changes.run_git, base, sha)
    except RuntimeError as error:
        log.line(f"Couldn't compare with origin/main ({error}), so every job runs.")
        return False
    log.line(reason)
    return not code


def stop_on_sigterm() -> None:
    """Treats SIGTERM like Ctrl-C, so a killed run takes its `unshare` tree
    down with it instead of leaving it running under the lock's name."""

    def handler(signum: int, frame: object) -> None:
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, handler)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run CI's jobs here, and post 'CI passed' on HEAD if they pass"
    )
    parser.add_argument("jobs", nargs="*", choices=list(JOBS), metavar="JOB",
                        help=f"one of {', '.join(JOBS)}; default all, which alone posts a status")
    parser.add_argument("--rebuild-image", action="store_true",
                        help="remove the image and build it again from the newest archlinux:latest")
    parser.add_argument("--cancel", action="store_true",
                        help="stop the run started from this worktree, queued or running, and no other")
    parser.add_argument("--full", action="store_true",
                        help="run every job even when the change touches only docs")
    args = parser.parse_args()
    jobs = args.jobs or list(JOBS)
    every_job = not args.jobs or set(args.jobs) == set(JOBS)
    docs_only = False
    claim = None
    try:
        if args.cancel:
            return cancel(worktree())
        stop_on_sigterm()
        if every_job:
            require_online()
        sha = require_clean_head()
        claim = Claim(worktree())
        claim.take(sha)
        lock = take_turn()  # held until the process exits
        claim.write("running", sha)
        log = Log(sha)
        log.line(f"ci-local on {sha[:7]}: {', '.join(jobs)}; log {log.display_path()}")
        removed = clear_stale_test_folders()
        if removed:
            log.line(f"Removed {len(removed)} stale /tmp/{STALE_TEST_FOLDER} folders.")
        prune_old_runs()
        # HEAD may have moved while this waited for its turn.
        sha = require_clean_head()
        if every_job and not args.full and is_docs_only(sha, log):
            docs_only, jobs = True, ["lint"]
            log.line("Docs only: lint alone, as GitHub CI skips the test jobs for such a change.")
        build_image(log, args.rebuild_image)
        started = time.monotonic()
        failed, tmp = run_jobs(log, sha, jobs)
        seconds = time.monotonic() - started
        log.line(f"== total: {seconds:.0f} s")
        if failed:
            log.line(f"Failed scenarios' directories are in {tmp}")
        else:
            remove_tree(tmp)
        if not every_job:
            log.line("Only some jobs ran, so no status was posted.")
            return EXIT_FAILED if failed else EXIT_PASSED
        return settle(sha, failed, seconds, log, docs_only)
    except Refused as refusal:
        print(refusal, file=sys.stderr)
        return EXIT_REFUSED
    except KeyboardInterrupt:
        print("Interrupted; nothing was posted.", file=sys.stderr)
        return EXIT_REFUSED
    finally:
        if claim:
            claim.release()


if __name__ == "__main__":
    sys.exit(main())
