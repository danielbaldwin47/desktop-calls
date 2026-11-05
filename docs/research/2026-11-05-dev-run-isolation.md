# Real-Phone dev runs never share state with, or run beside, the installed service

Date: 2026-11-05. Ticket #123, [ADR-0006](../adr/0006-real-phone-dev-runs-stop-the-installed-service.md). Simulated Phone and a stand-in systemd user manager only.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** a second copy of the app now refuses to start beside the first. The installed service keeps its earlier-run state in a directory systemd makes for it. `just test-call` and the new `just run` stop the service for their run and start it again afterwards, even when killed. The user's real service was never stopped or started: everything below ran against a stand-in.

## Why two apps could run at once

- zbus 6.2.1's `Connection::request_name` uses `RequestNameFlags::default()`, which is `ALLOW_REPLACEMENT | REPLACE_EXISTING | DO_NOT_QUEUE` (`zbus-6.2.1/src/fdo/dbus.rs:91`). So each new copy took the bus name from the running one, and the running one let it go without noticing. **[V]** by reading the source, and by `a_second_app_beside_a_running_one_refuses_and_leaves_it_running` timing out before the fix.
- The app now claims the name with `DO_NOT_QUEUE` alone, before it reads any earlier-run state, and exits with `already-running` on `Exists`. **[V]** (`dev_runs.rs`)
- The installed service still runs the old binary, which allows replacement, until the user reruns `just install`. A new dev run doesn't ask to replace, so it still refuses beside it. **[I]** from the flag semantics; not tried against the real service.

## Where the state lives

- systemd's `RuntimeDirectory=desktop-calls` makes `$XDG_RUNTIME_DIR/desktop-calls` (mode from `RuntimeDirectoryMode=0700`) before `ExecStart` and passes it as `$RUNTIME_DIRECTORY`. With `RuntimeDirectoryPreserve=restart` it survives an automatic restart after a crash and is removed on a stop. **[I]** from `systemd.exec(5)`; the stand-in implements the same rules and the tests run against it.
- A run without `RUNTIME_DIRECTORY` uses `$XDG_RUNTIME_DIR/desktop-calls-dev`. **[V]** `a_dev_run_stopped_mid_call_leaves_nothing_for_the_installed_service`: a dev run killed mid-Call leaves a file that the service never loads, and the next dev run does.
- The service still picks up its own Calls after a crash. **[V]** `the_installed_service_picks_up_its_own_calls_after_a_crash` (same `active_since_ms`); it fails when `RuntimeDirectoryPreserve=restart` is removed from the unit.

## The tools

- `test_call.py` and `run.py` read `ActiveState` through `Manager.GetUnit`, then call `Manager.StopUnit("desktop-calls.service", "replace")` and later `StartUnit` with the same arguments. Signatures were introspected read-only on the real user manager: `GetUnit(s)->o`, `StartUnit(ss)->o`, `StopUnit(ss)->o`. **[V]** (introspection). `StopUnit` and `StartUnit` were never called on the real manager. **[V]** they work against the stand-in (`real_phone_tools.rs`, 6 scenarios); **[I]** that the real manager behaves the same.
- The guard process gains a `service-stopped` line. When the tool dies without saying `done`, the guard stops the hook or dev run, hangs up the Test Call if one was dialled, and starts the service. **[V]** with SIGKILL for both tools.
- A Call that ended while the app was down is now logged when it's forgotten (`earlier-run-forgotten`), and the startup line says the Calls *may* be continuing. **[V]** `crash_restart.rs`.

## Follow-ups

- The user reruns `just install` to get the new unit file and binary.
- The Real-Phone Checklist still tells the user to start the app with `cargo run`; with the service installed it should say the service is already running, or point at `just run` (#120/#125 own the checklist).
- "Dev run" and "installed service" are used as the ticket uses them, not as glossary terms.
