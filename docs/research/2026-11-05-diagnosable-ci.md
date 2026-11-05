# Diagnosable CI: reproducing CI's limits on a shared Desktop, and failures that say why

Date: 2026-11-05. Ticket #43. Supersedes the local reproduction recipe in [2026-11-04-ci-banner-latency.md](2026-11-04-ci-banner-latency.md): **don't copy its `taskset -c 0` loop**. Use `just test-ci-like`.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** `just test-ci-like [args]` runs the tests under a cgroup CPU quota of 100% (one CPU's worth of time, spread over whichever CPUs the kernel picks), four scenarios at a time, with no GPU and CI's 1 GB `/dev/shm`. It takes a machine-wide lock, so agents take turns instead of skewing each other's timing. At that quota `main` passes and reverting PR #41's fix fails the Banner scenarios, and the failure's timeline says `main-loop-stalled: The main loop was held up for 1280 ms during=deciding`. CI's own runs showed the same 1.0 to 1.4 s stall.

## 1. Why not the CPU 0 recipe

The 2026-11-04 note reproduced CI's timing with `taskset -c 0 env RUST_TEST_THREADS=4 cargo test ...`. It works for one agent alone. On a Desktop shared by several agents, every agent that copies it pins its scenarios to the same CPU. Their runs then starve each other and every other agent's tests: timing depends on who else is running, and nothing says so.

The replacement:

| | CPU 0 recipe | `just test-ci-like` |
|---|---|---|
| CPU | everything pinned to CPU 0 | cgroup quota (`systemd-run --user --scope -p CPUQuota=100%`); the kernel spreads it over idle CPUs |
| Other agents | share CPU 0 with every copy | unaffected beyond one CPU's worth of time |
| Two runs at once | silently skew each other | `flock` on `$XDG_RUNTIME_DIR/desktop-calls-test-ci-like.lock`; the second prints who holds it (process, worktree, branch, start time, arguments) and waits |
| GPU, `/dev/shm` | hand-made `unshare -rm` and mounts | the same, built in, in a private mount namespace |
| Temporary files | shared `/tmp` | a TMPDIR per run, removed afterwards; failed scenarios' directories are copied to `target/ci-like/<time>/` first |

- The quota works in the agent sandbox: the user manager delegates the `cpu` controller (`cgroup.controllers` of `user@1000.service`: `cpu memory pids`). With a 50% quota, two busy processes got 0.50 CPU-seconds each over 2 s of wall clock, against 2.00 without it. Inside a run, `cpu.max` reads `100000 100000`, `/dev/dri` is empty (17 entries outside), `/dev/shm` is 1.0 GB, and the user ID is mapped back to 1000. **[V]**
- No fallback was needed. If a machine lacks `systemd-run --user`, the script exits 2 and says what is missing. **[V]** (code path) **[I]** (not run on such a machine)
- The binaries build before the lock is taken and outside the quota, so a run holds the lock only while tests run. **[V]**

## 2. Calibration

The scenarios in `incoming_call_surfaces` (11 tests), four at a time, on the Desktop (i5-13600K, 20 CPUs) while other agents ran their suites. "Reverted" means no `prepare_caller()` at startup and `regex-automata` and `regex-syntax` at `opt-level = 0`, as before #41.

| CPU quota | `main` | reverted #41 |
|---|---|---|
| 200% | 11 of 11 passed (1 run) | 11 of 11 passed (1 run): **does not reproduce** |
| 150% | not run | 1 of 11 failed in each of 2 runs |
| 100% | 11 of 11 passed in each of 3 runs | 7 of 11 failed in each of 3 runs |
| 75% | 11 of 11 passed in each of 2 runs | not run |
| 50% | 11 of 11 passed (1 run) | not run |

**[V]** (every cell)

- 100% is the default. It fails the reverted fix reliably and leaves `main` passing with room to spare (it still passes at 50%). The stall it produces, 1.2 to 1.3 s, matches what CI's runners showed for the same bug (1.03 to 1.42 s). **[V]**
- CI's runners have 2 vCPUs (private repo, standard runner), but each is slower than a Desktop core: the same bug cost 0.27 s on the Desktop alone and over 1 s on CI. So 200% is too generous, and 100% is the closer match for this workload. **[I]** from the stall durations above. If CI's failures stop reproducing, recalibrate with `TEST_CI_LIKE_CPU_QUOTA=…` and update this table.
- `/dev/shm` is 1 GB, as CI's container now has (`--shm-size=1g`). The ticket's 64 MB was Docker's default before #41, and four screen scenarios at a time fail at it on `main` too (2026-11-04 note, section 3). Use the small-`/dev/shm` check in section 4 to reproduce that failure on purpose.
- The whole suite on `main` under `just test-ci-like`: 223 of 223 tests passed in about 7 min, the gallery alone taking 240 s. **[V]**
- Stopping a run with SIGTERM mid-scenario leaves no TMPDIR, no scope and no process carrying its TMPDIR: the script stops its named scope (`desktop-calls-test-ci-like-<pid>`) on the way out. **[V]**

