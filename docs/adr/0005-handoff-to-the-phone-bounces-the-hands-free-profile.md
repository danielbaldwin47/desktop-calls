---
status: accepted 2026-11-04 (chief of staff)
---

# Handoff to the Phone bounces the hands-free profile through BlueZ

On 2026-11-04 we decided that a Handoff from the Desktop to the Phone disconnects and immediately reconnects the Phone's hands-free profile: `org.bluez.Device1.DisconnectProfile("0000111f-0000-1000-8000-00805f9b34fb")`, then `ConnectProfile` with the same UUID. PipeWire 1.6.9 has `AudioGatewayTransport1.Activate()` but no way to close the Call's audio link, and dropping the hands-free link is the only way the Desktop can close it today. On the real Pixel 9a this kept the Call alive in 5 of 5 trials and cost 0.5 to 1.0 s of Desktop control, while media audio (A2DP) was untouched ([prototype report](../research/2026-11-04-handoff-to-phone-prototype.md)).

This **amends [ADR-0004](0004-bluez-device-api-for-presence-and-pairing.md)**. ADR-0004 allows BlueZ's `Device1` only for presence and pairing and says Call audio goes only through `org.pipewire.Telephony` and WirePlumber. Desktop Calls may now also call `Device1.DisconnectProfile` and `Device1.ConnectProfile` for the hands-free UUID, for this one purpose. Everything else in ADR-0004 stands: Desktop Calls still never registers a Bluetooth profile and never speaks AT commands.

## Considered options

- **Card profile toggle away from `audio-gateway` and back** (ticket candidate B): not tried, because A worked. It would also drop A2DP, since one card profile carries both.
- **A PipeWire patch adding `AudioGatewayTransport1.Deactivate()`** (candidate C): the clean fix, with no loss of control, but it means running a patched system PipeWire until upstream ships it. Worth proposing upstream later; it would replace the bounce.
- **Asking the user to tap the audio button on the Phone's call screen**: the fallback if neither worked. It stays as the message when a bounce fails.

## Consequences

- During a bounce the Call object disappears **without** reaching `State = "disconnected"` and is re-added, already `active`, about 0.5-1.0 s later. The Phone link adapter must report this as one continuous Call (same Call id, same timer), not as an ended Call and a new one. A hang-up pressed in that window is queued until the Call is back.
- The bounce relies on `bluez5.telephony.default-reject-sco = true`: the re-created AudioGateway starts with `RejectSCO = true`, so the Phone's immediate attempts to reopen audio to the Desktop are refused. Without it the audio would come straight back.
- Handoff to the Desktop is `Set(RejectSCO, false)`, then `Activate()`, then `Set(RejectSCO, true)` once the transport is `active`. `Activate()` alone does nothing while `RejectSCO` is `true`. The Phone ignored `Activate()` once in 7 tries, so the adapter waits about 3 s for `State = "active"` and calls `Activate()` once more before reporting failure. _Superseded 2026-11-05 by #70 (PR #76, `8989cbc`): the Desktop asks once more after 1 s if the link hasn't reached `pending`, and reports failure 3 s after the request. A link that reached `pending` but isn't `active` by 3 s fails without a second try (never seen on the Pixel). See `docs/research/2026-11-05-earlier-handoff-retry.md`._
- The Simulated Phone implements `Device1.DisconnectProfile` and `ConnectProfile` for the hands-free UUID and reproduces the signal order recorded in the prototype report, §3.2.
- If upstream PipeWire gains `Deactivate()`, prefer it and supersede this ADR.

_2026-11-05 note (#101, PR #110, `f7a35a6`): when hands-free doesn't return after the bounce, the Desktop retries `ConnectProfile` (after 1 s, 2 s, then every 3 s) outside the bounce itself, while the Call is shown as continuing on the Phone. `ConnectProfile` was chosen over `Device1.Connect` because it is proven on the Pixel and the Phone stays connected. See `docs/research/2026-11-05-handsfree-not-back.md`._

_2026-11-05 note (#21): the Setup Window also calls `ConnectProfile` for the hands-free UUID, once, when a Phone it has just paired connects without hands-free, and again when the user chooses Try again after switching Phone calls on. A refusal tells the user where that switch is. This is a second purpose beyond the Handoff bounce; `DisconnectProfile` is still used only for the bounce. See `docs/research/2026-11-05-setup-window.md`._

_2026-11-05 note (#169, #170): once hands-free is back, a Call that vanished in the bounce is settled by the Calls the Phone lists, not by the 2 s from when it vanished: listed within 3 s, it is the same Call; otherwise it ends. See [ADR-0007](0007-a-vanished-call-is-settled-by-the-phones-calls-once-hands-free-is-back.md)._
