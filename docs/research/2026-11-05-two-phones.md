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
- **With no Phone followed, the Setup Window also lists the Phones already paired,** before the nearby ones. Choosing one keeps it as the chosen Phone, follows it, trusts it and connects it, with no pairing. Without this, the window opened with an empty list when the chosen Phone was unpaired, and the only way out was pairing again in Bluetooth settings. While a Phone is followed, the window lists nearby Phones alone, as before. **[V]** (core tests; scenario: unpair the chosen Phone, restart, choose the old Phone with Return, restart again)
- **A paired Phone chosen while out of range** ends on a step of its own, "Can't reach <Phone>; bring it close with Bluetooth on", with Try again, which connects the Phone afresh; the window is done once it is connected. Before, the Phone's `ConnectProfile` page timeout read as a refusal and the window said it wasn't sharing Calls. **[V]** (core test; scenario: choose the away Phone, see the message, bring it back, Try again, done)
- **No notice in the Setup Window says why it opened** when the chosen Phone is gone: ticket #150.

## Simulated Phone

- `--old-phone`'s address is now `00:A4:93:00:00:03`, sorting before any address the harness gives the Phone, so it is the Phone listed first and the scenarios fail if the choice is ignored.
- New control methods: `ConnectOtherPhone` brings the other Phone into range, connected, with an AudioGateway of its own at the next index (a `Connect` to it then succeeds), and `RingOnOtherPhone` makes a Call ring on that AudioGateway.
- `Adapter1.RemoveDevice` unpairs a Phone that isn't connected, with `InterfacesRemoved`. A connected one fails `org.bluez.Error.Failed`: the simulation does not model BlueZ disconnecting it first, so tests take the Phone out of range, then remove it.

## Not verified

- **[I]** That a real BlueZ disconnects a connected Phone before removing it, and so what the app sees when the user unpairs the followed Phone mid-Call. No scenario unpairs a Phone with a live Call.
- **[I]** That a written choice survives power loss: the file is written whole and renamed, which is atomic, but the directory is not fsynced.
- **[I]** The order a real BlueZ lists Phones in.
- **[I]** What a real BlueZ's `ConnectProfile` says for a Phone out of range. The app takes `br-connection-page-timeout` (seen on the Pixel for `Connect`), "host is down" or `org.bluez.Error.ConnectionAttemptFailed` as "didn't answer"; any other error stays "Phone calls off". Checklist item 3 records the real text.

## For the Real-Phone Checklist (user only; agents must not run these)

1. With the Pixel and another Phone paired, choose the Pixel in the Setup Window, restart the service (`systemctl --user restart desktop-calls`) a few times, and check the Pixel is followed each time.
2. Unpair the Pixel in Bluetooth settings with the other Phone still paired, restart, and check the Setup Window opens and the other Phone is not followed. Then choose the other paired Phone in the Setup Window and check it connects without pairing.
3. With that other Phone out of range (Bluetooth off on it), choose it in the Setup Window and check the window says "Can't reach <Phone>; bring it close with Bluetooth on", not that Phone calls are off. Switch its Bluetooth on, choose Try again, and check the window reaches "connected". Record the `detail` of the `hands-free-unreachable` journal line.
