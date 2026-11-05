# Always there

Date: 2026-11-05. Ticket #20, parts 1 (PR #78) and 2. Only the Simulated Phone was used. The real WirePlumber was never restarted, and the systemd unit was never enabled or started for the real session. The real checks belong to Real-Phone Checklist group 6 (#30).
Legend: **[V]** = verified, against the Simulated Phone or a temporary install as stated. **[I]** = inferred, not tested.

**Bottom line:** `just install` puts the app where systemd and D-Bus can start it. The app now keeps its Call through a Telephony restart and through its own crash. 6 of 6 install scenarios, 7 of 7 Telephony-restart scenarios and 4 of 4 crash-restart scenarios pass.

## 1. Install (part 1, PR #78)

- **[V]** `just install` writes the binary, a systemd user unit, a D-Bus activation file and the desktop file under the XDG directories. `crates/test-harness/tests/install.rs` installs into a temporary home.
- **[V]** `systemd-analyze --user verify` accepts the installed unit.
- **[V]** With the service stopped, `desktop-calls status` starts it through D-Bus activation on the private session bus, using the installed activation file.
- **[I]** The unit's `Restart=on-failure` and `PartOf=graphical-session.target` behave as systemd documents. Nothing here ran the real user manager.

## 2. Telephony restart (scenario 19)

What the real Pixel showed on 2026-11-04 ([conformance](2026-11-04-conformance.md)): the old owner drops its Calls, then the AudioGateway, then the name. About 0.73 s later the new owner announces the AudioGateway, and about 40 ms after that the Calls, all at the same object paths.

- The Simulated Phone's `RestartTelephony(gap_ms)` plays that order on the private bus. While it is away, the Phone's Telephony methods fail.
- In the app, a new owner of `org.pipewire.Telephony` counts as a resync. Each Call that vanishes gets a 10 s grace period, and a Call that comes back with the same number is the same Call.
- **[V]** `crates/test-harness/tests/telephony_restart.rs`, 7 of 7 pass:
  - restarted while idle: `ConnectionState` returns to connected;
  - mid-Call: the Call Window keeps the Call, and Hang up works afterwards;
  - audio on the Desktop comes back to the Desktop, and audio on the Phone stays there;
  - Hang up pressed during the gap ends the Call once it is back;
  - a slow restart of 4 s keeps the same Call;
  - Mute survives the restart and still unmutes.
- **[I]** The app gets owner changes and the AudioGateway's signals on separate streams, so on the real bus their order could swap. The grace period should absorb that; it is not tested.

## 3. Crash restart

The Phone reports each live Call's number and state, but not when it connected or which way it went. Only the Desktop knows that it silenced the microphone for Mute.

- The service keeps that in `$XDG_RUNTIME_DIR/desktop-calls/earlier-run.json` (directory 0700, file 0600, written whole and renamed). It updates the file whenever it changes, and removes it once nothing is left to keep. The next run reads it before anything the Phone reports.
- An earlier Call applies only to a Call first seen already connected with the same number, within 30 s of startup.
- Fixed on the way: an AudioGateway listed before the app attached to the Phone lost its transport properties, so a run started mid-Call showed "Audio on phone" while the audio was on the Desktop.
- **[V]** `crates/test-harness/tests/crash_restart.rs`, 4 of 4 pass:
  - started while a Call is live on the Phone, the app shows it;
  - after SIGKILL mid-Call with the audio on the Desktop, the Call keeps its direction and timer, and the audio stays on the Desktop;
  - with the audio on the Phone, the audio stays there, with the same `active_since`;
  - after SIGKILL while muted, the Call shows muted and unmutes to the microphone gain from before.
- **[I]** A Call the app never saw (installed or started mid-Call for the first time) is timed from when the app first sees it, because the Phone gives no start time.

## 4. Unverified

- A real WirePlumber restart mid-Call, a real SIGKILL under the systemd user manager, and activation through the real dbus-broker. All three are for Real-Phone Checklist group 6 (#30).
- Hang up sent during the real gap of about 1 s. The Simulated Phone covers it; the real Phone does not yet.
