#!/usr/bin/env python3
"""Run CI's jobs on this machine, and post "CI passed" on the commit if they pass.

Usage (through just):
  just ci-local                  # every job on HEAD: lint, test, scenarios
  just ci-local lint             # one or more jobs; nothing is posted
  just ci-local --rebuild-image  # throw the image away and build it again

GitHub runs the full CI only on pushes to main (ticket #158). Everywhere
else this is CI: run it before every push. It runs the same `just` recipes
as .github/workflows/ci.yml, in the same base image (archlinux:latest, with
the packages the workflow installs), unprivileged: a user namespace and a
chroot, no sudo, no container engine.

What it does, in order:
  1. Refuses unless the working tree is clean: it tests the commit HEAD, and
     the status it posts says that commit passed.
  2. Takes a lock, so runs from several agents take turns.
  3. Removes /tmp/desktop-calls-test-* folders of yours that are over an
     hour old (the harness leaks them).
  4. Builds the image the first time (a few minutes, 2 GB).
  5. Checks out HEAD into a scratch directory and runs each job in the
     image, under nice, with a private /tmp. The jobs that run tests have
     no network, and nothing reaches the host's D-Bus, PipeWire, Bluetooth
     or home, so a run can't touch the real Phone.
  6. If every job passed and HEAD is still the commit tested, pushes the
     branch if GitHub doesn't have the commit yet (a status needs a commit
     to sit on), and posts the commit status "CI passed" (state success).
     If a job failed, posts "CI passed" with state failure. Branch
     protection requires that context, and `just merge` reads it.

The cache, image and logs are under ~/.cache/desktop-calls-ci, never /tmp,
which has a per-user quota. Each run's log is in logs/. A failed run keeps
its /tmp (the failed scenarios' directories) in runs/.

Exit status: 0 every job passed and the status was posted; 1 a job failed;
2 refused, or could not run or post.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time
import urllib.request
from datetime import datetime, timezone

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

# How old the image may get before a run says so. CI installs the newest
# packages on every run, so an old image drifts from it.
IMAGE_STALE_DAYS = 14
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
#              four shards, then the first-frame timings one at a time
JOBS: dict[str, str] = {
    "lint": "just lint",
    "test": "just test --exclude test-harness && just scorecard",
    "scenarios": "just ci-scenarios && just ci-first-frame-timings",
}
# Lint's `cargo deny` fetches the advisory database, and its ticket check asks
# GitHub. The others build with the crates fetched beforehand, and run
# scenarios that must not see the host's network.
JOBS_WITH_NETWORK = {"lint"}
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


def take_turn() -> object:
    """Holds the lock until the process exits, and says so if it must wait."""
    ROOT.mkdir(parents=True, exist_ok=True)
    handle = open(LOCK, "w")
    try:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        print("Another ci-local run has the lock; waiting for it to finish...", flush=True)
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
            process.terminate()
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


def userns(*flags: str) -> list[str]:
    """`nice`, then `unshare` with a user namespace mapped onto the subuid range."""
    return [
        "nice", "-n", "19", "ionice", "-c3",
        "unshare", "--map-auto", "--map-root-user", "--mount", "--pid", "--fork", "--kill-child",
        *flags, "--",
    ]


def build_image(log: Log, rebuild: bool) -> None:
    """Builds the image if there is none (or when asked to), and warns if an old one."""
    if ROOTFS.exists() and rebuild:
        log.line("Removing the old image...")
        remove_tree(ROOTFS)
    if ROOTFS.exists():
        age_days = (time.time() - BUILT.stat().st_mtime) / 86400 if BUILT.exists() else 0
        if age_days > IMAGE_STALE_DAYS:
            log.line(
                f"The image is {age_days:.0f} days old; CI's is new each run. "
                "Next: just ci-local --rebuild-image"
            )
        return
    started = time.monotonic()
    log.line("Building the image (first run, or after --rebuild-image; a few minutes)...")
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
    subprocess.run(["git", "-C", str(scratch), "checkout", "--quiet", "--detach", sha], check=True)


def run_in_image(
    log: Log, scratch: pathlib.Path, tmp: pathlib.Path, command: str, network: bool
) -> int:
    flags = [] if network else ["--net"]
    CACHE.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ)
    env.pop("GH_TOKEN", None)
    if network and token():
        env["GH_TOKEN"] = token()
    return log.stream(
        userns(*flags)
        + [str(LOCAL_CI / "enter.sh"), str(ROOTFS), str(CACHE), str(scratch), str(tmp), command],
        env=env,
    )


_token: list[str] = []


def token() -> str:
    """GitHub's token, for lint's ticket check, which falls back to anonymous."""
    if not _token:
        result = subprocess.run(["gh", "auth", "token"], capture_output=True, text=True)
        _token.append(result.stdout.strip() if result.returncode == 0 else "")
    return _token[0]


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
        for job in jobs:
            started = time.monotonic()
            log.line(f"== {job}: {JOBS[job]}")
            status = run_in_image(log, scratch, tmp, JOBS[job], network=job in JOBS_WITH_NETWORK)
            if status != 0:
                failed.append(job)
            verdict = "passed" if status == 0 else f"FAILED (exit {status})"
            log.line(f"== {job}: {verdict}, {time.monotonic() - started:.0f} s")
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
    result = subprocess.run(
        ["git", "push", "--set-upstream", "origin", f"{branch}:{branch}"],
        capture_output=True, text=True,
    )
    log.line((result.stdout + result.stderr).strip())
    if result.returncode != 0:
        raise Refused(
            f"{sha[:7]} passed, but pushing {branch} failed, so no status was posted.\n"
            "Next: merge origin/main into the branch (or fix the push), then just ci-local"
        )


def status_description(state: str, seconds: float, failed: list[str], log: Log) -> str:
    what = "passed" if state == "success" else f"failed: {', '.join(failed)}"
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


def settle(sha: str, failed: list[str], seconds: float, log: Log) -> int:
    """Posts the result, if HEAD is still the commit that was tested."""
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
    post_status(sha, state, status_description(state, seconds, failed, log))
    if failed:
        log.line(f"{CONTEXT}: failure posted on {sha[:7]} ({', '.join(failed)} failed).")
        return EXIT_FAILED
    log.line(f"{CONTEXT}: success posted on {sha[:7]}.")
    return EXIT_PASSED


# -- Main ---------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run CI's jobs here, and post 'CI passed' on HEAD if they pass"
    )
    parser.add_argument("jobs", nargs="*", choices=list(JOBS), metavar="JOB",
                        help=f"one of {', '.join(JOBS)}; default all, which alone posts a status")
    parser.add_argument("--rebuild-image", action="store_true",
                        help="remove the image and build it again from the newest archlinux:latest")
    args = parser.parse_args()
    jobs = args.jobs or list(JOBS)
    try:
        sha = require_clean_head()
        lock = take_turn()  # held until the process exits
        log = Log(sha)
        log.line(f"ci-local on {sha[:7]}: {', '.join(jobs)}; log {log.display_path()}")
        removed = clear_stale_test_folders()
        if removed:
            log.line(f"Removed {len(removed)} stale /tmp/{STALE_TEST_FOLDER} folders.")
        prune_old_runs()
        # HEAD may have moved while this waited for its turn.
        sha = require_clean_head()
        build_image(log, args.rebuild_image)
        started = time.monotonic()
        failed, tmp = run_jobs(log, sha, jobs)
        seconds = time.monotonic() - started
        log.line(f"== total: {seconds:.0f} s")
        if failed:
            log.line(f"Failed scenarios' directories are in {tmp}")
        else:
            remove_tree(tmp)
        if args.jobs and set(args.jobs) != set(JOBS):
            log.line("Only some jobs ran, so no status was posted.")
            return EXIT_FAILED if failed else EXIT_PASSED
        return settle(sha, failed, seconds, log)
    except Refused as refusal:
        print(refusal, file=sys.stderr)
        return EXIT_REFUSED
    except KeyboardInterrupt:
        print("Interrupted; nothing was posted.", file=sys.stderr)
        return EXIT_REFUSED


if __name__ == "__main__":
    sys.exit(main())
