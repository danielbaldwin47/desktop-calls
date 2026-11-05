# CI red after #40: a slow first caller's number, and a full /dev/shm

Date: 2026-11-04. Follows ticket #7 (PR #40). CI runs 41947055883, 41947111574, 41947797252 (red on `main`) and 41961510810 (this branch, before the second fix).
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** two causes, both now fixed.

1. **A real bug, which CI's slower CPU made big enough to see.** Formatting the first caller's number loads libphonenumber's metadata on the GTK main thread, and that load compiles every region's patterns. It held up the Banner by 0.27 s on the Desktop and by 1.0 to 1.4 s on a CI runner (debug builds). The metadata now loads on a thread of its own at startup, and the regex compiler is optimised in debug builds too.
2. **A CI environment limit.** Docker gives the container a 64 MB `/dev/shm`, and headless sway and grim keep each 3840x2160 frame (33 MB) there. With screen scenarios running four at a time, grim died of SIGBUS and sway sometimes failed to start. The CI container now gets `--shm-size=1g`.

Neither CI's sway version nor its missing GPU was the cause. The 1 s Banner deadline in the tests is unchanged.

## 1. What CI showed

- 7 to 11 of the 11 `incoming_call_surfaces` scenarios failed. Most failures read `"the Banner shows" did not happen within 1s`. The app's state printed after the deadline already had the Banner, so it was late, not missing. **[V]** (logs of all three runs)
- In every kept app log, the gap from `ringing-started` to the next event was 1.03 to 1.42 s. The ringer's lookup runs on the main loop's next turn, so the main thread was busy for that long right after the Call appeared. **[V]** (`failed-scenario-logs` artifact of 41947797252)
- The other failures read `grim failed: `, with nothing on grim's standard error. Every one of grim's error paths prints a message, so a signal was likely. With the harness now printing the exit status, run 41961510810 showed `grim failed (signal: 7 (SIGBUS) (core dumped))` in 6 scenarios, and `sway did not start within 10s` in 1. **[V]**

## 2. The slow first caller's number

How it was found:

- The local loop that reproduces CI's timing: four scenarios at a time on one CPU, with `/dev/dri` hidden.
  `taskset -c 0 env RUST_TEST_THREADS=4 cargo test -p test-harness --test incoming_call_surfaces` (inside `unshare -rm`, with a tmpfs mounted over `/dev/dri`). Before the fix, 8 of 11 failed with CI's message. **[V]**
- Hiding the GPU alone (all 20 CPUs) passed 11 of 11, and so did one CPU with one scenario at a time. A software renderer is not the cause; CPU time is. **[V]**
- Timing probes in the app, under that loop: `Window::present` for the Banner took 0.3 to 15 ms, but `messages::caller` (the first number formatted) took 1.24 to 1.72 s. Without contention it took 0.31 s. **[V]**
- In a throwaway program on the Desktop, loading libphonenumber's database took 0.27 s in a debug build and 41 ms in a release build. The first number after it took another 37 ms (debug) or 5 ms (release); later numbers took well under 1 ms. **[V]**
- With `regex-automata` and `regex-syntax` at `opt-level = 3` in the dev profile, the debug load took 47 ms. Optimising `phonenumber` alone changed nothing, so the time goes on compiling patterns. **[V]**

The fix:

- `messages::prepare_caller()` in the core loads the metadata. The service calls it on a thread named `prepare-caller` as soon as it starts, while GTK starts up. If that thread can't start, the metadata loads on the main thread, still before any Call.
- The dev profile builds `regex-automata` and `regex-syntax` at `opt-level = 3`. Without this, a Call that rings soon after startup, as in every scenario, still waits for the load. Under the local loop, the first number then took 0.54 to 0.85 s. With both changes it took 17 to 62 ms, and the Banner mapped 37 to 108 ms after the Call appeared. **[V]**
- Under the local loop: before, 3 of 11 passed; after, 33 of 33 in three runs. In CI run 41961510810, no scenario missed the Banner deadline. **[V]**
- In a release build on the Desktop, the first Call after startup no longer waits about 45 ms for the metadata. **[I]** (from the release timings above)

## 3. The full /dev/shm

- grim allocates its capture buffer with `shm_open`, so it lives in `/dev/shm` (`buffer.c` in grim). A 3840x2160 frame at 4 bytes per pixel is 33 MB. Headless sway with the pixman renderer also allocates its frames as shared memory. **[V]** for grim (source); **[I]** for sway, from the failure below.
- Local reproduction: a 64 MB tmpfs mounted over `/dev/shm` in the same namespace. One scenario alone passed. Four at a time: 11 of 11 failed with `sway did not start within 10s`. With 256 MB or 512 MB: 11 of 11 passed. With 8 GB and all 20 CPUs: 11 of 11 passed. **[V]**
- Before #40, the only screen scenario ran alone, and the gallery runs its states one after another, so 64 MB was enough. **[V]** (run 41946445925 was green)
- `--shm-size=1g` leaves about four times the measured need for four scenarios at a time. The Desktop's `/dev/shm` is half its RAM, so this never happens there. **[I]**

## 4. CI after both fixes

- Run 41962396033, second attempt: green. `incoming_call_surfaces` passed 11 of 11 in 26.8 s, and the gallery passed in 19.9 s. **[V]**
- Its first attempt failed in the gallery, before the Banner scenarios ran: `the screen never showed call-window-ended-dark within 10s`. That attempt was slow throughout (the gallery took 35 s). **[V]**
- Likely cause, a race the gallery has always had: the Call Window shows "Call ended" for only 2 s (`CALL_ENDED_MS`). One check of that state takes a capture plus two `region_where` passes over 8.3 million pixels in a debug build: about 0.43 s on one core here, so 1 s or more on a slow runner. If the first capture lands a frame before the redraw, the next one can come after the window has closed. Not reproduced here, even with the gallery sharing one core with two or four busy loops (3 of 3 runs passed). **[I]**

## 5. Ruled out

- **Sway version or configuration:** both failures came from this machine's sway 1.12 under CI-like limits. CI's sway logs show only the missing Xwayland and swaybg, the same as here. **[V]**
- **Missing GPU:** see §2. GTK in CI falls back to software rendering (`ZINK: vkCreateInstance failed`); that may slow the first frame, but not the time before the Banner maps. **[I]** for CI; **[V]** here with `/dev/dri` hidden.
- **`cap_sys_nice`:** CI removes it so sway can start; neither slow path needed it. **[I]**
