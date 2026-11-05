# Answered or declined on the Phone, and the keybinding

Date: 2026-11-04. Ticket #8. Simulated Phone only; the real Phone was not touched in this batch.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** the Call state machine already kept the audio on the Phone when a Call is answered there, and already ended a Call at once when the Phone removes it. The work was in the Simulated Phone (it can now answer, decline and lose the caller on the Phone side, and models Android opening SCO only when `RejectSCO` is false), in the scenarios that prove it, and in the Desktop Calls keybinding with keyboard focus for the Banner and the Call Window.

## 1. What PipeWire does when the Phone acts

From PipeWire 1.6.9's `spa/plugins/bluez5/backend-native.c` (HFP Hands-Free side). **[V]** (source read)

- **Answered on the Phone:** `+CIEV: call,1` or a CLCC update moves the Call from `incoming` to `active` with a `PropertiesChanged` on `org.pipewire.Telephony.Call1`. Nothing else is sent to Desktop clients.
- **Declined on the Phone, or the caller hanging up while ringing:** `+CIEV: callsetup,0` (or the Call missing from the next CLCC list) sets the Call's state to `disconnected`, emits `PropertiesChanged`, then destroys the Call (`InterfacesRemoved`). There is no lingering object, so the app needs no grace period before it closes the Banner.
- **SCO:** when the Phone opens the SCO link itself (Android does on answering on the Phone), PipeWire accepts it unless the AudioGateway's `RejectSCO` is true. `RejectSCO=true` is what keeps the audio on the Phone.

## 2. What the Simulated Phone now models

- Control methods `AnswerOnPhone(call)`, `DeclineOnPhone(call)` and `CallerHangsUp(call)`, with the signal order above. **[V]** (scenarios in `crates/test-harness/tests/incoming_call.rs`)
- Answering on the Phone opens the audio link (codec, then transport `pending`, then `active`) only while `RejectSCO` is false, as PipeWire would accept it. Each scenario that answers on the Phone asserts the transport stayed idle, no `Activate` was recorded and no `RejectSCO=false` was set. **[V]**
- A Call's end, from either side, now always sets the transport idle with codec 0 after the last Call. **[V]**
- Mutation checks: removing the end-of-Call `KeepAudioOnPhone` turned "after a Call on the Desktop, the next Call answered on the Phone stays on the Phone" red; removing the on-connect `KeepAudioOnPhone` turned "starting while the Phone is connected rejects Desktop audio" red. **[V]**

## 3. Keyboard focus

- `desktop-calls` with no subcommand is the keybinding's command. It sends `Activate` over the control interface. The Call state machine focuses the Banner while a Call rings, else the Call Window while a Call is not ended, else reports `dialer-unavailable` until the Dialer lands in #11. **[V]** (`crates/core/tests/keybinding.rs`, `crates/test-harness/tests/keybinding.rs`)
- A focused surface switches its layer-shell keyboard mode to `Exclusive`. On the Banner, A answers on the Desktop and D declines; the Answer button holds GTK focus so Enter or Space can only answer. Its heading reads "Press A to answer or D to decline". **[V]** (sway, screenshot `banner-focused.png`)
- Escape sets the mode to `None` until the compositor confirms the window lost focus, then restores the surface's usual mode (`None` for the Banner, `OnDemand` for the Call Window), and sends `ReleaseFocus`. It never ends or declines a Call. Keys typed afterwards reach the window that had focus before. **[V]** (sway, test client's `keyboard-enter` and `key 30 released`)
- Each surface reports `focused` in `Surfaces` while GTK says its window is active, so tests see what the compositor decided. **[V]**

## 4. Timing under CI-like load

Reproduced as in `2026-11-04-ci-banner-latency.md` (one CPU, four scenarios, no GPU).

- The Banner's first frame held the main thread for about 0.6 s after it mapped. A decline on the Phone arriving in that window closed the Banner 610 ms after the removal; once the Banner had drawn, the app reacted in 2.7 ms. The screen scenarios therefore act on the Phone only after a screenshot shows the Banner drawn, as a person would. **[V]**
- Pre-rendering the Banner once at startup would remove that 0.6 s; not done here. **[I]**
- Each 3840x2160 capture and its scan take over a second under that load, so screenshot waits in the new scenarios allow 10 s. **[V]**
- In this reproduction, 5 of 11 `incoming_call_surfaces` scenarios fail on `origin/main` too (screenshot waits of 5 s), while CI on `main` is green; the reproduction is now harsher than CI. Not changed here. **[V]**

## 5. Not verified on the real Phone

- The Pixel's exact signals when a Call is answered or declined on the Phone, and that PipeWire then destroys the Call promptly. **[I]**
- That Android honours `RejectSCO=true` and the audio stays on the Phone after answering there. **[I]**
- Focus behaviour on Hyprland (only sway was used). **[I]**
