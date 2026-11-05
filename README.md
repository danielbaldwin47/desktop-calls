# Desktop Calls

> **Alpha software.** Desktop Calls is tested with one phone, a Pixel 9a, on
> Omarchy (Hyprland). It may break, drop or misroute Calls, or stop working
> after a system update. Don't rely on it for calls that matter.

Take and place your Android phone's Calls on a Linux desktop, over Bluetooth
hands-free: the Desktop acts as the phone's car kit, so Call audio and control
come to the Desktop while the phone stays in your pocket.

## Install

Needs PipeWire with its Bluetooth telephony support, BlueZ, and `bluez-obex`
for Contacts. Pair the phone first, then:

```sh
just install    # build, install for this user, and start
just uninstall  # stop and remove
```

`just --list` shows every other recipe. Licensed under the GPL-3.0 (see
[LICENSE](LICENSE)).
