# Out of range and back

Date: 2026-11-05. Ticket #19, including its comment from #6.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** The Desktop now follows BlueZ's `Device1.Connected` as the Phone being present or absent, and pages the Phone with `Device1.Connect` while it is absent: at once, then after 1 s, 2 s and every 3 s. On logind's `PrepareForSleep(false)` it pages at once. Out of range changes only the Indicator. A live Call that loses the link carries on on the Phone, and the Call Window says so. A Phone that has forgotten the pairing becomes the "pairing lost" Problem, and paging stops. All of this is verified against the Simulated Phone only. Nothing was tried on the real Pixel: no disconnect, unpair or Bluetooth toggle.

## Decisions

- **Present and absent come from `Device1.Connected`, not from the AudioGateway.** An AudioGateway also goes away during the Handoff bounce (ADR-0005, #14) and on a WirePlumber restart, while the device stays connected. Neither of those counts as out of range, so the 5 s bounce grace and #101's hands-free-not-back case are untouched. **[V]** (`handoff.rs` and `connection.rs` scenarios still pass)
- **The reconnect is `Device1.Connect`, not `ConnectProfile(HFP AG)`.** ADR-0004 names Device1 for reconnection. `Connect` restores whatever the Phone itself allows (its per-device toggles), as the Phone's own auto-reconnect does. **[I]** that on the Pixel this also brings A2DP when "Media audio" is on for the Desktop. The Real-Phone Checklist should look.
- **The backoff is 1 s, 2 s, then capped at 3 s.** A page to a Phone out of range fails after BlueZ's page timeout of 5.12 s **[V]** (the Desktop, earlier in #6). A Phone in range answers within a page scan of 1.28 s **[I]** (Bluetooth Core spec R1 default). So a Phone back just after a failure is connected within about 3 + 1.3 + 1 s, inside the 10 s target. The radio pages at most about 5 s in every 8 while the Phone is away. **[I]** That the Desktop's other Bluetooth devices notice this is unmeasured.
- **Suspend stops paging, and resume pages at once.** It also pages again at once if a page was under way during the resume. **[V]** (core tests; scenario 18)
- **Pairing lost** is a `Connect` failing with `org.bluez.Error.Failed` "br-connection-key-missing", `AuthenticationFailed` or `AuthenticationRejected`. **[I]** from BlueZ's error table; not seen on the Pixel. Any other failure counts as unreachable and is retried. After pairing lost, the Desktop stops paging, because each page could prompt the person holding the Phone to pair. It starts again when the Phone connects anyway, or the pairing is redone on the Desktop. The Problem tells the user how to pair again. The "Pair again" action belongs to the Setup Window (#21).
- **Link lost mid-Call.** When the Phone goes absent with a live Call in the Call Window, the window keeps the Call. It keeps the timer running, removes the audio and Mute controls, and says "Connection to phone lost: the Call continues on your phone." Hang-up, mute, tones and Handoff are refused with that message, because a hang-up held back until the Phone returns could end whatever the user is now doing on the Phone. When the Phone link gives up waiting for the Call, it is dropped quietly: no "Call ended", no Missed Call, no Call History refresh. If the Phone returns with the Call, the Call carries its direction and timer through earlier calls, and the notice clears. The window closes 10 s after the loss if the Phone is still away. If the Phone returns without the Call, it closes 2 s after hands-free is back. **[V]** (core tests; harness scenario)
- **A ringing Call is hidden while the Phone is absent.** No Banner and no ringing, because it can't be answered from here. **[V]** (core test)

## Simulated Phone

- `Disconnect` (the control method) now means out of range. PipeWire unannounces each Call, then the AudioGateway, then sends `ModemRemoved`. BlueZ removes `Battery1` and sets `Connected` to false. The Calls carry on on the Phone (`CallsOnPhone`). When hands-free returns, a new AudioGateway is registered at the next index, followed by `ModemAdded` and the Calls at their new paths. **[V]**
- `Battery1` exists only while hands-free is up, as on the Pixel (#6). The conformance snapshot still matches after `connect()`. **[V]**
- The Handoff bounce now sends the late `ModemRemoved` once `ConnectProfile` has replied, and sends `ModemAdded` after the AudioGateway, matching the #14 prototype trace. **[V]** against the trace's order, not its timing.
- `Device1.Connect`: in range, the Phone connects in 0.6 s. Out of range, it pages for up to 5.12 s, and a Phone that returns while paging answers. A Phone that forgot the pairing fails after 0.3 s with "br-connection-key-missing". The method replies `AlreadyConnected` or `InProgress` as BlueZ does.
- New control methods: `ReturnToRange`, `ForgetPairing`, `CallsOnPhone`, `PrepareForSleep(b)`. For the last, the Simulated Phone owns `org.freedesktop.login1` on the private system bus.

## For the Real-Phone Checklist (user only; agents must not run these)

1. Walk the Pixel out of range with no Call. The Indicator says out of range, and no notification appears. Walk back and time it to `connected`, which should be under 10 s.
2. Suspend the Desktop, take the Phone away, resume, bring it back, and time it to `connected`.
3. During a Call to the Test Number, walk out of range. Check the Call Window's message, check the Call carries on on the Phone, and see what happens on return.
4. Record the real `Connect` errors from the journal (`phone-connect-failed` lines, `detail` field): one out of range, and one after the user forgets the Desktop on the Phone. This confirms "br-connection-key-missing" and the Problem.
5. Check whether `Connect` brings A2DP up, and whether paging every 3 s disturbs other Bluetooth devices on the Desktop.
6. Note how long `Device1.Connected` takes to go false after the Phone leaves (the link supervision timeout).
