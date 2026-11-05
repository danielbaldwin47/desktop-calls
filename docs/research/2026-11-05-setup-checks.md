# Setup checks

Date: 2026-11-05. Ticket #23. BlueZ 5.87, bluez-obex 5.87, WirePlumber 0.5.18 (Arch). Simulated Phone only: nothing on the real Desktop was changed.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** the app finds four kinds of Desktop Problem and shows each in the Setup Window, in `Problems` and in the Indicator (NeedsAttention, explained in its menu):

- Bluetooth off or blocked;
- the Bluetooth file transfer service (`org.bluez.obex`) missing;
- the audio not set up for Calls;
- BlueZ's own hands-free (`org.bluez.Telephony1`) on.

Two of them are fixed after one key press: Turn on, and Set up audio. The other two show the command to run, with a copy button. Ten harness scenarios (`crates/test-harness/tests/setup_checks.rs`) and 17 core tests (`crates/core/tests/setup_checks.rs`) cover them.

## 1. Bluetooth

- `org.bluez.Adapter1.PowerState` is `"off-blocked"` while rfkill blocks the adapter. Setting `Powered` then fails with `org.bluez.Error.Blocked`. **[I]** (BlueZ's `adapter.c` and its API documentation; the Simulated Phone copies this behaviour, and the real Desktop wasn't blocked to try it)
- `StartDiscovery` and `Device1.Connect` on an adapter that is off fail with `org.bluez.Error.NotReady`. The Simulated Phone does the same. **[I]** (same source)
- The app follows each adapter's `Powered` and `PowerState` through BlueZ's ObjectManager:
  - On if any adapter is powered.
  - Blocked if one is `off-blocked`; otherwise Off.
  - With no adapter, nothing is reported. The Setup Window already says Bluetooth is off or missing.
- Turn on is `Properties.Set("org.bluez.Adapter1", "Powered", <true>)` on the first adapter that is off: the same call `pairing.rs` makes for `Pairable`. **[V]** (Simulated Phone records it; scenario `bluetooth_off_shows_turn_on_…`)

## 2. Contacts support (`bluez-obex`)

- `obexd` owns `org.bluez.obex` on the session bus, started by D-Bus activation from `/usr/share/dbus-1/services/org.bluez.obex.service` (package `bluez-obex`). **[V]** (`pacman -Ql bluez-obex` on this Desktop)
- The check counts it as present if the name has an owner or is activatable. A bus that can't answer counts as present, so a wrong Problem never shows.
- The install command shown is `sudo pacman -S --needed bluez-obex`: Arch only, like the rest of the app.

## 3. Audio (WirePlumber)

- User drop-ins live in `$XDG_CONFIG_HOME/wireplumber/wireplumber.conf.d/*.conf` and load in file-name order; a later value overrides an earlier one. **[I]** (WirePlumber 0.5 configuration documentation)
- The packaged `/usr/share/wireplumber/wireplumber.conf` sets none of the three properties below. **[V]** (read on this Desktop) The check reads only the user's drop-ins, following links (dotfile managers), so a drop-in in `/etc/wireplumber` or the packaged `wireplumber.conf.d` could still override the app's setting. That is a known limit: the app can't write there without root.
- What Calls need:
  - `bluez5.roles` (in `monitor.bluez.properties`) includes `hfp_hf`, or is unset. The default includes it. **[I]**
  - `bluez5.auto-connect`, set by a `monitor.bluez.rules` rule's `update-props`, includes `hfp_hf`. Unset counts as missing. **[I]**
  - `bluez5.telephony.default-reject-sco` is `true`, so a Call's audio stays on the Phone until the app moves it. **[V]** (the Phone link's `RejectSCO`, `docs/research` on telephony)
- Set up audio:
  - Writes one drop-in, `60-desktop-calls.conf`, with only the properties that needed changing.
  - Comments out the same properties wherever else the user's drop-ins set them (`# Moved to 60-desktop-calls.conf by Desktop Calls: …`), so each is set in one file and file order can't undo it.
  - Writes each file whole, through a rename, the new drop-in first.
  - Then calls `org.freedesktop.systemd1.Manager.RestartUnit("wireplumber.service", "replace")` on the session bus.

  **[V]** (scenarios `audio_not_set_up_…` and `a_drop_in_of_the_users_own_…`, against the Simulated Phone's systemd stand-in)
- A `60-desktop-calls.conf` the user wrote (without the app's header line) is edited like any other drop-in, and the settings go in `60-desktop-calls-audio.conf` instead. This Desktop has such a hand-written file, and it already has all three settings, so the check finds no audio Problem here. **[V]** (read only)
- The Phone link already sets `RejectSCO=true` on each AudioGateway whenever no Desktop-audio Call is live, including after a WirePlumber restart. **[V]** (scenario `a_problem_stays_while_the_phone_comes_and_goes_…`, and `telephony_restart`)
- Set up audio is refused during a Call: a restart would drop the Call's audio.
- If the settings are saved but WirePlumber doesn't restart, the Problem stays, with its notice, until a restart works. The saved files alone would otherwise read as set up.

## 4. BlueZ's own hands-free

- BlueZ 5.8x's experimental telephony (`org.bluez.Telephony1`) takes the hands-free UUID, so PipeWire's hands-free can't register it. It is off unless bluetoothd runs with `-E` or `Experimental = true` is in `/etc/bluetooth/main.conf`. **[I]** (BlueZ's `doc/org.bluez.Telephony.rst`, `main.conf` documentation; not tried on this Desktop)
- The app shows the Problem while any object has `org.bluez.Telephony1`. The object path the Simulated Phone uses (`…/dev_…/telephony_ag1`) is a guess. So is when BlueZ adds the object: the Simulated Phone has it from the start, even before pairing, but real BlueZ probably adds it only once it holds a Phone's hands-free link. On a real Desktop, the Problem may first show after the Phone connects. **[I]**
- The fix needs root (`/etc`), so the app explains it and offers `sudo systemctl restart bluetooth` to copy.

## 5. Not verified

- None of the checks has run on the real Desktop, because the app mustn't run against the real Phone until #123 lands.
- The real `RestartUnit` and the clipboard under Hyprland were not tried. The harness's `wl-paste` reads the headless compositor's clipboard.
