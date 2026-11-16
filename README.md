# Desktop Calls

> **Alpha software.** Desktop Calls is tested with one phone, a Pixel 9a, on
> Omarchy (Hyprland). It may break, drop or misroute calls, or stop working
> after a system update. Don't rely on it for calls that matter.

Take and place your Android phone's calls on a Linux desktop, over Bluetooth
hands-free. Your computer acts as the phone's car kit, so you hear and control
calls there while the phone stays in your pocket.

## Install

Needs PipeWire with its Bluetooth telephony support, BlueZ, and `bluez-obex`
for contacts. Pair the phone first, then:

```sh
just install    # build, install for this user, and start
just uninstall  # turn calls back on, stop and remove
```

Uninstalling turns calls back on: if you turned calls off, your phone is
unblocked first, so it can connect to this computer again.

`just --list` shows every other recipe. Licensed under the GPL-3.0 (see
[LICENSE](LICENSE)).

## Calls off and quiet hours

Turn calls off when you don't want calls on this computer. Your phone stays
disconnected from it. Use **Turn calls off** in the Desktop Calls menu in the
bar (until you turn them on, for 1 hour, or until morning) and **Turn calls
on** there to undo it, or the command line:

```sh
desktop-calls calls off              # until you turn them on
desktop-calls calls off --for 1h     # or 90m; up to 24h
desktop-calls calls off --until 18:30
desktop-calls calls off --until-morning
desktop-calls calls on
```

Until morning means until 07:00, or until your quiet hours end if you set them.
If you turn calls off during a call, they turn off when the call ends; **Keep
calls on** in the menu, or `desktop-calls calls on`, cancels that. You can
still place a call from the dialer while calls are off: the phone connects for
that one call, and calls turn off again when it ends.

Quiet hours turn calls off at the same times every day. Set them in
**Quiet hours…** in the menu, or:

```sh
desktop-calls quiet-hours 22:00-07:00   # may cross midnight
desktop-calls quiet-hours off
desktop-calls quiet-hours               # open the quiet hours window
```

If you turn calls on during quiet hours, they stay on until quiet hours next
begin. `desktop-calls status` says whether calls are off, and shows your
quiet hours.
