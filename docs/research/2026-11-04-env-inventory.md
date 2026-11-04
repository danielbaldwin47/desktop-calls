# Env inventory (read-only, 2026-11-04)
Verdict: hardware + BlueZ + PipeWire native HFP backend are ready; no phone is paired; no ofono, no obexd (PBAP/MAP), no notification daemon besides quickshell.

## 1. Bluetooth
- `lsusb`: `Bus 001 Device 010: ID 8087:0026 Intel Corp. AX201 Bluetooth`
- `rfkill list`: `hci0: Bluetooth  Soft blocked: no  Hard blocked: no`
- `bluetoothctl --version`: `bluetoothctl: 5.87` (pacman: bluez/bluez-utils/bluez-libs 5.87-2)
- `systemctl status bluetooth`: `enabled; Active: active (running) since Tue 2026-11-03 19:58:57 EST`
- `bluetoothctl show`: `Controller 11:22:33:44:55:66 (public)`, Name desktop, Powered yes, Pairable yes, Discoverable no, Roles central+peripheral.
  Local UUIDs include `Handsfree (0000111e)` AND `Handsfree Audio Gateway (0000111f)`, plus A2DP sink/source, AVRCP.
  => PipeWire native backend already registers the HF role (what we need to act as a car kit).
- `bluetoothctl devices Paired`:
  `AA:BB:CC:00:00:01 a game controller` / `AA:BB:CC:00:00:02 a speaker` / `AA:BB:CC:00:00:03 a pair of earbuds`
  => No phone paired; `bluetoothctl info` on a phone not possible (remote AG/PBAP/MAP UUIDs unverified).
- /etc/bluetooth/main.conf: all section headers only, everything commented (defaults).

## 2. Audio
- `pacman -Q`: `pipewire 1:1.6.9-1`, `wireplumber 0.5.18-1`, `pipewire-pulse 1:1.6.9-1`, `pipewire-audio 1:1.6.9-1`
- `pactl info`: `Server Name: PulseAudio (on PipeWire 1.6.9)` => pipewire-pulse in use.
- libspa-bluez5: no separate package (Arch ships it in pipewire); `/usr/lib/spa-0.2/bluez5/libspa-bluez5.so` present,
  codecs incl. hfp-cvsd, hfp-msbc, hfp-lc3-swb, hfp-lc3-a127.
- WirePlumber config: `/etc/wireplumber` does not exist. `/usr/share/wireplumber` has no hfphsp-backend/hfp_hf/roles overrides (defaults = native backend, all roles).
  `~/.config/wireplumber/wireplumber.conf.d/`: `bluetooth-a2dp-autoconnect.conf` (`bluez5.auto-connect = [ a2dp_sink a2dp_source ]` for `~bluez_card.*`), `kef-lsx-no-suspend.conf`. Note: auto-connect omits hfp_hf.
- `pacman -Q ofono`: `error: package 'ofono' was not found`
- `pacman -Q bluez-obex`: not found; `which obexd` none; `/usr/lib/bluetooth/` contains only `bluetoothd`; no obex user unit.

## 3. Desktop
- Notifications: no mako/swaync/dunst (not installed). `busctl --user status org.freedesktop.Notifications` => `Comm=quickshell`, `CommandLine=quickshell -n -p /usr/share/omarchy/shell` (Omarchy shell owns notifications).
- `hyprctl version`: `Hyprland 0.56.2 ... Tag: v0.56.2`
- `pacman -Q`: `gtk4 1:4.24.1-1`, `gtk4-layer-shell 1.3.0-1`, `libadwaita 1:1.10.0-1`

## 4. Toolchains
- `rustc 1.98.0`, `cargo 1.98.0` (via rustup 1.29.1)
- `Python 3.14.8`; `node v26.12.1` (mise; pacman nodejs 26.12.0)
- go, adb (android-tools), scrcpy, kdeconnect: not found
- AUR helper: `yay v13.0.1 - libalpm v16.0.1`; paru not found

## 5. Groups
- `id`: `uid=1000(user) gid=1000(user) groups=1000(user),992(input),998(wheel)` (no lp/bluetooth/audio group; not needed under systemd-logind/polkit defaults, inferred)
