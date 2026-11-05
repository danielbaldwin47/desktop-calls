# Desktop Calls

Lets the user take and place their Android Phone's cellular Calls on their Linux Desktop, the way an iPhone's calls ring on a Mac.

## Devices

**Phone**:
The user's Android handset that owns the cellular line.
_Avoid_: device, handset, mobile, AG

**Desktop**:
The Linux computer running Desktop Calls.
_Avoid_: PC, host, computer, HF

**Companion App**:
A planned Android app on the Phone that adds what Bluetooth alone cannot provide. Not part of v1.
_Avoid_: phone app, Android client

## Calls

**Call**:
One cellular voice call on the Phone's line, incoming or outgoing.
_Avoid_: session, line

**Audio Location**:
Where a Call's audio is currently heard and spoken: on the Phone or on the Desktop.
_Avoid_: audio route, output

**Handoff**:
Moving a live Call's Audio Location from Phone to Desktop or back. The Call itself stays on the Phone's line.
_Avoid_: transfer (in telephony, that means sending the call to another number)

**Waiting Call**:
An incoming Call that rings while another Call is already live.
_Avoid_: second call, call waiting (that names the feature, not the Call)

**Held Call**:
A live Call that the user has put on hold, so neither side hears the other until it is resumed.
_Avoid_: paused call, parked call

**Hold & Answer**:
Answering a Waiting Call by putting the live Call on hold.
_Avoid_: accept and hold

**End & Answer**:
Answering a Waiting Call by ending the live Call.
_Avoid_: replace, release and accept

**Swap**:
Switching which of two Calls is live, putting the other on hold.
_Avoid_: toggle, switch calls

**Contacts**:
The Phone's address book, as visible from the Desktop.
_Avoid_: phonebook, address book

**Missed Call**:
An incoming Call that stopped ringing without being answered on either device and without being declined.
_Avoid_: unanswered call

**Call History**:
The Phone's record of recent incoming, outgoing and missed Calls.
_Avoid_: call log, recents

## Desktop surfaces

**Incoming Call Banner**:
The transient on-screen notice for a ringing Call, with Answer and Decline.
_Avoid_: popup, toast, notification

**Dialer**:
The search-as-you-type window for starting a Call from a name or number.
_Avoid_: launcher, call picker

**Call Window**:
The small window shown during a live Call: timer, mute, keypad, Handoff, hang up.
_Avoid_: in-call screen, call UI

**Indicator**:
The icon in the Desktop's top bar that shows whether the Phone is connected or in a Call.
_Avoid_: tray icon, status icon

**Setup Window**:
The guided window that pairs the Phone with the Desktop and walks through granting access to Contacts and Call History.
_Avoid_: first-run wizard, onboarding, "Connect your phone" window

**Do Not Disturb**:
The Desktop's system-wide setting for silencing notifications, which Desktop Calls also obeys.
_Avoid_: DND, quiet mode

## Testing

**Simulated Phone**:
A stand-in for the Phone that the Desktop cannot tell apart from a real one, so Calls can be exercised without real hardware.
_Avoid_: mock phone, fake AG

**Test Number**:
The single phone number, chosen in advance by the user, that agents may Call on their own during testing.
_Avoid_: echo number, test line

**Helper**:
A person who calls the user's Phone from another phone, or takes the user's Call, while the user works through a Real-Phone Checklist.
_Avoid_: caller, tester, assistant

**Real-Phone Checklist**:
A short, grouped list of steps only the user can perform with the real Phone, such as receiving a Call from another person.
_Avoid_: manual test, QA script
