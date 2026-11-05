# Call waiting

Date: 2026-11-05. Ticket #16 (scenarios 14 and 15). Only the Simulated Phone was used. Call waiting on the real Pixel needs two Helpers, so it is Real-Phone Checklist group 4 (steps 4.1 to 4.4).
Legend: **[V]** = verified, by reading PipeWire 1.6.9's source or against the Simulated Phone, as stated. **[I]** = inferred, not tested.

**Bottom line:** PipeWire 1.6.9 has what call waiting needs, as three AudioGateway methods that each send one `AT+CHLD`. The app uses them, plus `Hangup` and `Answer` for a Waiting Call that rings while the user's own Call is still being placed. With a Waiting Call ringing, `AT+CHLD` acts on it first, so the app offers neither Swap nor Hang up for a Held Call then. 21 core tests and 14 Simulated Phone scenarios pass. Nothing here has been tried on the Pixel.

## 1. What PipeWire implements

All of these are in `spa/plugins/bluez5/backend-native.c` and `telephony.c`, at tag 1.6.9.

- **[V]** `AudioGateway1.HoldAndAnswer` sends `AT+CHLD=2`. It needs an `active` Call and a `waiting` Call, or it replies `InvalidState`.
- **[V]** `AudioGateway1.ReleaseAndAnswer` (End & Answer) sends `AT+CHLD=1`. It needs the same, or it replies `InvalidState`. A `dialing` or `alerting` Call doesn't count as active.
- **[V]** `AudioGateway1.SwapCalls` sends `AT+CHLD=2`. It needs a `held` Call, or it replies `InvalidState`. It doesn't check for a `waiting` Call.
- **[V]** All three reply `NotSupported` ("Method is not supported on this Audio Gateway") when the Phone didn't list `CHLD` support, before they look at the Calls, and `InProgress` while another AT command is outstanding.
- **[V]** `Call1.Answer` works only on an `incoming` Call. It refuses a `waiting` one with `InvalidState`, so a Waiting Call can only be answered with `HoldAndAnswer` or `ReleaseAndAnswer`.
- **[V]** `Call1.Hangup` sends a different command depending on the Call's state:
  - On an `active`, `dialing`, `alerting` or `incoming` Call while another Call is `held`, it sends `AT+CHLD=1`.
  - With no `held` Call, it sends `AT+CHUP`.
  - On a `held` or `waiting` Call, it sends `AT+CHLD=0`.
- **[V]** Without `+CLCC`, PipeWire's own handling of `+CIEV: callheld,0` destroys a `waiting` Call first, and `held` Calls only if there was none.
- **[I]** In 3GPP TS 27.007, `AT+CHLD=0` rejects a waiting Call before it releases held ones, `AT+CHLD=1` and `AT+CHLD=2` accept a waiting Call before a held one. So with three Calls (one active, one held, one waiting):
  - `Hangup` on the held Call declines the waiting one, and the held Call stays.
  - `SwapCalls` answers the waiting Call, not the held one.
  - `Hangup` on the active Call answers the waiting Call, not the held one.
- **[V]** A Waiting Call arrives in state `waiting`, from `+CCWA` or `+CLCC`. With `+CLCC`, its state follows whatever the Phone reports next, so it can go from `waiting` to `incoming`.
- Multiparty (`CreateMultiparty`, `HangupMultiparty`) is there too, but it is out of scope for #16.

## 2. What the app does

