# Incoming Call: answering, declining and hanging up, and the Banner on screen

Date: 2026-11-04. Ticket #7. Stack as in the [conformance note](2026-11-04-conformance.md): PipeWire 1.6.9 Telephony, BlueZ 5.87, Omarchy 4.5 on Hyprland; the screen harness is sway 1.12 headless at 3840x2160, scale 1.6.
Legend: **[V]** = verified on this machine. **[S]** = read in the PipeWire 1.6.9 source (`spa/plugins/bluez5/telephony.c`, `backend-native.c`), not run against the Pixel. **[I]** = inferred, not tested.

**Bottom line:** no real Call was placed or answered for this ticket, so Answer and Hangup on a ringing Call are modelled from the PipeWire 1.6.9 source, not from the Pixel. The Simulated Phone now behaves as that source does, and the Real-Phone Checklist (§6) lists what to confirm on the Pixel. On the Desktop, GTK's first window costs about 0.7 s, so both surfaces are realised at startup to meet the 1 s Banner target.

## 1. Answer, Hangup and Call removal in PipeWire 1.6.9

- `Call1.Answer` works only on an `incoming` Call. In any other state the reply is `org.pipewire.Telephony.Error.InvalidState`, "The current state does not allow this method call" (`backend-native.c:1524`, `telephony.c:251,285`). A `waiting` Call cannot be answered this way; call waiting (#16) needs `AT+CHLD`. **[S]**
- `Call1.Hangup` is accepted in `active`, `dialing`, `alerting` and `incoming` (`backend-native.c:1546-1550`). With another Call held it becomes `AT+CHLD=1`. **[S]**
- Both reply only after the Phone answers `OK` to the AT command (`rfcomm_send_cmd(…, m, "ATA")`). The state change follows as `PropertiesChanged(Call1)`, then `org.ofono.VoiceCall.PropertyChanged`. **[S]**
- `Call1` and `org.ofono.VoiceCall` share one handler (`telephony.c:1630-1634`), so a correct PipeWire never says "Method doesn't exist" for `Call1.Hangup`. That error text is libdbus's for an unhandled method: `Method "Hangup" with signature "" on interface "org.pipewire.Telephony.Call1" doesn't exist`. Foner's handset report (see [quirks](../wiki/telephony-api-quirks.md)) therefore points at an older PipeWire or another provider. The app still falls back to `org.ofono.VoiceCall.Hangup`. **[S]**
- "Too few arguments" appears nowhere in PipeWire 1.6.9. Its origin is unknown; the app treats it, and `InvalidState`, `UnknownObject` and `UnknownMethod` from a hang-up, as the Call already ending. **[S]**
- A Call is unregistered with `InterfacesRemoved`, then `CallRemoved` (`telephony.c:1739-1757`). Its id is the lowest free one (`find_free_object_id`, `telephony.c:320,1345`), so consecutive Calls reuse `call1`. The app gives each Call its own id and clears all per-Call state when it is removed. **[S]**
- Setting `RejectSCO` returns at once and emits no `PropertiesChanged` (`telephony.c:897-903`). So the app cannot observe its own `RejectSCO` writes, and tests read them from the Simulated Phone's record of method calls. **[S]**

## 2. Answer, then audio on the Desktop

- The app sends, in order and waiting for each reply: `Call1.Answer`, `Set(AudioGatewayTransport1, RejectSCO, false)`, `Activate()`. This order is now asserted (`answering_answers_then_clears_reject_sco_then_activates_in_order`). **[V]** (Simulated Phone)
- `Activate()` does nothing while `RejectSCO` is true (`backend-native.c:3015`, and the [Handoff prototype](2026-11-04-handoff-to-phone-prototype.md) on the Pixel). On the Pixel, `Activate()` once took 2.9 s and once did nothing. The 1.5 s audio target is met only under simulation, and a retry is still to do (#14). **[V]** on the Pixel, from #3
- `RejectSCO=true` is set when the Phone connects, whenever an AudioGateway appears with no Call that has Desktop audio, and when the last Call ends. **[V]** (Simulated Phone)
- `Codec` stays 0 while idle and is not reported as a change. The app adopts the codec only once the transport is up. 2 (mSBC) and 3 (LC3-SWB) count as wideband and show "HD". The Pixel negotiated LC3-SWB every time in #3. **[V]** (Simulated Phone)

## 3. The Banner and the Call Window on screen

- **Latency:** the first realise of a GTK window took about 0.7 s in the harness, because GDK tries Vulkan (`/dev/dri/renderD130: Permission denied`) before falling back to GL. Both windows are now realised hidden at startup. Since then, 11 of 11 surface scenarios and two full `just test` runs have met the 1 s Banner deadline. **[V]**
- **What counts as shown:** `Surfaces` reports a surface once GTK maps it, which can be a frame before the compositor draws it. Screen scenarios use `Harness::screenshot_when` and wait for the pixels. **[V]**
- **Layout:** the Call Window's Audio Location row keeps its space while it is empty, so Hang up does not jump down under the pointer when the Call connects. Clicks at the old position missed before this fix. **[V]**
- **Placement (Omarchy 4.5, see [recheck](2026-11-04-omarchy-4.5-recheck.md)):**
  - The top margin is only the gap (12 px). The default exclusive zone keeps the surfaces below the bar. **[V]** under sway with a 26 px bar stand-in; **[I]** on Hyprland.
  - The right margin is the width of the `omarchy-notifications` layer plus 12 px, or 400 + 12 px when Hyprland can't say. The width comes from Hyprland's request socket, `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`: write `j/layers`, read to EOF, and the reply is the same JSON as `hyprctl layers -j`. **[V]** (socket protocol, by hand, here: the layer is 394 wide at x 2006 on the overlay level)
  - The app asks off the main thread at startup and again each time the Banner appears. **[V]** against a harness stand-in reporting 300 px.
- **Idle:** the Banner is unmapped whenever nothing rings; realising does not map it. Nothing app-drawn is on screen while idle. **[V]** (screenshot). Whether Hyprland keeps direct scanout of a fullscreen window during idle is **[I]**.
- **Over fullscreen and focus:** the Banner shows over a fullscreen client and never takes keyboard focus. Typing while it rings reaches the focused client, and the Call keeps ringing. **[V]** under sway; **[I]** on Hyprland.

## 4. Ringing

- The ringer loops the sound theme's `phone-incoming-call` (`/usr/share/sounds/freedesktop/stereo/phone-incoming-call.oga` exists here) through `pw-play`, and stops on any state change. With no audio server, it logs `ringing-failed` (`playback-failed`, `player-unavailable` or `no-sound-file`) and the app carries on. **[V]** (harness, which has no audio server). Audible ringing on the real Desktop is **[I]**.

## 5. Policy and tooling changes

- `deny.toml` now scopes `[graph] targets` to x86_64 and aarch64 Linux. phonenumber reaches `atomic-polyfill`, which is flagged as unmaintained, only on targets without atomics. **[V]** (`cargo deny check`)
- New crates: gtk4 0.11, libadwaita 0.9, gtk4-layer-shell 0.8 and phonenumber 0.3 for the app, and png 0.18 for the harness. `cargo deny` warns, without failing, about duplicate miniz_oxide, regex-syntax, syn and system-deps versions. **[V]**

## 6. For the Real-Phone Checklist (group 2)

These still need the Pixel and a helper. Do them in the group 2 sitting (#28).

- Answer on the Desktop:
  - the Pixel's `Answer` reply time, and the order of signals afterwards;
  - whether `active` arrives before or after `Activate()`'s transport `pending`;
  - Banner latency and audio latency from the journal;
  - which codec was negotiated.
- Decline on the Desktop: confirm `Call1.Hangup` exists on the Pixel (no oFono fallback logged as `phone-command-fallback`) and that the Pixel shows no missed Call.
- Hang up on the Desktop: confirm `RejectSCO` returns to true and that a second Call reuses `call1` with a new app id.
- On Hyprland, confirm:
  - the Banner sits below the real bar and left of a toast shown at the same time;
  - it shows over a fullscreen video;
  - typing in the focused window is unaffected while it rings.
