# A real 16-minute Call on the installed build: what the logs show

Date: 2026-11-13. No ticket: a read of the logs after the user's first long real Call through the installed Desktop Calls. Strictly read-only: nothing touched the Phone, Bluetooth, WirePlumber or the running app.
Sources: the app's journal (`journalctl --user -u desktop-calls.service`; the installed app logs to the journal, so it has no log file), the user journal for WirePlumber and PipeWire, and the system journal for bluetoothd, the kernel and rfkill, 17:10 to 17:40 local time, plus the app's journal back to 2026-11-08 for context.
Legend: **[observed in log]** = read from a journal line or its fields. **[inferred]** = reasoned from the logs or the source, not seen directly.
Redaction: the Phone's address is `<address>`. The info-level journal carries no phone numbers or Contact names.

**Bottom line:** Call 3 ran 16 min 11.6 s (17:16:42.165 to 17:32:53.796), with the audio on the Desktop for 16 min 6.9 s after a Handoff to the Desktop. Hands-free never dropped, and WirePlumber and PipeWire logged nothing during the Call. The Handoff took 2.50 s by the app's own clock, over the 1.5 s scorecard target. Throughout the Call, and for five days before it, the old build kept calling `Device1.Connect` on a Phone that was already connected, about every 45 s. Before that, for 70 hours with the Desktop's Bluetooth off, it called it every 3 s: 84,162 calls, each failing at once.

## Which build

