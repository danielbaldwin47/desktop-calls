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
just uninstall  # turn calls back on, stop and remove
```

Uninstalling turns calls back on: if you turned calls off, your phone is
unblocked first, so it can connect to this computer again.

`just --list` shows every other recipe. Licensed under the GPL-3.0 (see
[LICENSE](LICENSE)).

## Calls off and Quiet Hours

Turn calls off when you don't want Calls on this computer. Your phone stays
disconnected from it. Use **Turn calls off** in the Indicator's menu (until
you turn them on, for 1 hour, or until morning) and **Turn calls
on** there to undo it, or the command line:

```sh
desktop-calls calls off              # until you turn them on
desktop-calls calls off --for 1h     # or 90m; up to 24h
desktop-calls calls off --until 18:30
desktop-calls calls off --until-morning
desktop-calls calls on
```

Until morning means until 07:00, or until your Quiet Hours end if you set them.
Turned off during a Call, calls go off when the Call ends; **Keep calls on**
in the Indicator's menu, or `desktop-calls calls on`, cancels that. A Call
you place from the Dialer while calls are off connects the phone for that
one Call, and calls go off again when it ends.

Quiet Hours turn calls off at the same times every day. Set them in
**Quiet hours…** in the Indicator's menu, or:

```sh
desktop-calls quiet-hours 22:00-07:00   # may cross midnight
desktop-calls quiet-hours off
desktop-calls quiet-hours               # open the Quiet Hours window
```

Turning calls on during Quiet Hours keeps them on until Quiet Hours next
begin. `desktop-calls status` says whether calls are off, and shows your
Quiet Hours.
