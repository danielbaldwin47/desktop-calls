# Placing a Call from the Desktop: Dial, Desktop audio and the Phone's refusals

Date: 2026-11-04. Ticket #10. Stack as in the [conformance note](2026-11-04-conformance.md): PipeWire 1.6.9 Telephony, BlueZ 5.87, the Pixel 9a; the screen harness is sway 1.12 headless.
Legend: **[V]** = verified on this machine. **[S]** = read in the PipeWire 1.6.9 source (`spa/plugins/bluez5/telephony.c`, `backend-native.c`), not run against the Pixel. **[I]** = inferred, not tested.

**Bottom line:** one real Call to the Test Number, placed with `RejectSCO` cleared first, brought its audio to the Desktop without any `Activate()`: the Pixel opened the audio link itself while the Call was still `alerting`. So placing a Call here is `Set(RejectSCO=false)`, then `Dial(number)`, and nothing more. The Phone's refusals reach the Desktop only as PipeWire's error names and description strings, which the app turns into one plain sentence each.

## 1. One real Call, placed with Desktop audio

`tools/real-phone/test_call.py` gained `--desktop-audio`: it clears `RejectSCO` before `Dial`, logs each change of the transport's `State`, `Codec` and `RejectSCO` while the Call connects and is held, and sets `RejectSCO` back to true after hanging up (its guard process does too, if the tool dies). One Call, `--desktop-audio --hold 3`. Times are from just before `Dial`: **[V]**

| t (s) | What happened |
|---|---|
| before | `RejectSCO` set to false on `ag1` |
| 0.641 | `Dial` returned (empty reply) |
| 0.742 | The Call appeared, straight in `alerting` (never `dialing`, as in the earlier Handoff probe) |
| 1.345 | Transport `pending`, `Codec` 3 (LC3-SWB) |
| 1.446 | Transport `active` |
| 2.052 | The Call went `active` |

The tool exited 0, `GetCalls` was empty afterwards, and `RejectSCO` was back to true. The tool redacts the Test Number from everything it prints.

- No `Activate()` is needed for a Call placed here: with `RejectSCO` false, the Pixel opens the audio link during `alerting`. **[V]**
- `Dial` replies only once the Phone answers `ATD` with `OK` or `ERROR` (`ag_dial` → `rfcomm_send_cmd`, `backend-native.c`). On the Pixel that took 0.64 s, and the Call appeared 0.1 s later. **[V]** for the timing, **[S]** for the cause.
- The app gives the Phone 15 s to report the dialled Call before it says the Call wasn't placed (`DIAL_TIMEOUT_MS`). **[I]**: 20 times the Pixel's time, never measured on a slow network.

## 2. What `Dial` checks, and how it fails

- `validate_phone_number` (`telephony.c:911`) accepts only `0`-`9`, `A`-`D`, `#`, `*`, `+` and `,`, 1 to 80 of them. Anything else is `org.pipewire.Telephony.Error.InvalidFormat`, "Invalid phone number or tones", before the Phone is asked. **[S]** The app's own check (`core::matching::dialable`) accepts exactly this, after dropping spaces, `-`, `.`, `(`, `)` and a `tel:` link's scheme and parameters, so the user hears about a typo before the Phone does.
- A non-`OK` reply to `ATD` is `org.pipewire.Telephony.Error.Failed`, "Method call failed", unless it is `+CME ERROR: N` (`backend-native.c:511-520`). **[S]**
- A `+CME ERROR` is `org.pipewire.Telephony.Error.CME`, and **its message is PipeWire's description of the code, not the code** (`telephony_error_to_description`, `telephony.c:275-317`): 26 "dial string too long", 27 "invalid characters in dial string", 30 "no network service", 31 "network Timeout", 32 "network not allowed - Emergency calls only", SIM codes 10 to 18, and "Unknown CME error" for any other. A client can only match these strings. **[S]**
- The other errors a method can return: `NotSupported` "Method is not supported on this Audio Gateway", `InvalidState` "The current state does not allow this method call", `InProgress` "Command already in progress". **[S]** None was seen from the Pixel; airplane mode and no signal were not tried, as they would need the user's Phone changed.

How the app maps them (`phone_link::dial_refusal`, then the Messages text):

| From PipeWire | Failure kind | What the user reads |
|---|---|---|
| `InvalidFormat`; CME 26, 27 | `number-refused` | Your phone can't dial that number; check it and try again. |
| `InvalidState`, `InProgress` | `phone-busy` | Your phone is busy with another Call; try again in a moment. |
| `NotSupported` | `dial-not-supported` | Your phone doesn't let this computer place Calls; call from your phone instead. |
| CME 30, 32 | `no-signal` | Your phone has no mobile signal, or is in airplane mode. |
| CME 31 | `network-error` | The mobile network didn't answer your phone; try again in a moment. |
| `Failed`, any other CME, no reply, the 15 s timeout | `dial-failed` | Your phone couldn't place the Call; check it has signal and isn't in airplane mode. |

The raw error name and description go only to the log, as `detail` on the `phone-command-failed` event.

## 3. The Simulated Phone

- `Dial` on `AudioGateway1` or `org.ofono.VoiceCallManager` validates as PipeWire does, then replies with no Call yet. `RefuseNextDial` makes the next one fail with any error above, with PipeWire's description as the message.
- `Alert` registers the dialled Call `alerting` and, while `RejectSCO` is false, brings the transport up (codec, `pending`, `active`) as the Pixel did in §1. `CalleeAnswers` makes it `active`.
- Quirk `get-calls-empty-while-dialling`: `GetCalls` leaves out `dialing` and `alerting` Calls. The app never calls `GetCalls` (it follows `ObjectManager` signals), so the quirk cannot make the Call Window say "Call ended"; the scenario guards that. **[V]**

## 4. Environment notes

- `just test-call` failed with `ModuleNotFoundError: No module named 'dbus'`: the mise `python3` shim comes first on `PATH`, and `dbus-python` is installed only for `/usr/bin/python3`. Running `/usr/bin/python3 tools/real-phone/test_call.py …`, which is what the recipe runs, worked. **[V]** The recipe could call `/usr/bin/python3`, or the tool could say which interpreter lacks `dbus`.
- The new screen scenarios pass under the CI-like loop of the [CI latency note](2026-11-04-ci-banner-latency.md) (one CPU, four at a time, `/dev/dri` hidden, 1 GB `/dev/shm`). Each step waits for the app's `Surfaces` before the Simulated Phone moves on, so no transient state is raced. **[V]**

## 5. For the Real-Phone Checklist

- Place a Call to a real number from `desktop-calls dial`, and confirm "Calling…", "Ringing…", the timer, and the audio on the Desktop. Only the tool's Call (§1) was placed on the Pixel; the app's own `Dial` has run only against the Simulated Phone. **[I]**
- With the Phone in airplane mode, confirm which error `Dial` returns (expected: CME 30 or `Failed`). **[I]**
