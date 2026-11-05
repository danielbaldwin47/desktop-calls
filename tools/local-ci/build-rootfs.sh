#!/usr/bin/env bash
# Builds the local-CI image as a plain directory, rootless and without a
# container engine. Run by tools/ci_local.py inside `unshare --map-auto
# --map-root-user --mount --pid --fork`, which maps root and the other IDs
# onto this user's /etc/subuid range.
#
# Usage: build-rootfs.sh ROOTFS LAYERS_DIR
#
# Unpacks CI's base image, archlinux:latest (its layers, pulled by
# `just ci-local` into LAYERS_DIR as 00.tar, 01.tar, ...), and installs what
# the "Install system packages" step of .github/workflows/ci.yml installs.
# That list is copied here: a package CI gains and this lacks shows up as a
# local run that fails where CI passes, or the reverse, so keep them equal
# (tools/ci_local_test.py checks it).
set -euo pipefail
R=$1
LAYERS=$2
mkdir -p "$R"
for layer in "$LAYERS"/*.tar; do
    tar -xf "$layer" -C "$R" --numeric-owner
done
mount --rbind /dev "$R/dev"
mount -t proc proc "$R/proc"
mount --rbind /sys "$R/sys"
mount -t tmpfs tmpfs "$R/tmp"
cp -L /etc/resolv.conf "$R/etc/resolv.conf"
# pacman's download sandbox needs a seccomp and Landlock setup that an
# unprivileged user namespace doesn't offer.
sed -i 's/^#\?DisableSandbox.*//; /^\[options\]/a DisableSandbox' "$R/etc/pacman.conf"
exec chroot "$R" /usr/bin/env -i HOME=/root PATH=/usr/local/sbin:/usr/local/bin:/usr/bin TERM=dumb \
    bash -euo pipefail -c '
pacman-key --init >/dev/null 2>&1
pacman-key --populate archlinux >/dev/null 2>&1
pacman -Syu --noconfirm --needed \
    gcc pkgconf git rustup just cargo-deny cargo-nextest \
    gtk4 gtk4-layer-shell libadwaita dbus python python-dbus python-gobject \
    sway grim wl-clipboard libnotify
pacman -Scc --noconfirm >/dev/null
rm -rf /var/cache/pacman/pkg/* /usr/share/man /usr/share/doc /usr/share/gtk-doc
# As in CI: the package gives sway a file capability the container refuses.
setcap -r /usr/bin/sway
'
