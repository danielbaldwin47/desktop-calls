#!/usr/bin/env bash
# Runs one command in the local-CI image. Run by tools/ci_local.py inside
# `unshare --map-auto --map-root-user --mount --pid --fork --kill-child`
# (plus `--net` for a job that mustn't reach the host's network).
#
# Usage: enter.sh ROOTFS CACHE SRC TMP COMMAND
#
#   ROOTFS  the image, built by build-rootfs.sh
#   CACHE   cargo-home and target, kept between runs
#   SRC     a checkout of the commit under test, mounted at /src
#   TMP     a directory mounted as /tmp, so a failed scenario's directory
#           outlives the run
#   COMMAND run by bash in /src
#
# The host's D-Bus, PipeWire, Bluetooth and home are not mounted: nothing in
# the run can reach the real Phone or the user's session. Nor does any of the
# host's environment get in: not GH_TOKEN (lint's ticket check asks GitHub
# anonymously, which works on this public repository) and not
# DESKTOP_CALLS_OFFLINE (which would skip that check, and so can't pass a run).
set -euo pipefail
R=$1
CACHE=$2
SRC=$3
TMP=$4
COMMAND=$5

mount --rbind /dev "$R/dev"
# Headless sway and grim keep each 3840x2160 frame in /dev/shm: CI gives its
# container 1 GB of it.
mount -t tmpfs -o size=1g shm "$R/dev/shm"
if [ -d "$R/dev/dri" ]; then
    mount -t tmpfs none "$R/dev/dri"
fi
mount -t proc proc "$R/proc"
mount --rbind /sys "$R/sys"
mount --bind "$TMP" "$R/tmp"
mount -t tmpfs tmpfs "$R/run"
mkdir -p "$R/src" "$R/cache" "$CACHE/target"
mount --bind "$SRC" "$R/src"
mount --bind "$CACHE" "$R/cache"
mkdir -p "$R/src/target"
mount --bind "$CACHE/target" "$R/src/target"
# GitHub's job container runs as root on a checkout another user (the runner's)
# owns, so there git refuses the repository ("detected dubious ownership").
# Inside this namespace the checkout is root's own, so a test that ran git
# against it passed here and failed on GitHub for a week (#196). Hand the
# checkout, but not the cache mounted at target, to uid 1, another user
# mapped into this namespace.
find "$R/src" -path "$R/src/target" -prune -o -exec chown -h 1:1 {} +
cp -L /etc/resolv.conf "$R/etc/resolv.conf"
ip link set lo up 2>/dev/null || true

exec chroot "$R" /usr/bin/env -i HOME=/root PATH=/root/.cargo/bin:/usr/local/bin:/usr/bin TERM=dumb \
    CARGO_HOME=/cache/cargo-home CARGO_TERM_COLOR=never CI=true bash -euo pipefail -c "cd /src && $COMMAND"
