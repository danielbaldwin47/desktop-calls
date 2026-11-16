#!/usr/bin/env python3
"""Look at free space on /tmp and /home before work that fills them (ticket
#397, from the retro of 2026-11-15, candidate 12).

Three outages had one cause: `/home` filled on 2026-11-14 from worktrees'
target/ folders, and the per-user `/tmp` quota was hit on 2026-11-05 and
2026-11-15 from scenario folders. Nothing looked before work started. So
`just land`, `just ci-wait`, and every run of the harness's scenarios
(tools/scenario_shader_cache.py, and tools/test_ci_like.py before its
build) call `preflight()` first; `just ci-wait` only warns:

  under the warning headroom: print a warning naming the largest folders;
  under the refusal headroom: refuse, naming them, so nothing starts that
  would fail halfway on a full disk.

Headroom on `/tmp` is the smaller of the filesystem's free space and what
is left of this user's quota there (tmpfs `usrquota`, read with
quotactl_fd). On GitHub's runners, which start empty each time, it does
nothing.

  python3 tools/free_space.py      # print the headroom and any warning
"""

from __future__ import annotations

import ctypes
import dataclasses
import os
import pathlib
import subprocess
import sys

GB = 1_000_000_000


@dataclasses.dataclass(frozen=True)
class Place:
    """A filesystem to watch, the headrooms that warn and refuse, and the
    folders that have filled it before."""

    name: str
    path: pathlib.Path
    warn_below: int
    refuse_below: int
    usual_suspects: tuple[str, ...]
    next_step: str


HOME = pathlib.Path.home()


def main_checkout() -> pathlib.Path:
    """The repository's main checkout, whose .claude/worktrees holds every
    agent's worktree (this file may be in one of them)."""
    here = pathlib.Path(__file__).resolve().parent.parent
    result = subprocess.run(["git", "-C", str(here), "rev-parse", "--path-format=absolute", "--git-common-dir"],
                            capture_output=True, text=True)
    common = pathlib.Path(result.stdout.strip())
    return common.parent if result.returncode == 0 and common.name == ".git" else here


REPO = main_checkout()
# A worktree's target/ is 5 to 6 GB, so /home warns while a few more fit,
# and refuses before the next build could fill it. A screen scenario's
# folder is about 90 MB, and a failing run keeps every one, so /tmp warns
# with room for a few failing runs.
PLACES = (
    Place("/home", HOME, warn_below=30 * GB, refuse_below=10 * GB,
          usual_suspects=(f"{REPO}/.claude/worktrees/*/target", f"{REPO}/target",
                          f"{HOME}/.cache/desktop-calls-ci", f"{HOME}/.cache/*", f"{HOME}/.local/share/*"),
          next_step="`cargo clean` in a finished worktree; a merged one can go with "
                    "`claude-steward --dry-run --full` first"),
    Place("/tmp", pathlib.Path("/tmp"), warn_below=3 * GB, refuse_below=1 * GB,
          usual_suspects=("/tmp/desktop-calls-test-*", "/tmp/*"),
          # Other agents' runs share /tmp: never sweep it (chief-of-staff-practice).
          next_step="delete the /tmp/desktop-calls-test-* folders your own finished runs left, "
                    "or ask the chief of staff; never other agents'"),
)
LARGEST_SHOWN = 5
DU_SECONDS = 60

# quotactl_fd(2), the same number on x86-64 and arm64, and Q_GETQUOTA for user quotas (linux/quota.h).
SYS_QUOTACTL_FD = 443
Q_GETQUOTA_USER = 0x800007 << 8
QUOTA_BLOCK = 1024


class Refused(Exception):
    """Too little headroom to start. The message names what to clear."""


class _Quota(ctypes.Structure):
    _fields_ = [(name, ctypes.c_uint64) for name in (
        "hard", "soft", "used", "inode_hard", "inode_soft", "inodes", "block_time", "inode_time")] + [
        ("valid", ctypes.c_uint32)]


def quota_left(path: pathlib.Path) -> int | None:
    """Bytes left of this user's quota on `path`'s filesystem, or None if it
    has none (or the kernel won't say)."""
    if sys.platform != "linux":
        return None
    try:
        libc = ctypes.CDLL(None, use_errno=True)
        descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    except OSError:
        return None
    try:
        quota = _Quota()
        if libc.syscall(SYS_QUOTACTL_FD, descriptor, Q_GETQUOTA_USER, os.getuid(), ctypes.byref(quota)) != 0:
            return None
    finally:
        os.close(descriptor)
    if not quota.hard:
        return None
    return max(quota.hard * QUOTA_BLOCK - quota.used, 0)


def headroom(path: pathlib.Path) -> int:
    """Bytes that can still be written under `path`."""
    stats = os.statvfs(path)
    free = stats.f_bavail * stats.f_frsize
    left = quota_left(path)
    return free if left is None else min(free, left)


def largest(patterns: tuple[str, ...]) -> list[tuple[int, str]]:
    """The biggest folders matching `patterns`, as (bytes, path), biggest first."""
    folders = sorted({str(found) for pattern in patterns for found in pathlib.Path("/").glob(pattern.lstrip("/"))
                      if found.is_dir() and not found.is_symlink()})
    if not folders:
        return []
    try:
        result = subprocess.run(["du", "-sb", "--", *folders], capture_output=True, text=True, timeout=DU_SECONDS)
    except (OSError, subprocess.TimeoutExpired):
        return []
    sizes = []
    for line in result.stdout.splitlines():
        size, _, folder = line.partition("\t")
        if size.isdigit():
            sizes.append((int(size), folder))
    return sorted(sizes, reverse=True)[:LARGEST_SHOWN]


def gb(size: int) -> str:
    return f"{size / GB:.1f} GB"


def message(place: Place, room: int, folders: list[tuple[int, str]]) -> str:
    lines = [f"{place.name} has {gb(room)} free for you"]
    if folders:
        lines.append("Its largest folders:")
        lines += [f"  {gb(size):>9}  {folder}" for size, folder in folders]
    return "\n".join(lines)


def preflight(places: tuple[Place, ...] = PLACES, room_of=headroom, largest_of=largest) -> None:
    """Warns on standard error under a place's warning headroom; raises
    Refused under its refusal headroom. Silent with room to spare."""
    if os.environ.get("GITHUB_ACTIONS") == "true":
        return
    for place in places:
        if not place.path.exists():
            continue
        room = room_of(place.path)
        if room >= place.warn_below:
            continue
        text = message(place, room, largest_of(place.usual_suspects))
        if room < place.refuse_below:
            raise Refused(
                f"{text}\nThat is under {gb(place.refuse_below)}, so this would likely fail halfway (#397).\n"
                f"Next: free space ({place.next_step}), then run this again"
            )
        print(f"Warning: {text}\n(under {gb(place.warn_below)}; this refuses under {gb(place.refuse_below)}, #397)",
              file=sys.stderr, flush=True)


def main() -> int:
    for place in PLACES:
        if place.path.exists():
            print(f"{place.name}: {gb(headroom(place.path))} free for you "
                  f"(warns under {gb(place.warn_below)}, refuses under {gb(place.refuse_below)})")
    try:
        preflight()
    except Refused as refusal:
        print(f"Refused: {refusal}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