## 3. Failures that say why

**The scenario's timeline.** Every harness wait that times out appends the last 80 lines of a timeline in ms since the scenario started, and keeps all of it in `timeline.txt` in the scenario's directory:

- Every signal and method call on both scenario buses, from a connection that calls `org.freedesktop.DBus.Monitoring.BecomeMonitor`. dbus-daemon allows it for a connection from the bus's own user, so the harness needs no policy change. Senders and destinations are labelled (`app`, `harness`, `Simulated Phone (Telephony)` ...), and polling calls (`Get`, `GetAll`, `Introspect`, `RecordedMethodCalls` ...) and method returns are left out. **[V]**
- The app's log events, read from its JSON log, with their fields. **[V]**
- Bus entries are timed when the harness's monitor reads them, so under the quota they can trail the app's own log entries by tens of ms (a `call-added` log at 1920 ms against its `CallAdded` signal at 2001 ms). Read the order within about 100 ms as approximate. **[V]**

**`main-loop-stalled`.** The app logs it with `stalled_ms` and `during` in two ways:

- `during=deciding`: the turn that runs the Call state machine's `step` and works out the surfaces took over 100 ms. The #41 bug was here: the first caller's number formatted in this turn, before anything else could run. **[V]**
- `during=between ticks`: a 20 ms GLib timer came over 100 ms late. It runs only while there is a Call or a surface is shown, so an idle app doesn't wake 50 times a second. A first version had only this timer. It started in the same turn as the slow step, so it missed the stall that mattered, which is why the step is now timed directly. **[V]**

Reverted #41 at 100%, from the panic of `clicking_answer_answers_and_the_call_window_shows_the_audio_here_and_a_timer` (lines shortened):

```
"the Banner shows" did not happen within 1s; the app's state is Ok(AppState { ... })
The scenario's timeline, in ms since it started (also in .../timeline.txt):
   1920 app log  call-added: Call 1 appeared, ringing call_id=1 call_state=ringing
   2001 session  Simulated Phone (Telephony) signals VoiceCallManager.CallAdded on /org/pipewire/Telephony/ag36 ...
   2001 session  app calls Simulated Phone (obex) PhonebookAccess1.Select on /org/bluez/obex/client/session0 ("int", "cch")
   3201 app log  main-loop-stalled: The main loop was held up for 1280 ms during=deciding stalled_ms=1280
   3201 app log  ringing-started: Ringing
   3217 session  app signals Properties.PropertiesChanged on /io/github/danielbaldwin47/DesktopCalls (... {"Surfaces": <{"banner": ...
```

## 4. sway and grim

- Shrinking `/dev/shm` in a private mount namespace (`unshare --user --map-root-user --mount`, then a tmpfs over `/dev/shm`) reproduces both CI failures in one screen scenario. **[V]**
  - 50 MB: `grim failed (signal: 7 (SIGBUS) (core dumped)); /dev/shm has 0 MB free of 50 MB. grim said: ""`
  - 10 MB: `sway did not start: it exited (signal: 7 (SIGBUS) (core dumped)); /dev/shm has 10 MB free of 10 MB. sway wrote nothing to its log`
- sway's start used to wait the full 10 s even after sway had died. It now stops as soon as sway exits. **[V]**
- The scenario's sway runs with `xwayland disable` and `swaybg_command -`. Before, its log was mostly Xwayland and swaybg lines, which hid sway's own error. **[V]**

## 5. Nothing a scenario starts outlives it

- Every process a scenario starts joins one process group, led by a guard `sh` that reads its stdin. The harness holds the write end of that pipe. When the test process dies for any reason, SIGKILL included, the kernel closes the pipe, and the guard runs `kill -s KILL 0` on the group. **[V]** (test `nothing_a_scenario_started_outlives_its_killed_test_process`: 5 or more processes before, none 5 s after SIGKILL; with the group disabled, 5 leaked)
- Why not `PR_SET_PDEATHSIG`: it needs `unsafe` (the codebase forbids it), it fires when the spawning *thread* exits rather than the process, and the kernel clears it across an exec of a binary with file capabilities (sway has one). **[V]** (prctl(2))

## 6. CI keeps the evidence

- Both CI jobs upload `/tmp/desktop-calls-test-*` with `if: always()` (artifacts `scenario-logs-check` and `scenario-logs`). Failed scenarios keep their directories. With `KEEP_SCENARIO_LOGS=1`, passing ones do too: set it locally, or tick "keep_scenario_logs" when running the workflow by hand. **[V]**
- `just ci-wait [run-id]` waits on a run. If it passed, it prints one line. If not, it prints each failed test with its panic and downloads the scenario logs into `target/ci/<run-id>/`. **[V]** (failed runs 41981986246, 41965977375 and 41961510810; passing run 41986685308)
