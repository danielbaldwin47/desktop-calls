# Setup Window: pairing a Phone from nothing

Date: 2026-11-05. Ticket #21 (no comments).
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** On a fresh install the Setup Window opens by itself, lists only nearby Phones by name, and pairs the chosen one by numeric comparison with the Desktop as BlueZ's pairing agent. It then trusts and connects the Phone, and asks for hands-free alone if the Phone connected without it. Pairing started from the Phone works while the window is open, because the adapter is discoverable only then. Every step works by keyboard alone. All of this is verified against the Simulated Phone and simulated BlueZ only. Nothing was paired, trusted, discovered or made discoverable on the real adapter, and the real Phone was not touched.

## Decisions

- **An ordinary window, not a layer-shell surface.** Pairing is a task the user works through, partly on the Phone, so it tiles, takes focus when it opens and can be moved aside. The Banner, Call Window and Dialer stay on the layer shell. **[V]** (gallery: sway floats it, centred, because it isn't resizable)
- **Only Phones are listed.** A device is listed when it isn't paired, has a `Name`, and its Class of Device has major class 2, Phone (Bluetooth Assigned Numbers, 2.8.2). Headphones, major class 4, are never listed. A device with no `Name` is left out, because BlueZ's `Alias` is then only the address. **[V]** (scenario with headphones nearby) **[I]** that the Pixel advertises major class 2 during discovery.
- **The Desktop is the default agent only while the window is open,** registered with `DisplayYesNo`. With a Phone that also has a screen and yes/no input, Secure Simple Pairing uses numeric comparison, so BlueZ calls only `RequestConfirmation` (Core spec Vol 3 Part C 5.2.2.6). The agent has no PIN, passkey or authorization methods: BlueZ fails such a pairing on the unknown-method reply. `AuthorizeService` lets the Phone just paired connect a profile before `Trusted` is set. **[V]** for the code flow, against the simulation. **[I]** that the Pixel needs nothing else.
- **A pairing request while the window is closed is turned down.** The agent is unregistered then, so BlueZ's own default agent (if any) answers. While the window is open, a request for a Phone other than the one being paired is turned down. **[V]** (core tests)
- **Discoverable only while the window is open.** Opening it sets `Discoverable` and `Pairable`, sets a `bredr` discovery filter and starts discovery. Closing it stops discovery and clears `Discoverable`. `Discoverable` is set again every 60 s in case BlueZ's `DiscoverableTimeout` (default 180 s) ends it first. **[V]** (scenario reads `Discoverable` before and after) **[I]** the timeout's value on the Desktop.
- **After the code is confirmed:** once `Paired` is true the Phone is followed (switching from any other Phone), `Trusted` is set and `Device1.Connect` is called unless already connected. If the Phone connected but hands-free hasn't come up 2.5 s after the connect finished, the Desktop calls `ConnectProfile` for the Hands-Free Audio Gateway alone. Any refusal other than `AlreadyConnected` or `InProgress` means the Phone's "Phone calls" switch for the Desktop is off, and the window says where to switch it on. **[V]** (scenarios for each Phone behaviour)
- **This is a second use of `ConnectProfile(HFP)`.** ADR-0005 allows `ConnectProfile` for the hands-free UUID for the Handoff bounce alone, and #101 added its retry. The ticket requires it here too; a dated note on ADR-0005 records it.
- **Switching Phones keeps the old one paired in BlueZ.** The Desktop forgets the old Phone (its Contacts and Calls), and follows the new one. Unpairing the old one is left to the user, because removing a pairing is not undoable. Which Phone is followed after a restart, with two paired, is still the first one BlueZ lists. **[I]** that this matters to few users; a follow-up.
- **Pair again** (the pairing-lost Problem) opens the window. The stale pairing stays in BlueZ, so `Paired` stays true and no `PhonePaired` arrives. The pairing completes when the Phone connects after the confirmed code, or 30 s after the confirmation if it doesn't (then the Desktop connects it). **[V]** (scenario) **[I]** that BlueZ replaces a stale link key on a fresh pairing started from the Phone, without removing the device first.
- **Per-step timings** go to the journal as `setup-step-reached` with `step` and `since_opened_ms`, logged when the step's frame reached the compositor. `setup-phone-listed` marks the first Phone listed.

## Timings (Simulated Phone, debug build)

From one run of the choose-and-confirm scenario, with synthetic keys pressed as soon as each step showed, so the gaps are the app's and the simulation's, not a person's **[V]** (journal of that run):

| Step reached | ms after the window opened |
| --- | --- |
| choose, Phone listed | 361 (first frame; the Phone was already found) |
| pairing | 627 |
| confirm (code shown) | 849 |
| confirmed | 1055 |
| connecting | 1099 |
| done | 1709 |

The 0.36 s to the first frame is GTK's first paint. The rest of the 3-minute budget (scorecard item 4) is the person and the real radio: discovery, the page, and tapping Pair on the Phone. **[I]** that a real pairing takes well under a minute.

## Simulated Phone

- `--unpaired` starts the Phone not paired and nearby, `--headphones NAME` adds headphones (class 0x240418) and `--old-phone NAME` adds a second Phone, paired and never in range. Discovery adds each undiscovered device 300 ms apart.
- `Device1.Pair` waits 0.2 s, then asks the Desktop's agent to confirm a code. Without an agent it fails `AuthenticationFailed`; turned down, `AuthenticationRejected`.
- New control methods: `PairFromPhone` (needs the Desktop discoverable and an agent; returns the code), `PairingRequests` (each code and `waiting`, `confirmed` or `rejected`) and `SetHandsFree(shared | when-asked | refused)`.
- `ConnectProfile(HFP)` while connected without hands-free brings hands-free up after 0.3 s, or fails `br-connection-refused` when refused. That error name is **[I]**.
- A `Connect` to the old Phone no longer holds BlueZ's other answers for the page timeout.

## For the Real-Phone Checklist (user only; agents must not run these)

1. Pair the Pixel from the Desktop: open `desktop-calls connect`, put the Pixel on Settings > Connected devices > Pair new device, choose it. Check the Pixel is listed by its name, the code matches on both screens, and it reaches "connected". Time it from opening the window to the first Call to the Test Number (scorecard item 4: 3 minutes or less).
2. Forget the Desktop on the Pixel, then pair from the Pixel's side by tapping the Desktop's name while the window is open. Check the same code flow, and that the Desktop no longer shows on the Pixel's list once the window closes.
3. Check whether the Pixel brings hands-free up by itself after pairing, or only after `ConnectProfile` (journal: `ConnectHandsFree` in effects, `hands-free-refused` lines).
4. Switch Phone calls off for the Desktop on the Pixel (gear icon next to the Desktop), connect, and record the real `ConnectProfile` error from the journal's `hands-free-refused` `detail`. Check the window's instruction matches the Pixel's screens.
5. Pair again over a stale key: forget the Desktop on the Pixel only, then use the Indicator's "Pairing lost: pair again…" and pair from the Pixel. Check BlueZ accepts the new key without removing the device on the Desktop.
6. Check the adapter's `DiscoverableTimeout` (`busctl get-property org.bluez /org/bluez/hci0 org.bluez.Adapter1 DiscoverableTimeout`; read-only) and that the Desktop stays discoverable for as long as the window is open.
7. With headphones in pairing mode nearby, check they are not listed.
