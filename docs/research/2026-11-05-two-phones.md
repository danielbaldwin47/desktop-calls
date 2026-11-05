# Two Phones paired: following the one chosen last

Date: 2026-11-05. Ticket #121 (no comments). Follows [Setup Window](2026-11-05-setup-window.md), whose "Switching Phones" note this closes.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** The Setup Window keeps the Phone the user pairs in it as the chosen Phone, and every start follows that Phone and no other, whichever one BlueZ lists first. With the chosen Phone unpaired, a start opens the Setup Window and follows nobody. Verified against the Simulated Phone and simulated BlueZ only; nothing touched the real adapter, the Pixel, or the installed service.

## Decisions

- **Only a choice in the Setup Window is kept,** by Bluetooth address, in `$XDG_STATE_HOME/desktop-calls/chosen-phone.json` (`{"version":1,"address":"..."}`, directory 0700, file 0600, written whole and renamed). Following the first Phone BlueZ lists by default is not a choice and is not kept. Otherwise a user who unpaired their only Phone in Bluetooth settings and paired another outside the Setup Window would get the Setup Window on every start. **[V]** (scenarios)
- **The choice outlives the Phone being unpaired,** so no later start picks another Phone silently. The Phone paired again is followed again. **[V]** (core test; scenario for the restart)
- **Nothing kept means as before:** the first Phone listed is followed. A file that can't be read, has another version, or holds something that isn't a Bluetooth address counts as nothing kept, is logged as `chosen-phone-failed` (`error_kind` `chosen-phone-unreadable` or `chosen-phone-io`), and is replaced by the next choice. **[V]** (scenarios: not JSON, empty, another version, not an address; next choice replaces it)
- **The chosen Phone out of range at start is still the one followed:** it shows as connecting, and connects when it is back. **[V]**
- **Both Phones connected at once:** only the chosen Phone's hands-free and Calls are followed; the other Phone's AudioGateway is ignored by address. **[V]** (the chosen Phone's Call rings with both connected)
- **"First listed" is now the same every start.** BlueZ's `GetManagedObjects` is a dictionary with no order a run can rely on, and the app read it in the order its hash map gave. The app now reads objects sorted by device path, so with two Phones paired and none chosen the same one is followed each run. A real BlueZ's own order is **[I]** unspecified.
- **No notice in the Setup Window says why it opened** when the chosen Phone is gone. A follow-up if it confuses.

## Simulated Phone

- `--old-phone`'s address is now `00:A4:93:00:00:03`, sorting before any address the harness gives the Phone, so it is the Phone listed first and the scenarios fail if the choice is ignored.
- New control method `ConnectOtherPhone` brings the other Phone into range, connected, with an AudioGateway of its own at the next index.
- `Adapter1.RemoveDevice` unpairs a device that isn't connected, with `InterfacesRemoved`. A connected one fails `org.bluez.Error.Failed`: the simulation does not model BlueZ disconnecting it first, so tests take the Phone out of range, then remove it. Real BlueZ's behaviour is **[I]**.

## For the Real-Phone Checklist (user only; agents must not run these)

1. With the Pixel and another Phone paired, choose the Pixel in the Setup Window, restart the service (`systemctl --user restart desktop-calls`) a few times, and check the Pixel is followed each time.
2. Unpair the Pixel in Bluetooth settings with the other Phone still paired, restart, and check the Setup Window opens and the other Phone is not followed.
