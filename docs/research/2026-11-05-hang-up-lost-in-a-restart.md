# A Hang up lost in a Telephony restart

Date: 2026-11-05. Ticket #175, extending [ADR-0007](../adr/0007-a-vanished-call-is-settled-by-the-phones-calls-once-hands-free-is-back.md).
Legend: **[V]** = verified against the Simulated Phone on this machine. **[I]** = inferred, not tested. Nothing in this note was tried on the real Pixel.

**Bottom line:** the Phone link took a hung-up Call that went away as ended, even without `disconnected`. If the Hang up never reached the Phone, the Call came back as a new Call that nobody hung up. Now such a Call is a vanished Call: the Hang up goes again when the Call comes back. If the Telephony service is away longer than the 10 s the Desktop waits, the user is told the Call may still be live on the Phone. The Hang up goes again only to a Call that can be the same one (ADR-0007 point 6). **[V]**

## How the bus answers a Hang up the service never read

- dbus-daemon answers a method call left unanswered by a connection that closes with `org.freedesktop.DBus.Error.NoReply` "Message recipient disconnected from message bus without replying". **[V]** in the test bus. A call sent once the name has no owner gets `ServiceUnknown` "The name org.pipewire.Telephony was not provided by any .service files". **[V]**
- zbus 6 turns both into `zbus::Error::FDO` variants, not `MethodError`. **[V]** from its source.
- The user's session bus is dbus-broker 37 (`dbus-broker-launch --scope user`), not dbus-daemon, which the tests run. **[V]** The binary holds the string "Remote peer disconnected" beside `org.freedesktop.DBus.Error.NoReply`. **[V]** That this is the text of a `NoReply` for a peer that leaves is **[I]**: never observed. The app treats a `NoReply` as "the service went away" only if its text says "disconnected", so a `NoReply` that timed out still counts as a refusal.
- A real `WirePlumber` that exits for a restart closes its connection, so a Hang up it never read gets that `NoReply`. **[I]**: the 2026-11-04 restart probe sent no Hang up during the restart.

## Simulated Phone changes

- `restart_telephony` now closes the old service's connection after it leaves the bus, as the old process exiting does. Before, the connection stayed open with nothing reading it, and the Desktop's call would never have been answered. **[V]**
- New quirk `hangup-unanswered-once`: the next `Hangup` is recorded and gets no reply, and the Call carries on, as a `WirePlumber` shutting down never reads it. **[V]**

## Reproduced first

- `telephony_restart::a_hang_up_lost_as_the_service_leaves_is_sent_again_once_the_call_is_back` failed before the fix: `NoReply` became `hang-up-failed` ("Your phone didn't hang up the Call…"), the restarted service listed the Call, and it became Call 2 with no second `Hangup`. **[V]**
- `telephony_restart::a_hang_up_that_never_reaches_the_phone_says_the_call_may_still_be_live` (a 12 s restart) failed before the fix: `call-ended: Call 1 ended: it didn't come back` at 11.2 s, and no `Error`. **[V]**

## Review round 1

- Matching a returning Call by number alone sent the Hang up to the wrong Call. Before the fix, `a_caller_who_rings_straight_back_is_not_declined_as_the_call_hung_up` failed: the ringback logged `call-returned: Call 1 came back` and got `Hangup`, so the caller was declined under the old Call's id. `a_withheld_caller_who_rings_after_a_withheld_call_is_not_declined` failed the same way, with `Hangup` on the new Call. `a_returning_call_with_no_number_is_not_hung_up_on_a_guess` failed with no `Error` within 15 s, since the unnumbered Call that came back was hung up on a guess. All three pass after the fix. **[V]**
- The words changed from "Your phone didn't hang up the Call; hang up on your phone instead." to "The Call may still be live on your phone; check your phone and hang up there if it is.", with the `Error` kind `call-may-still-be-live`. The old words were false when the Phone took the Hang up and hands-free dropped before `disconnected`. `HangUpFailed` stays for a Phone that refuses a Hang up outright. With the old `Failure`, `a_hang_up_that_never_reaches_the_phone_says_the_call_may_still_be_live` fails on the kind `hang-up-failed`. **[V]**

## Follow-up candidates

- A Hang up the Phone took, followed by hands-free dropping before `disconnected` and not coming back within the grace, is reported undelivered: the Phone link doesn't know the reply came. The Call did end. The words say only that it may still be live, so they are still true. This needs a profile drop within milliseconds of the Hang up. **[I]** from the code.
- Declining a ringing Call whose decline is undelivered says so only on the control interface. The Call has no Call Window to show it as lost.

- `HangUpFailed` and other `Error`s reach only the control interface. A Call Window user sees nothing when the Phone refuses a Hang up outright. **[V]** from the code: the Call Window shows only dial, Call Waiting and audio notices.
