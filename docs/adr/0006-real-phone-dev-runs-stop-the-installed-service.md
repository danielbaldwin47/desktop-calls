---
status: accepted 2026-11-05 (ticket #123)
---

# Real-Phone dev runs stop the installed service, and keep their own state

Since 2026-11-05 the user's installed `desktop-calls.service` runs on the real Phone. An app started from a checkout (a dev run) by `just run`, or by a `just test-call` hook, used to run beside it: both answered the Phone, the later one silently took the app's bus name, and both read and wrote one earlier-run file, so the service picked up a dev run's Calls after its next restart (#123).

On 2026-11-05 we decided three things.

1. **The tools stop the service for the run (option b).** `just test-call` and `just run` (`tools/real-phone/test_call.py`, `tools/real-phone/run.py`) ask the systemd user manager over D-Bus whether `desktop-calls.service` is active. If it is, they stop it (`Manager.StopUnit`) before the run and start it again (`Manager.StartUnit`) once the run ends: when the run finishes, on Ctrl-C, SIGTERM or SIGHUP, and, through the guard process `just test-call` already used for hanging up, even when the tool is killed outright. A service that wasn't running stays stopped. `just run` refuses without the Phone's hands-free link or while the Phone has a Call, so it never takes a live Call away from the service. If the service can't be started again, the tool says so and exits 3. Restarting never waits on hanging up: the guard gives the service back whatever the hang-up did.
2. **One app per session bus, enforced by the app.** The app claims `io.github.danielbaldwin47.DesktopCalls` with `DO_NOT_QUEUE` and without `ALLOW_REPLACEMENT` or `REPLACE_EXISTING`, before it reads any earlier-run state. A second copy exits with `already-running` and touches nothing; the first keeps running. zbus's default flags let a later copy take the name, which is how two apps ran at once.
   Runs share the stop. Each run that holds the service stopped has a file named by its process ID (with the process's start time, so a reused ID doesn't count) in `$XDG_RUNTIME_DIR/desktop-calls-service-holds/`, and the directory itself says the service must be started again. A lock beside it (`desktop-calls-service-holds.lock`, `flock`) orders every change, so two runs at once stop the service once, and only the last to end starts it. `start` never starts the service while another copy of the app is still on the bus.
3. **Separate runtime state.** The installed service keeps its earlier-run file in the directory systemd makes for it (`RuntimeDirectory=desktop-calls-service`, mode 0700, `RuntimeDirectoryPreserve=restart`), passed in `RUNTIME_DIRECTORY`. Any other run uses `$XDG_RUNTIME_DIR/desktop-calls-dev`. The service's directory survives its own crash-restarts, so it still picks up its own Calls, and is removed when the service is stopped, so nothing outlives a stop. The name is one no earlier build used: builds before this decision kept their earlier-run file and pulled vCards in `$XDG_RUNTIME_DIR/desktop-calls`, and systemd reuses a directory that already exists, so that name would have handed the service whatever an older dev run left there.

The Real-Phone Checklist starts no app. It drives whatever app is running through its control interface, which is option (a) by construction.

## Considered options

- **(a) Drive the installed service through its control interface** for `just test-call` and `just run` too. It would never touch the service, but a dev run exists to test this checkout's code, which the installed binary is not. Rejected for those two tools.
- **A second bus name or a second unit for dev runs.** Both apps would still answer the same Phone and fight over the Call's audio. Rejected: the Phone has one hands-free link, so only one app may hold it.
- **Leave the service running and refuse the run.** Safe, but every dev run would need the user to stop and restart the service by hand, which agents may not do. Rejected for the user's sake.

## What restarts the service, and what doesn't

| The run ends by | The service is started again by |
| --- | --- |
| finishing, Ctrl-C, SIGTERM, SIGHUP | the tool itself |
| SIGKILL of the tool | its guard, at once |
| SIGKILL of the tool and its guard (or a crash of both) | the next `just run` or `just test-call`, when it starts |
| logout or reboot | the user session: `$XDG_RUNTIME_DIR` goes, and the enabled service starts at the next login |

Not covered: if everything dies and no Real-Phone tool runs again, the service stays stopped until the user starts it or logs in again. A dev run left on the bus by such a death keeps the service from being started beside it; the tool says so and exits 3.

## Consequences

- The installed service gets its systemd-made directory, and its exclusive name claim, only once the user reruns `just install`. Until then the old binary keeps its old file, `$XDG_RUNTIME_DIR/desktop-calls/earlier-run.json`, which no new dev run reads or writes, and the tools still stop it for each run.
- While a tool has the service stopped, D-Bus activation could start it (the unit is `Type=dbus`, and a client calling the app's name may activate it). If it does, the app's own name claim still keeps one app: whichever starts second exits with `already-running`. The Real-Phone tools ask the bus (`NameHasOwner`) before they call the app's name, so they never activate it themselves.
- The harness has a stand-in for the systemd user manager (`crates/test-harness/src/service_manager.rs`) that runs the installed unit file as systemd would, with `RuntimeDirectory=` and `Restart=on-failure`, and counts the service active only once its own process owns the bus name. Tests stop and start the service only through it; nothing in the repo stops the user's real service except the two tools, as decided here.