- The service that carried the Call was PID 1505, started at boot on 2026-11-08 14:52:44 and stopped at 17:36:37 today, when the update's `just install` restarted it. **[observed in log]**
- Before that boot, the service last started on 2026-11-05 at 08:18:28, so that is when the old binary was installed. **[observed in log]** for the start time; that the restart came from `just install` is **[inferred]**.
- The old build's journal fields name the source lines it logged from: `service.rs` 58, 375, 469 and 480, `phone_link.rs` 441 and 509, `bluez.rs` 202, and `surfaces.rs` 292. Every one is a `tracing::info!` at `c5db475` (and at `c6ff55d`, main's tip at 08:16, which changed only the wiki). At `0ba24c9`, the commit before, and at `0dd8734`, the next app commit (merged at 08:26, after the restart), they are not. **[observed]** in the journal fields and `git show`. That the build was `c5db475` is **[inferred]** from those matches and the times. Both builds report `VERSION` 0.1.0, which doesn't tell them apart. **[observed in log]**
- So the Call ran on a build without about eight days of fixes, among them call waiting (#129), every Dialer opening logged for 7.4 (#135), a lost connection shown on every surface (#151), the Hang up resent across a Telephony restart (#176), and the Sol audit fixes (#193). **[inferred]** from `git log c5db475..origin/main`.

## Timeline

All times are local (EST), from the journals.

| Time | What | Label |
|---|---|---|
| 2026-11-10 19:03:59 | The Desktop's Bluetooth is turned off (`rfkill: block set for type bluetooth`). From 19:04:02 the app's `Device1.Connect` fails with `org.bluez.Error.NotReady: br-connection-adapter-not-powered`, every 3 s | observed in log |
| 17:16:22.833 | The Dialer draws its first frame. No `activate-requested` line comes before it: this build logged only the keybinding's opening (#135 later logged every opening) | observed in log; the reason inferred |
| 17:16:37.868 | The last `adapter-not-powered` failure | observed in log |
| 17:16:38.542 | `rfkill: unblock set for type bluetooth`: the Desktop's Bluetooth comes back on | observed in log |
| 17:16:42.065 | bluetoothd: `a2dp.c:load_remote_sep() Unable to load LastUsed: rseid 16 not found`, as the Phone connects | observed in log |
| 17:16:42.107 | `The Phone is connected` (`connection-state-changed`), 3.56 s after the unblock. A Contacts sync starts in the same instant | observed in log |
| 17:16:42.165 | `Call 3 appeared, active`. No ringing or alerting state came first, so the Call was already live on the Phone: dialled or answered there, before or as hands-free came up. Whether it was incoming or outgoing isn't in the info-level log | observed in log; the cause inferred |
| 17:16:42.181 | The Call Window draws its first frame | observed in log |
| 17:16:44.173 | `Asked for a Handoff to the Desktop` (`handoff-requested`) | observed in log |
| 17:16:46.674 | `The Call audio is on the Desktop` (`audio-moved`), 2.501 s after the request | observed in log |
| 17:17:12.787 | GTK warning `gdk_frame_timings_throttling_hint: runtime check failed: (self->result != GDK_FRAME_PREPARING)`, then 2 ms later the Dialer draws its first frame, opened during the Call | observed in log |
| 17:17:19.804 | `Synced 319 Contacts and 951 Call History entries`, 37.7 s after the sync started | observed in log |
| 17:17:28.634 | The first of 21 Connect failures during the Call, each `org.bluez.Error.Failed: le-connection-abort-by-local`, every 45.06 s from then until the restart (26 in all) | observed in log |
| 17:17:30.037 | `Read the rest of the Call History: 951 entries` | observed in log |
| 17:32:53.602 | `The Call audio is on the Phone` (`audio-moved`) | observed in log |
| 17:32:53.604 | Kernel: `Bluetooth: hci0: corrupted SCO packet` | observed in log |
| 17:32:53.796 | `Call 3 ended`, from the branch of `phone_link.rs` that handles the Phone's `State` turning `disconnected` | observed in log (line 441); the branch read at `c5db475` |
| 17:36:37.799 | The update restarts the service. The new build logs `phone-present` and is connected at 17:36:38.169, and logs no Connect failure through at least 18:10 | observed in log |
| 17:36:40.333 | The new build reads 952 Call History entries, one more than before: the Call is in the Phone's Call History | observed in log; that the new entry is Call 3 is inferred |

What the log can't say:

- Who ended the Call. This build logged neither Hang up nor the Phone's own end at info level. **[inferred]** from `c5db475`'s source.
- Whether Mute was used. **[inferred]**: this build logs neither Mute nor `MicrophoneVolume` at info level.
- Whether wideband speech was negotiated (scorecard item 3). Nothing in WirePlumber's, PipeWire's or bluetoothd's journal names a codec. **[observed in log]**: absent.

The audio moving to the Phone 193 ms before the Call ended, in the same millisecond as the corrupted SCO packet, reads as the audio link closing at hang-up rather than a Handoff anyone asked for. No `handoff-requested` line comes before it. **[inferred]**

## Real-Phone Checklist steps

Logs can support a step or count against it. Only the user can pass one, by running `just real-phone-checklist`, which writes results under `~/.local/state/desktop-calls/real-phone-checklist/`. That folder doesn't exist yet, so no step has a recorded result. **[observed]**

- **1.4, a second page of Call History (#31): supports.** The sync read 951 Call History entries, and "Read the rest of the Call History" came 10.2 s after the first part. **[observed in log]** That more than one page was read, at 50 a page, is **[inferred]**. The user's `just pbap-probe` on 2026-11-05 already showed the Pixel honouring `Offset`. The user still has to run the step.
- **2.2, answer on the Pixel and the audio stays there (#28): supports, if the Call was incoming.** The Call appeared already active, and the audio stayed off the Desktop until the user's own Handoff 2 s later. **[observed in log]** Whether the Call was incoming is **[inferred]** at best: the log doesn't say. The step also wants the Banner closing, and no Banner was shown because the Call was never seen ringing. **[observed in log]** The user still has to run the step.
- **3.1, Handoff to the Phone and back, twice (#29): half, once.** One Handoff to the Desktop, which took 2.50 s. **[observed in log]** No Handoff to the Phone was asked for. The audio moving to the Phone at the end was the Call ending (**[inferred]**, above). Confirming by ear is the user's. The user still has to run the step.
- **3.2, Mute and unmute (#29): no evidence.** This build doesn't log Mute at info level. **[inferred]** from the source.
- **3.3, keypad tones (#29): no evidence.** No such line in the log. **[observed in log]**
- **6.1, out of range and back, reconnect timed (#30): related, not the step.** The Desktop's Bluetooth (not the Pixel's) came back on and the Phone was connected 3.56 s later, inside the 10 s target. **[observed in log]** Whether the app's Connect (due about 17:16:40.87, 3 s after the last failure) brought the Phone back, or the Pixel connected by itself because it had a live Call, is **[inferred]** either way: that Connect failed only at 17:17:28 on its LE leg. The step measures the Pixel leaving and returning, so the user still has to run it.
- **7.3, the Indicator opens the Dialer (#28): no evidence.** The Dialer opened twice without the keybinding's line, so something else opened it, perhaps the Indicator. **[inferred]**, weakly.
- **7.4, the Dialer's first frame within 150 ms (#28): supports, from after the update.** The old build didn't log these openings. **[observed in log]** After the update, at 17:36:41, `activate-requested` to `surface-shown` took 19.6 ms. **[observed in log]** That opening was outside the Call, after the idle first-frame work, and the step wants the timing judged at the step. The user still has to run it.
- **Not a step: a long Call holds.** 16 minutes of Desktop audio with no hands-free drop, no connection-lost state, and no WirePlumber or PipeWire message. **[observed in log]** Call quality by ear is the user's to judge.

## The four open issues

- **#112 (the Phone drops hands-free mid-Call by itself):** no drop happened. "Call ended" came from the Phone reporting the Call's `State` as `disconnected`, not from the Call vanishing. **[observed in log]** for the line, **[inferred]** for the branch, read at `c5db475`. One 16-minute Call without a drop is one sample, and it doesn't answer #112's question (what BlueZ and PipeWire report when the user turns "Phone calls" off mid-Call). No change to #112.
- **#109 (paging cost and back-off):** the strongest evidence today, in two parts.
  - **Bluetooth off.** From 2026-11-10 19:04:02 to 2026-11-13 17:16:37, the app called `Device1.Connect` every 3 s with the Desktop's Bluetooth off: 84,162 calls, each failing at once with `br-connection-adapter-not-powered`. **[observed in log]** With the adapter off, those calls can't have paged anything over the air, so they cost CPU and journal space, not radio time. **[inferred]** Each failure wrote a journal line: 20,737 today alone. The service used 13 min 50 s of CPU over its 5 d 2 h run. **[observed in log]** How much of that the loop took is **[inferred]** and unmeasured. Current `main` reads the adapter's `Powered` only for the setup checks, not before paging. **[inferred]** from `grep`; not run.
  - **A connected Phone.** Through the old build's run, outside the Bluetooth-off stretch (2026-11-08 to 11-10, and today through the whole Call), `Device1.Connect` went on failing with `le-connection-abort-by-local` about every 45 s, whether the app showed the Phone connected or not: 3,970 times in all, 21 of them during the Call. **[observed in log]** At `c5db475` the app calls `Device1.Connect` only while it holds the Phone as absent (`Reach::Absent`). So it never registered the Phone as present again after its first absence, though BlueZ was connected. **[inferred]** from the source. Why it never did isn't in the log: that build logged `phone-present` at debug level only. **[observed]** in the source. Each call returned after about 42 s, which looks like BlueZ trying the Phone's LE side because BR/EDR was already up. **[inferred]** On 2026-11-10, bluetoothd logged `Unable to get Hands-Free unit SDP record: Host is down` about 8 s after each failed Connect. **[observed in log]** The new build connected once at startup and logged no further Connect through 18:10. **[observed in log]** It hasn't yet been seen through a Phone leaving and coming back.
  - What it means for #109's questions: the Call's own SCO audio ran for 16 minutes beside a Connect attempt every 45 s. Whether it stuttered at those moments, the user alone can say. No log line shows a dropout. **[observed in log]**: absent. The 3 s schedule becomes about 45 s in practice whenever BlueZ spends 42 s on each attempt. **[inferred]** The A2DP line at connection (`load_remote_sep`) hints that the Connect, or the Phone, brought up A2DP too. **[inferred]**, weakly.
- **#51 (Mute falls back to the Desktop microphone):** no evidence. The old build logs neither Mute nor `MicrophoneVolume` at info level, so whether the user muted is unknown. **[inferred]** from the source. The user can say whether they muted and whether the far end heard silence.
- **#104 (a Call whose number came late returns without one):** not reached. Call 3 appeared once and never left the bus: there was no `call-returned`, no Telephony restart, and no WirePlumber journal entry all day. **[observed in log]** Whether its number came late isn't logged at info level. No change to #104.

## Errors, warnings and oddities in the window, with proposed ticket titles

None are filed.

1. **A connected Phone was paged about every 45 s, through the Call** (`le-connection-abort-by-local`, above). **[observed in log]** Proposed: "Confirm the Desktop stops calling Device1.Connect once a Phone that was away connects by itself (seen every 45 s for five days, through a Call, on `c5db475`)".
2. **Paging every 3 s for 70 hours with the Desktop's Bluetooth off** (84,162 lines). **[observed in log]** Proposed: "Don't page the Phone while the Desktop's Bluetooth adapter is off; connect when it powers on".
3. **The Handoff to the Desktop took 2.50 s by the app's clock**, against the 1.5 s scorecard target (item 2, which holds Handoffs to it) and 219 to 281 ms on the real Pixel on 2026-11-05. It came 2 s after hands-free connected, with a Contacts sync starting at the same time. **[observed in log]** Whether either slowed it is **[inferred]**. Proposed: "A Handoff to the Desktop just after hands-free connects took 2.5 s on the real Phone (target 1.5 s)".
4. **The audio moved to the Phone 193 ms before the Call ended**, with the kernel's `Bluetooth: hci0: corrupted SCO packet` in the same millisecond. **[observed in log]** Whether the Call Window showed "audio on the Phone" for that moment is **[inferred]** and unchecked. Proposed: "When the Call ends, the audio link closing is logged, and may be shown, as the audio moving to the Phone".
5. **The kernel's `corrupted SCO packet` itself**, once, at the Call's end. **[observed in log]** Probably harmless at teardown. **[inferred]** Proposed: "Record whether hci0's 'corrupted SCO packet' at Call end recurs on real Calls (AX201)".
6. **GTK warning** `gdk_frame_timings_throttling_hint: runtime check failed: (self->result != GDK_FRAME_PREPARING)` as the Dialer opened during the Call. **[observed in log]** Proposed: "GTK frame-timings runtime check fails when the Dialer opens over a live Call Window".
7. **The Contacts sync during the Call's first minute took 37.7 s**, against 9.7 s for the phonebook in the 2026-11-05 probe, and pulled over the same Bluetooth link as the Call's audio. **[observed in log]** Any effect on the audio is **[inferred]**, unknown. Proposed: "Measure whether a Contacts sync during a live Call slows or disturbs it (38 s on 2026-11-13; 10 s idle)".
8. **bluetoothd** `a2dp.c:load_remote_sep() Unable to load LastUsed: rseid 16 not found` as the Phone connected. **[observed in log]** A BlueZ-internal note about a cached endpoint. **[inferred]** Proposed: "Note BlueZ's 'Unable to load LastUsed' at Phone connect as known noise in telephony-api-quirks".
9. **Vulkan warnings at every start** (`Unable to open device /dev/dri/renderD130: Permission denied (VK_ERROR_INCOMPATIBLE_DRIVER)` and `Unable to open device : No such file or directory`), at 17:36:38 and at the 2026-11-08 start. **[observed in log]** Which renderer GTK then used isn't logged. The wiki's guess that the real session renders with Vulkan on the RTX 4080 SUPER is open. **[inferred]** Proposed: "Find which GTK renderer the installed app uses on the Desktop, given Vulkan's device-open warnings at every start".
10. **The service's journal volume:** 20,737 lines today, all but 42 of them the Connect failure. **[observed in log]** Proposed: "Log a repeated Connect failure once per change of cause, not every attempt".
11. **No `activate-requested` for the two Dialer openings** in the old build. **[observed in log]** Already fixed by #135 (`0b2c59c`, "every Dialer opening logged for 7.4"). **[inferred]** from the commit subject. No ticket.
12. **The omarchy-shell Bluetooth panel** logged `Binding loop detected for property "height"` at 17:18:27. **[observed in log]** Omarchy's, not this project's. No ticket.

Outside the window, for context: on 2026-11-09 at 08:00:00, WirePlumber logged `Failed to create BT node bluez_input.<address>.2` (`mod.adapter: can't get format: Invalid argument`), 35 s before the app logged the Phone disconnected. **[observed in log]** Proposed, if it recurs: "WirePlumber failed to create the Phone's Bluetooth input node once (2026-11-09)".

## For the user

Three answers only you can give would turn this log into checklist evidence:

1. Did the Call's audio stutter, about every 45 s or at any time? (#109)
2. Did you mute, and did the far end hear silence? (#51, step 3.2)
3. Was the Call incoming or outgoing, and who hung up?
