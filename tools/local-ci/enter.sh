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
# the run can reach the real Phone or the user's session.
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
cp -L /etc/resolv.conf "$R/etc/resolv.conf"
ip link set lo up 2>/dev/null || true

environment=(HOME=/root PATH=/root/.cargo/bin:/usr/local/bin:/usr/bin TERM=dumb
    CARGO_HOME=/cache/cargo-home CARGO_TERM_COLOR=never CI=true)
for name in GH_TOKEN DESKTOP_CALLS_OFFLINE; do
    if [ -n "${!name:-}" ]; then
        environment+=("$name=${!name}")
    fi
done
exec chroot "$R" /usr/bin/env -i "${environment[@]}" bash -euo pipefail -c "cd /src && $COMMAND"
