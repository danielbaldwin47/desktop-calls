# Call waiting

Date: 2026-11-05. Ticket #16 (scenarios 14 and 15). Only the Simulated Phone was used. Call waiting on the real Pixel needs two Helpers, so it is Real-Phone Checklist group 4 (steps 4.1 to 4.4).
Legend: **[V]** = verified, by reading PipeWire 1.6.9's source or against the Simulated Phone, as stated. **[I]** = inferred, not tested.

**Bottom line:** PipeWire 1.6.9 has everything call waiting needs, as three AudioGateway methods that each send one `AT+CHLD`. The app uses them, and nothing else. 14 core tests and 10 Simulated Phone scenarios pass.

## 1. What PipeWire implements

All of these are in `spa/plugins/bluez5/backend-native.c` and `telephony.c`, at tag 1.6.9.

- **[V]** `AudioGateway1.HoldAndAnswer` sends `AT+CHLD=2`. It needs an active Call and a waiting Call, or it replies `InvalidState`.
- **[V]** `AudioGateway1.ReleaseAndAnswer` (End & answer) sends `AT+CHLD=1`. It needs an active Call and a waiting Call, or it replies `InvalidState`.
- **[V]** `AudioGateway1.SwapCalls` sends `AT+CHLD=2`. It needs a held Call, or it replies `InvalidState`.
- **[V]** All three reply `NotSupported` when the Phone didn't list `CHLD` support, and `InProgress` while another AT command is outstanding.
- **[V]** `Call1.Answer` works only on an `incoming` Call. It refuses a `waiting` one, so a waiting Call can only be answered with `HoldAndAnswer` or `ReleaseAndAnswer`.
- **[V]** `Call1.Hangup` sends a different command depending on the Call's state:
  - On an active, dialing, alerting or incoming Call while another Call is held, it sends `AT+CHLD=1`. That ends the active Call, and the held one becomes active.
  - With no held Call, it sends `AT+CHUP`.
  - On a held or waiting Call, it sends `AT+CHLD=0`.
- **[I]** In 3GPP 27.007, `AT+CHLD=0` rejects a waiting Call before it releases held Calls. So with three Calls (active, held and waiting), hanging up the held one could decline the waiting one instead. The app never puts three Calls in that state itself, and the Simulated Phone doesn't model it.
- **[V]** A waiting Call arrives in state `waiting`, from `+CCWA` or `+CLCC`. `callheld` changes swap the active and held Calls.
- Multiparty (`CreateMultiparty`, `HangupMultiparty`) is there too, but it is out of scope for #16.

## 2. What the app does

- A Call that rings while another is connected is the waiting Call. The Banner offers Hold & answer, End & answer and Decline for it, and the Desktop ringtone stays off.
- Hold & answer and End & answer answer here, so the audio comes to the Desktop, just as Answer does. Answer on a waiting Call acts as Hold & answer, the choice that loses nothing.
- Decline hangs up only the waiting Call (`AT+CHLD=0`).
- The Call Window shows the Active Call, and lists the other Call below Hang up as on hold, with Swap (S) and its own Hang up (Ctrl+Shift+H). A Call left alone on hold shows "On hold", with Resume in place of Swap.
- When the Phone refuses, the Call Window says so for 6 s, for example: "Your phone didn't put your Call on hold, so both Calls are as they were; answer on your phone instead." The refusal goes out as the `Error` signal too. Both Calls stay as they were, and the Banner comes back.
- The control interface has three new methods: `HoldAndAnswer`, `EndAndAnswer` and `Swap`. When no Call is waiting or held, they fail at once with `NoWaitingCall` or `NoHeldCall`. `HangUp(id)` already existed.

## 3. Stacking the Banner and the Call Window

This answers the comment on #16 that came from #7: with call waiting, both surfaces show at once in the top-right corner. The Banner sits below the Call Window, 12 px under it. The Call Window never moves under the pointer, and the Banner never covers it. The Banner's top margin follows the Call Window's natural height, which is recomputed on every render and whenever the Call Window updates, so showing the keypad pushes the Banner down too. **[V]** on screenshots: `the_waiting_calls_banner_shows_below_the_call_window_which_stays_put` fails when the stacking is turned off.

## 4. The Simulated Phone

- A Call that rings while another is active, held, dialing or alerting registers as `waiting`.
- `HoldAndAnswer`, `ReleaseAndAnswer` and `SwapCalls` change the Calls as PipeWire would, and refuse with `InvalidState` when the Calls they need aren't there. `Hangup` follows the `AT+CHLD=1` rule above.
- **[I]** A waiting Call left with no Call the user is on rings again as `incoming`, as the network presents it again.
- The new quirk `hold-refused` refuses `HoldAndAnswer` and `SwapCalls` whatever the state, standing for a network without call hold.

## 5. For the Real-Phone Checklist

Group 4 now asks for two Helpers: one takes the user's Call on the Test Number's phone, and the other calls in during it. Step 4.4 was added: the second Call rings out and a Missed Call notification appears. What to watch for on the Pixel:

- whether Android reports the waiting Call through `+CCWA`, so it shows up as `waiting`;
- whether `AT+CHLD=2` is accepted;
- whether the held Call comes back as `active` after the live one is hung up.
