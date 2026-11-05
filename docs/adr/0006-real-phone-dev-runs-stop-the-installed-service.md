---
status: accepted 2026-11-05 (ticket #123)
---

# Real-Phone dev runs stop the installed service, and keep their own state

Since 2026-11-05 the user's installed `desktop-calls.service` runs on the real Phone. An app started from a checkout (a dev run) by `just run`, or by a `just test-call` hook, used to run beside it: both answered the Phone, the later one silently took the app's bus name, and both read and wrote one earlier-run file, so the service picked up a dev run's Calls after its next restart (#123).

On 2026-11-05 we decided three things.

1. **The tools stop the service for the run (option b).** `just test-call` and `just run` (`tools/real-phone/test_call.py`, `tools/real-phone/run.py`) ask the systemd user manager over D-Bus whether `desktop-calls.service` is active. If it is, they stop it (`Manager.StopUnit`) before the run and start it again (`Manager.StartUnit`) once the run ends: when the run finishes, on Ctrl-C, SIGTERM or SIGHUP, and, through the guard process `just test-call` already used for hanging up, even when the tool is killed outright. A service that wasn't running stays stopped. `just run` refuses while the Phone has a Call, so it never takes a live Call away from the service. If the service can't be started again, the tool says so and exits 3.
2. **One app per session bus, enforced by the app.** The app claims `io.github.danielbaldwin47.DesktopCalls` with `DO_NOT_QUEUE` and without `ALLOW_REPLACEMENT` or `REPLACE_EXISTING`, before it reads any earlier-run state. A second copy exits with `already-running` and touches nothing; the first keeps running. zbus's default flags let a later copy take the name, which is how two apps ran at once.
3. **Separate runtime state.** The installed service keeps its earlier-run file in the directory systemd makes for it (`RuntimeDirectory=desktop-calls`, mode 0700, `RuntimeDirectoryPreserve=restart`), passed in `RUNTIME_DIRECTORY`. Any other run uses `$XDG_RUNTIME_DIR/desktop-calls-dev`. The service's directory survives its own crash-restarts, so it still picks up its own Calls, and is removed when the service is stopped, so nothing outlives a stop.

The Real-Phone Checklist starts no app. It drives whatever app is running through its control interface, which is option (a) by construction.

## Considered options

- **(a) Drive the installed service through its control interface** for `just test-call` and `just run` too. It would never touch the service, but a dev run exists to test this checkout's code, which the installed binary is not. Rejected for those two tools.
- **A second bus name or a second unit for dev runs.** Both apps would still answer the same Phone and fight over the Call's audio. Rejected: the Phone has one hands-free link, so only one app may hold it.
- **Leave the service running and refuse the run.** Safe, but every dev run would need the user to stop and restart the service by hand, which agents may not do. Rejected for the user's sake.

## Consequences

- The installed service gets its systemd-made directory, and its exclusive name claim, only once the user reruns `just install`. Until then the old binary keeps its old file, `$XDG_RUNTIME_DIR/desktop-calls/earlier-run.json`, which no dev run reads or writes any more, and the tools still stop it for each run.
- While a tool has the service stopped, D-Bus activation could start it (the unit is `Type=dbus`, and a client calling the app's name may activate it). If it does, the app's own name claim still keeps one app: whichever starts second exits with `already-running`.
- The harness has a stand-in for the systemd user manager (`crates/test-harness/src/service_manager.rs`) that runs the installed unit file as systemd would, with `RuntimeDirectory=` and `Restart=on-failure`. Tests stop and start the service only through it; nothing in the repo stops the user's real service except the two tools, as decided here.
