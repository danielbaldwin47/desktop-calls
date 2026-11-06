---
status: accepted 2026-11-05 (chief of staff, tickets #169 and #170)
---

# A vanished Call is settled by the Phone's Calls once hands-free is back

A Call can vanish: its Telephony object goes away without first going `disconnected`. This happens when hands-free drops for a Handoff to the Phone ([ADR-0005](0005-handoff-to-the-phone-bounces-the-hands-free-profile.md)), when the Desktop asks BlueZ for hands-free again after that ([#101](../research/2026-11-05-handsfree-not-back.md)), and when `WirePlumber` restarts. The Call carries on on the Phone the whole time. When hands-free is back, the AudioGateway reappears first and its Calls follow: 30 to 60 ms later on the Pixel, and later still on a loaded Desktop or with a busy `WirePlumber`. Nothing in PipeWire's Telephony API says when the Phone has listed all its Calls.

Until now the Phone link ended a vanished Call when a grace period, counted from the moment it vanished, ran out: 2 s (`GRACE`), or 10 s once the whole service had gone (`SERVICE_GRACE`). In #170 hands-free was back 12 ms before the 2 s ran out, the Call was listed 28 ms after, and the Desktop showed "Call ended" for a live Call and then picked it up as a new Call with a new timer.

On 2026-11-05 the chief of staff decided, on the rule that UX wins: **while a Call is live on the Phone, the Desktop must not declare it ended, or turn it into a new Call, because of local timing.** So:

1. **While hands-free is away, nothing changes.** A vanished Call waits `GRACE` (2 s) from when it vanished, or `SERVICE_GRACE` (10 s) from when the service left the bus. If that runs out first, the core already treats the Call as lost track of, not ended: the Call Window says the connection to the Phone is lost, and a Call that comes back later keeps its direction and timer ([#101](../research/2026-11-05-handsfree-not-back.md)).
2. **Once hands-free is back, a vanished Call is settled by the Calls the Phone lists, not by the time it vanished.** When the followed Phone's AudioGateway is back, every vanished Call's grace is replaced by a new one from that moment:
   - a Call the Phone lists again is the same Call, with its id and timer;
   - a Hang up pressed while it was away is sent to the Call that comes back;
   - a Call the Phone doesn't list ends.
3. **The wait for the listing is bounded: `LISTING_GRACE`, 3 s from hands-free being back.** Some bound is needed because nothing marks the end of the listing. 3 s is 50 times the Pixel's 60 ms and 8 times the 366 ms main-loop stall seen under load in #170. Waiting longer costs only this: a Call that ended on the Phone while hands-free was away shows for up to 3 s more. Ending a live Call costs "Call ended" and a new Call. After a `WirePlumber` restart this bound replaces the rest of `SERVICE_GRACE`, so a Call that ended meanwhile now goes about 3 s after the service is back rather than up to 10 s after it left.

All of this stays in the Phone link (`crates/desktop-calls/src/phone_link.rs`), as ADR-0005 put it: it hides how PipeWire re-registers Calls, and the core sees one continuous Call.

## Extended for a Hang up that never reached the Phone (#175)

On 2026-11-05, ticket #175 extended this decision to a Call the Desktop has already asked to hang up. Going away is no proof that the Hang up worked: a `WirePlumber` restart takes Calls away without `disconnected`, and a Hang up the old service never passed on leaves the Call live on the Phone. On the Pixel, a Call that really is hung up goes `disconnected` first ([prototype note](../research/2026-11-04-handoff-to-phone-prototype.md) §4).

4. **A hung-up Call that goes without `disconnected` is a vanished Call.** It is settled as in point 2: the Hang up goes again to the Call that comes back, and a Call the Phone doesn't list has ended. A Hang up the bus answers with "the service went away" is not reported as refused, since the Call went with the service.
5. **If hands-free isn't back before the grace in point 1 runs out, the Hang up is undelivered.** The Desktop doesn't say "Call ended". It says "The Call may still be live on your phone; check your phone and hang up there if it is.", and the Call Window shows the Call as lost, as for a lost connection. The words hold whether the Hang up was lost, or the Phone took it and hands-free dropped before `disconnected` came. A Call that comes back later keeps its direction and timer, and isn't hung up by itself: by then the user may have taken it on the Phone.
6. **A Hang up goes again only to a Call that can be the same one.** A Call never goes back to ringing, nor to dialling: an `incoming` or `waiting` Call is never a vanished connected or outgoing Call, nor a `dialing` or `alerting` Call a vanished connected one. With a Hang up to send, the number must also be the same, and known: a withheld number, or none, may be anyone. A caller who rings straight back is a new Call, not declined. If a Call comes back that may be the hung-up one with no number to tell, nothing is hung up on a guess: once the listing window runs out, the Desktop says the Call may still be live, as in point 5. Without a Hang up to send, two Calls with no number and states that fit are still taken as the same, since nothing is sent on it and the Call keeps its timer.

## Considered options

- **Widen `GRACE`.** It would only move the edge #170 fell over, and it would also keep a Call that really ended longer while hands-free is away, when the Desktop can't know. Rejected.
- **Stop the clock entirely once hands-free is back, and wait for a "listing complete" signal.** PipeWire has none. Its Calls are registered one by one after the AudioGateway, and the AudioGateway's `GetManagedObjects` and `org.ofono.VoiceCallManager.GetCalls` can only list what is registered (inferred from the API, not observed in that gap). Without a bound, a Call that ended while hands-free was away would never end on the Desktop. Rejected.
- **Extend `GRACE` while the Desktop is reconnecting hands-free itself.** Not needed: when `GRACE` runs out while hands-free is away, the core already keeps the Call Window's lost notice and the timer (#101). Only the end after hands-free is back showed "Call ended".

## #169 was not this race

#169 looked like the same race, but its log says otherwise. The Desktop did keep the Hang up until the Call came back, and sent it to that Call (`call-returned`, then `Hangup`, which the Simulated Phone recorded and carried out). Then the Simulated Phone announced the Call again, `active`, 40 ms after the AudioGateway, from a list it had made before the restart, and the Desktop took it as a new Call 2. The Simulated Phone also listed the Calls in the AudioGateway's `GetManagedObjects` before announcing them. The fix is in the Simulated Phone: after a restart its Calls leave the service with it, and the new service registers them, with their current state, only when it announces them. That a real restarted PipeWire doesn't list Calls before announcing them is inferred, not observed.

## Consequences

- The Simulated Phone's `calls-listed-late` quirk lists the Calls 2 s after hands-free is back, after both a `DisconnectProfile` and a Telephony restart. Scenarios use it to put "hands-free is back" well before "the Calls are listed", so the order these tickets depend on doesn't come down to machine load.
- The core's `LOST_CALL_BACK_MS` (2 s) is unchanged. It applies only to a Call the Phone link already gave up on while hands-free was away.