- A Call that rings while another is live (`active`, `held`, `dialing` or `alerting`) is the Waiting Call. The Banner, headed "Waiting Call", offers Hold & Answer, End & Answer and Decline for it, never plain Answer, and the Desktop ringtone stays off.
- Hold & Answer and End & Answer answer here, so the audio comes to the Desktop, just as Answer does. Answer on a Waiting Call acts as Hold & Answer, the choice that loses nothing.
- Decline hangs up only the Waiting Call (`AT+CHLD=0`).
- While the user's own Call is still being placed, nothing is active: PipeWire refuses `HoldAndAnswer` and `ReleaseAndAnswer`, and a Call nobody has answered can't be held anyway. The Banner offers End & Answer and Decline only. End & Answer hangs up the Call being placed (`AT+CHUP`), then answers the Waiting Call with `Answer` once the Phone presents it as `incoming`. Hold & Answer over the control interface fails with `NotConnectedYet`, and Answer acts as End & Answer.
- The Call Window shows the Active Call, and lists the other Call below Hang up as on hold, with Swap (S) and its own Hang up (Ctrl+Shift+H). A Call left alone on hold shows "On hold", with Resume in place of Swap.
- While a Waiting Call rings, the Held Call's row offers neither Swap nor Hang up, and says instead: "Swap, or hanging up this Call, would act on the Waiting Call instead; answer or decline it first." `HangUp` on the Held Call reports `waiting-call-first` with the same sentence, and `Swap` fails with `WaitingCallFirst`. The Phone isn't asked.
- When the Phone refuses, the Call Window says so for 6 s, for example: "Your phone didn't put your Call on hold, so both Calls are as they were; answer on your phone instead." The refusal goes out as the `Error` signal too. Both Calls stay as they were, and the Banner comes back.
- The control interface has three new methods: `HoldAndAnswer`, `EndAndAnswer` and `Swap`. They fail at once with `NoWaitingCall`, `NoHeldCall`, `NotConnectedYet` or `WaitingCallFirst`. `HangUp(id)` already existed.

## 3. Stacking the Banner and the Call Window

This answers the comment on #16 that came from #7: with call waiting, both surfaces show at once in the top-right corner. The Banner sits below the Call Window, 12 px under it. The Call Window never moves under the pointer, and the Banner never covers it. The Banner's top margin follows the Call Window's natural height, which is recomputed on every render and whenever the Call Window updates, so showing the keypad pushes the Banner down too. **[V]** on screenshots: `the_waiting_calls_banner_shows_below_the_call_window_which_stays_put` fails when the stacking is turned off.

## 4. The Simulated Phone

- A Call that rings while another is `active`, `held`, `dialing` or `alerting` registers as `waiting`.
- `HoldAndAnswer`, `ReleaseAndAnswer` and `SwapCalls` change the Calls as PipeWire would, and refuse with `InvalidState` when the Calls they need aren't there.
- With three Calls it follows 27.007 as in section 1: `Hangup` on the `held` Call ends the `waiting` one, and `SwapCalls`, or `Hangup` on the `active` Call, makes the `waiting` Call `active`.
- **[I]** A Waiting Call left with no Call the user is on rings again as `incoming`, as the network presents it again.
- The quirk `chld-unsupported` stands for a Phone that didn't list `CHLD` support: `HoldAndAnswer`, `ReleaseAndAnswer` and `SwapCalls` reply `NotSupported` whatever the state, as PipeWire does.

## 5. Unverified on the Pixel

- Whether Android reports a Waiting Call through `+CCWA` or `+CLCC`, so that it shows up as `waiting`, including while the user's own Call is still `dialing` or `alerting`.
- Whether the Pixel lists `CHLD` support and accepts `AT+CHLD=2` and `AT+CHLD=1`.
- How a refusal arrives: `NotSupported` straight from PipeWire, or the Phone's `ERROR` after the AT command, and whether the Calls then stay as they were.
- Whether the Held Call comes back as `active` after the live one is hung up.
- Whether a Waiting Call goes `incoming` once the user's own Call being placed is hung up. If it stays `waiting`, End & Answer leaves it ringing with no Banner.
- What the Pixel does with three Calls.

## 6. For the Real-Phone Checklist

Group 4 now asks for two Helpers: one takes the user's Call on the Test Number's phone, and the other calls in during it. Step 4.4 was added: the second Call rings out and a Missed Call notification appears. The points in section 5 are what to watch for.
