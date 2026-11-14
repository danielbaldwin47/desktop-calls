# UI pass: a design review of every surface

Date: 2026-11-13. No ticket: a design proposal for the user to review. It changes no product code and opens no implementation tickets.
Sources: `just screenshot` on `40411a0` (the real binary against the Simulated Phone in headless sway: 96 captures of 24 states, plus the Indicator's 5 icons, light and dark); two Setup Window Problem states captured with a temporary, uncommitted gallery change; `crates/desktop-calls/src/surfaces.rs`, `theme.rs` and `crates/core/src/messages.rs`; issues #197 (Call Pill) and #198 (Calls Off and Quiet Hours); Omarchy's shell style (`/usr/share/omarchy/shell/Commons/Style.qml`); `docs/research/2026-11-13-real-call-log.md`. Nothing touched the Phone, Bluetooth or WirePlumber.
Companion page: the same review with the current screenshots and before-and-after mockups in light and dark is a private artifact, https://claude.ai/artifact/9tnmmuUZBu2PgEP7gh2sWE.
Legend: **[seen]** = judged from a gallery capture. **[from source]** = read from code, Messages or a spec, not seen rendered. **[inferred]** = computed or reasoned.

**Bottom line:** the app says the right things and every action has a key, but it looks like a stack of default buttons: nothing on a surface is more important than anything else, and none of it looks like it belongs beside Omarchy's own panels. The three changes that matter most are (1) the Call Window opening over the Call Pill's corner instead of across the screen from it, (2) a Call Window rebuilt around a hierarchy, and (3) one visual system drawn from the theme, including secondary text held to 4.5:1 (about 3.1:1 in the light theme now **[inferred]**).

## What was and wasn't captured

- **Seen:** Incoming Call Banner (ringing, identifying, Waiting Call with and without hold), Call Window (calling, ringing, live, muted with keypad, audio on the Phone, Held Call, Held Call with a third Call waiting, lost contact, ended, not placed), Dialer (Call History with Missed Calls, results, not connected), Setup Window (7 steps, 2 Problems), Indicator icons.
- **Not seen, judged from source:** the Indicator's menu and the Missed Call notification. Omarchy's shell draws both and the headless compositor has no shell.
- **Not built yet, designed from the specs:** the Call Pill (#197), Calls Off and the Quiet Hours Window (#198).
- **Not verified on the Desktop:** corner rounding and fonts. The gallery has no Hyprland, so its buttons keep stock round corners; on the Desktop the app follows Hyprland's rounding, 0 by default. Whether the Setup Window floats or tiles on Hyprland was not checked.
- Contrast figures come from colours sampled in the captures and libadwaita's 55% opacity for dimmed labels. **[inferred]**

## Already good: leave alone

- The words. "Connection to phone lost: the Call continues on your phone." Plain, specific, and they say what to do.
- The keyboard model: nothing steals focus, Escape never ends or declines a Call, Hang up needs Control, every shortcut is printed on its button, Enter in the Banner can only answer.
- The theme mechanics: live theme switching, Hyprland's rounding, green and red buttons whose label colour is chosen for 4.5:1. The proposals extend this code.
- Decline left, Answer right, with Hold & Answer as the one green choice for a Waiting Call.
- "Identifying caller…" for up to 2 s instead of a wrong "Withheld number".
- The Setup Window's consent (the exact keybinding line before anything is written) and its large, tabular pairing code.
- The Dialer's speed and fixed keyboard; Missed Calls red only on their detail line; "can't be called back" on a withheld number.
- The in-call and ringing Indicator silhouettes.

## Critique by surface

### Incoming Call Banner [seen]

- Keyboard answering is hidden: A and D work only after the keybinding gives the Banner the keyboard, and only then does the heading become the hint, replacing "Incoming call".
- No border and no icon; beside Omarchy's bordered notification cards it reads as a different product.
- It is narrower than the Call Window it stacks under, so the pair has a ragged left edge.
- Waiting Call: End & Answer ends the Call you are on but is the one neutral grey button, and nothing says who gets held or ended.
- Case: "Incoming call" but "Waiting Call".

### Call Window [seen]

- No hierarchy: five full-width grey buttons and a text entry at one size. The timer, looked at most, is the smallest text.
- The tone entry is always open, with a focus ring, though the keypad is closed.
- Muting inserts an orange "Microphone muted" bar under Hang up and shifts everything below it. The orange is hard-coded (`#e66100`); its white label is about 3.5:1. **[inferred]**
- Calling, Ended and Not placed show Mute, Keypad, Move audio and the tone entry disabled. Lost contact leaves a half-row gap, a half-width Mute and "Hang up on your phone" drawn as a disabled button, though it is an instruction.
- "Audio on this computer" is dimmed text beside a button twice its weight.
- With the audio on the Phone, a two-line explanation shows permanently and the action that fixes it (Move audio here) looks like every other button.
- A Held Call adds a second Hang up with a three-key shortcut next to Swap at the same weight.

### Call Pill [from source: #197 and the work in progress]

- At 280 px with two text buttons the caller has room for about six characters.
- A filled red Hang up sits in peripheral vision for the whole Call.
- The expanded Call Window stays top-right while the Call Pill rests bottom-right: every expand is a jump across the screen, and a click on the Call Pill opens controls a screen away from the pointer.
- Muted shows only as "Mute" turning to "Unmute" on a small button.
- No motion between the two forms.

### Dialer [seen]

- No footer: nothing says Enter calls, or that the Phone is connected.
- Rows are sentences: direction, label, number and date in one grey line.
- On the selected row, dimmed detail is about 2.7:1 (light) and 3.5:1 (dark); the red "Missed" is about 3.6:1 and 3.1:1. **[inferred]**
- The notice ("Phone not connected…") is plain text between the entry and the list.
- A fixed 420 px list leaves most of the surface empty for three matches.
- No border, so no edge against a dark window behind it.

### Setup Window [seen]

- No progress: seven screens with no sign of how many there are.
- A Problem is said twice (the Problem block and a red sentence under the steps), and the steps it blocks stay at full strength.
- "Looking for phones nearby…" sits in a field that looks like an input.
- The countdown is a bare number.
- The key hint is the smallest, dimmest line.
- The keybinding line wraps mid-word and shows a full home path where `~` would do.
- The tip to pin the tray icon is dimmed, though without pinning the Indicator sits in the tray drawer.

### Indicator [icons seen; menu from source]

- Out of range (phone with a cross), Problem (phone with an exclamation mark) and, with #198, Off (crossed-out phone) are all a phone plus a small mark at 16 px.
- Menu wording: "Open Dialer" uses the project's word for the window; "Connect your phone…" is offered when the Phone is connected; "put it right…" where the glossary's word is Fix.
- With #198 the menu gains three more groups in a list that already changes shape during a Call.
- Unverified: whether Omarchy's tray draws submenus.

### Missed Call notification [from source]

- Good: the wording, the grouping of repeats, and that it obeys Do Not Disturb.
- It carries the app icon, not a missed-call glyph, and the summary mixes who with the number label.
- Once it leaves the screen, nothing in Desktop Calls shows there are Missed Calls; under Do Not Disturb it never showed.

## Journeys

1. **Incoming Call, ring to hang-up.** The Banner appears fast and never takes the keyboard; the Call Window replaces it in the same corner. Snags: keyboard answering is undiscoverable; seven equal controls arrive on Answer; the tone entry sits open all Call; muting shifts the layout; "Call ended" is a dead control panel with no duration.
2. **Placing a Call.** The Dialer is instant. Snags: no footer says Enter calls; "Calling…" surrounds the one working control with four disabled ones; a refusal reads as a row, not a notice.
3. **A long Call while working.** Snags: the collapse and every expand jump between corners; a red button is lit for the whole Call; muted is not readable at a glance.
4. **A Missed Call.** The notification's Call back is one click. Snag: nothing lasting says a Call was missed.
5. **First run.** The code screen is the best in the app. Snags: no progress, a Problem said twice, a bare countdown, the pin tip buried.
6. **Turning calls off for the night.** Each timed choice shows its clock time, and the one-Call exception is right. Snags: the Indicator may be in the drawer; a 16 px icon is the only confirmation; four spin buttons for two times.

## Proposed direction

### Principles

1. One line answers the question: one dominant line (who) and one state line per surface.
2. Show what can be used now; a control that can't work is absent unless its place must not move.
3. Nothing moves under the pointer: state changes recolour, and Hang up stays put.
4. Colour means one thing: green answers, red ends, the theme's orange or yellow warns, the accent marks selection and focus. No hard-coded colours.
5. Quiet at rest: during a long Call the app is a small neutral strip.
6. At home on Omarchy: bordered surfaces, Hyprland's corners, Omarchy's cursor row, monospace for numbers and keys.
7. Keys are always in view: one footer line of keycaps per surface, saying only what works now.

### Shared system

| Role | Face | Size / weight | Used for |
| --- | --- | --- | --- |
| Who | sans | 20 / 650 | Caller on Banner and Call Window; a bare number in monospace at 18 |
| Heading | sans | 18 / 650 | Setup Window and Quiet Hours Window titles |
| Body | sans | 13.5 / 400, 600 | Buttons, rows, instructions |
| Meta | monospace | 12 / 400 | Numbers, labels, times, counts; tabular figures |
| Timer | monospace | 18 / 500 | Call Window timer on the name's line; 13 on the Call Pill |
| Eyebrow | monospace | 10.5 / 600 caps | "Incoming call", "On hold", "Recent", step names |
| Keycap | monospace | 10.5 / 500 | Shortcuts in a 1 px outlined box |

- **Spacing:** 4, 8, 12, 16, 20. Surface padding 16 (20 for the two ordinary windows).
- **Sizes:** controls 36 high, tiles 58, small buttons 30, Call Pill 44 with 32 px icon buttons. Banner and Call Window both 340 wide; Call Pill 320; Dialer 560.
- **Corners:** surface = Hyprland's rounding; controls the smaller of that and 8; keycaps the smaller of that and 3.
- **Border:** 1 px, foreground at 30%, on every layer surface; 2 px accent while a surface has the keyboard; 2 px warning while muted or while contact is lost.
- **Colour roles from `colors.toml`:** surface `background`; raised `lighter_background`; text `foreground`; secondary text the weakest foreground-over-background mix that reaches 4.5:1, computed as `label_on` computes button labels; answer `green`; end, decline and missed `red`, with the same 4.5:1 floor on a selected row; muted, warnings and the Calls Off strip `orange`, else `yellow`; selection a 3 px `accent` bar over a 10% foreground fill.
- **Motion:** 120 ms hover and press; 150 ms ease-out for a surface appearing or leaving; 200 ms for Call Window and Call Pill changing into each other; "Call ended" stays 2 s; reduced motion makes everything a cut.

### Per surface

- **Incoming Call Banner:** 340 wide, bordered, eyebrow with a ringing dot, an initial or `#` tile, the keybinding always in a footer; once it has the keyboard the keycaps move onto the buttons and the border takes the accent. Waiting Call choices carry a consequence line ("Alice Smith waits on hold", "Ends the Call with Alice"), and End & Answer is outlined in red.
- **Call Window:** name and timer on one line, number below, one state line ("On this computer", "Muted. Alice can't hear you", "On your phone. Mute there, or move the audio here"), three tiles (Mute, Keypad, audio), Hang up full-width in a fixed place. Muting recolours the tile, the state line and the border. The tone entry exists only with the keypad, which grows downward. Calling, Ended and lost contact shrink to what they can do; "Call ended" gains the duration.
- **Call Pill:** 320 px; name, timer, then icon buttons for Mute and Hang up, neutral until hovered; the whole strip takes the warning tint when muted. The Call Window opens over the Call Pill's corner and grows upward.
- **Dialer:** a search row, section labels, a direction glyph or a Contact's initial, the time in a right-hand column, matched letters underlined, Omarchy's cursor row, a footer of keys with the Phone's state. Notices and the Calls Off line (#206) are a tinted strip; Enter's label becomes "Call anyway" during Calls Off.
- **Setup Window:** four named steps across the top; a Problem is one tinted card with its Fix and the steps it blocks are dimmed; the countdown is labelled; syncing is a checklist with a bar; the last screen offers the keybinding and the pin tip as two equal cards.
- **Quiet Hours Window:** two HH:MM fields (Tab between them, Up and Down step 15 minutes, digits type), a sentence saying what was saved and when calls next go off, and a 24-hour bar showing the span.
- **Indicator:** a dashed outline for out of range, a full diagonal for Calls Off, a filled badge for a Problem. One menu order in every state: status, the main action, calls on or off, Quiet Hours with its times, then setup and Problems.
- **Missed Call notification:** a missed-call glyph through Omarchy's glyph hint; the summary is who, the body is the number label and time.

## Ranked changes

Size: S is one ticket in one surface's view; M touches the view, Messages and scenarios; L is several tickets. "Before" means before the user's Real-Phone Checklist sitting, because the change alters what the sitting judges or what a step's script clicks.

| # | Change | User-visible gain | Size | When |
| --- | --- | --- | --- | --- |
| **1** | **The Call Window opens over the Call Pill's corner and grows in place** (needs question 1) | No jump on every expand; Keypad, Handoff and Swap appear under the pointer | S–M | Before, and before #200 merges |
| **2** | **Call Window rebuilt: name and timer line, state line, three tiles, fixed Hang up; mute moves nothing; compact Calling, Ended and lost contact** | The Call's state in one glance; no dead buttons; nothing shifts under the pointer | M–L | Before: checklist steps click these controls |
| **3** | **Shared visual system in the theme code: surface border, secondary text at 4.5:1, warning colour from the theme, monospace data, keycaps, footer line** | One app, at home on Omarchy; light-theme text becomes readable | M | Before, lands with 2 |
| 4 | Call Pill at 320 px with icon buttons, red only on hover, tinted when muted | A long Call leaves a quiet strip; muted is unmistakable | S | Before, inside #200 and #201 |
| 5 | Banner: 340 wide, bordered, keybinding footer, consequence lines on the Waiting Call's choices | Keyboard answering is discoverable; nobody ends the wrong Call | S | Before |
| 6 | Calls Off made visible: Dialer strip, distinct Off icon, "Calls off after" tag, a confirmation on a hand change, one menu order | You always know whether calls are off, why, and until when | S each | Before, inside #204, #205, #206 |
| 7 | Quiet Hours Window: two time fields, a saved sentence, a 24-hour bar | Two fields instead of eight targets; the midnight crossing is visible | S | Before, inside #209 |
| 8 | Dialer: selected-row contrast, notice as a strip, footer of keys and Phone state | Readable Missed Calls; Enter is discoverable | S | Before |
| 9 | Setup Window: one Problem card instead of two messages; the keybinding path wraps at separators and shows `~` | First run doesn't repeat itself | S | Before |
| 10 | Gallery: add the Setup Window's Problems, the Call Pill, the Quiet Hours Window and the Dialer during Calls Off | The screenshot review covers what it can't see today | S | Before |
| 11 | Dialer rows: direction glyphs, time column, section labels, matched letters; top edge pinned, height follows the rows | Faster scanning; no empty slab | M | After |
| 12 | Setup Window: step progress, grouped instructions, labelled countdown, sync checklist, pin tip as a card | First run feels shorter; the Indicator gets pinned | M | After |
| 13 | Motion: 150 ms in and out, 200 ms window to Call Pill, progress on the audio tile during a Handoff | The eye can follow; a 2.5 s Handoff doesn't look stuck | S–M | After |
| 14 | Copy pass: sentence case in the interface, "New call…", "Phone setup…", "fix…" (needs question 4) | One voice | M | After |
| 15 | A lasting Missed Call cue on the Indicator and in the Dialer (needs question 7) | Back at the desk, you can tell you missed a Call | M | After |

## Open questions for the user

1. **Where does the expanded Call Window open?** #197 keeps it top-right with the Call Pill bottom-right. Recommended: bottom-right, growing out of the Call Pill, with the Banner alone top-right. Cost: on Answer the eye moves corners once per Call instead of on every expand.
2. **Type: names in sans and data in monospace, or everything in monospace like Omarchy's menu?** Recommended: mixed, as Omarchy's own notifications do.
3. **Call Pill: is Hang up one click and red only on hover, and does the change between window and Call Pill animate?** Recommended: yes to both, lifting #197's "no animation" for this one 200 ms change.
4. **Interface copy in sentence case?** Recommended: lower case for call, phone and contacts in the interface, glossary capitals kept in docs; menu wording "New call…", "Phone setup…", "fix…".
5. **A confirmation when calls go off or on by hand?** Recommended: one transient notification; none when Quiet Hours start or end.
6. **Quiet Hours times as two fields or four spin buttons?** Recommended: two HH:MM fields with 15-minute arrow steps and typing, plus the saved sentence.
7. **A lasting Missed Call cue?** Recommended: a menu line and an icon badge until the Dialer is opened, after the sitting.
