# What the surfaces offer while contact with a live Call is lost

Date: 2026-11-05. Ticket #111, following [#101's hands-free not back](2026-11-05-handsfree-not-back.md) and #19's out of range.
Legend: **[V]** = verified against the Simulated Phone. **[I]** = inferred, not tested. Nothing in this note was tried on the real Pixel.

**Bottom line:** While the Call Window says "Connection to phone lost: the Call continues on your phone.", nothing on the Desktop offers to act on the Call any more. **[V]**
- Hang up is a plain grey button that says "Hang up on your phone". Mute, the keypad and Handoff are greyed out, none of them has a shortcut, and the "Mute works only…" hint is gone.
- The Indicator's status reads "Connection to phone lost: the Call with <who> continues on your phone". Its menu offers only a greyed-out "Hang up on your phone".
- Once the Call Window closes, if only hands-free is lost and the Phone is still in range, the Indicator says "Connection to <Phone> lost", not "<Phone> out of range".

## Decisions

- **`CallWindowView.connection_lost`** is the core's word that the Call Window shows Calls the Desktop can't act on. The shell greys out every control from it, the same way it uses `waiting_first`. **[V]**
- **A Held Call stays listed.** `lose_calls` now keeps the Calls that the Call Window was showing: the live one, and the one on hold behind it. Before, it took the first live Call in the list, which could be the held one, and the window dropped the other. In the held row, Swap and its Hang up are hidden, and the window's notice explains why. **[V]**
- **A Waiting Call's Banner goes as soon as the Calls are out of reach,** and that includes hands-free being lost. Before, only an absent Phone hid it. So the Banner offered Hold & Answer for about 2 s, until the Phone link gave up on the Call. **[V]**
- **Ringing follows the same rule.** Before, the scenario's log showed `ringing-started` for a moment, when the Phone link dropped the live Call before the Waiting Call. That is **[V]** from the log. The scenario now asserts that the app isn't ringing in the state that first shows the lost message. That the momentary ring is gone is **[I]**: a ring shorter than one property read can slip between the harness's reads.
- **What is refused while the Call Window shows the lost message:**
  - Hang up of any Call, the held one included.
  - Answer and Decline, while the Calls are also out of reach.
  - Each is refused with the lost message and never kept for later. **[V]**
- **A greyed-out button's shortcut no longer presses it.** `gtk::Widget::activate` pressed a greyed-out button anyway. For example, Ctrl+H on the greyed-out Hang up sent `HangUp`, and the core refused it with an `Error` signal. That was also true of #16's greyed-out Hang up while a Waiting Call rings. The shortcut now checks `is_sensitive` first. **[V]**
- **No new glossary term.** The new labels and statuses reuse the Call Window's existing "Connection to phone lost" wording. "Hang up on your phone" is a label, not a concept.

## Not covered

- `desktop-calls status` and a Dialer refusal while hands-free is lost still say "out of range or has Bluetooth off". They read `ConnectionState`, which is `disconnected` in both cases. Fixing that needs a refusal kind of its own. Follow-up candidate. **[I]** from the code path.
- The greyed-out Mute, Keypad and Handoff buttons still show their shortcut letters, as greyed-out buttons do everywhere else in the app. Unchanged.
