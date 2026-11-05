# Mute and keypad tones: what PipeWire and Android accept

Date: 2026-11-04. Ticket #15. Stack as in the [conformance note](2026-11-04-conformance.md): PipeWire 1.6.9 Telephony, BlueZ 5.87; the Phone is a Pixel 9a. No real Call was placed for this ticket: only the Simulated Phone was used.
Legend: **[V]** = verified on this machine against the Simulated Phone. **[S]** = read in source, not run against the Pixel: PipeWire 1.6.9 (`spa/plugins/bluez5/telephony.c`, `backend-native.c`, `defs.h`) or AOSP's Bluetooth stack (`bta/ag/bta_ag_cmd.cc`, `btif/src/btif_hf.cc`). **[I]** = inferred, not tested.

**Bottom line:** Mute is `MicrophoneVolume=0` on the AudioGateway, sent as a byte. Unmuting restores the last gain the Phone reported if it was 1 to 15, and 15 otherwise, so PipeWire's -1 (the byte 255) is never written back. Keypad tones go one per `SendTones`, because PipeWire puts the whole string in one `AT+VTS` and Android accepts exactly one tone per `AT+VTS`. That departs from the ticket's wording ("`SendTones` with exactly those strings"). Whether the Pixel honours `MicrophoneVolume=0` is unverified.

## 1. MicrophoneVolume

- `MicrophoneVolume` and `SpeakerVolume` are `y` (byte), read-write, on `org.pipewire.Telephony.AudioGateway1` (`telephony.c:121`). **[S]**
- PipeWire keeps them as `int` and sends them as a byte. Its "not known" value, `SPA_BT_VOLUME_INVALID` (-1), therefore reaches D-Bus as 255. **[S]**
- Before the Phone sends `+VGM`, the gain is `bluez5.hfp-hf.default-mic-volume` if set, else 15 (`SPA_BT_VOLUME_HS_MAX`, `defs.h:626`; `backend-native.c:53,2443,4136`). The range is 0 to 15 in HFP and in AOSP. **[S]**
- `ag_properties_set` (`telephony.c:862`) reads the variant with `dbus_message_iter_get_basic` and no type check. A value of any type but `y` writes the wrong number of bytes into that `int`. A malformed `Properties.Set` has already crashed WirePlumber once (see [quirks](../wiki/telephony-api-quirks.md)). The app sends `Variant::from(u8)`, and the Simulated Phone refuses any other type, so a regression fails the tests. **[S]**, **[V]** (Simulated Phone)
- A successful `Set` emits `PropertiesChanged(AudioGateway1, {MicrophoneVolume})`, then sends `AT+VGM=<n>` to the Phone. **[S]**

How the app uses it:

- Mute is offered only while the Audio Location is the Desktop. With the audio on the Phone, the Call Window shows Mute disabled with the reason, and `SetMuted(true)` returns the D-Bus error `io.github.danielbaldwin47.DesktopCalls1.Error.MuteUnavailable` with the plain-language sentence. **[V]**
- Muting remembers the gain to restore: the last one reported if 1 to 15, else 15. So 0 and 255 restore 15. **[V]** (`unmuting_after_a_microphone_volume_of_minus_one_restores_the_default_not_minus_one`)
- When the audio leaves the Desktop, or the Call ends, while muted, the app unmutes on its own, so the next Call doesn't start silent. **[V]** (core tests)
- If the Phone refuses the mute, the Call Window shows unmuted and an error says the other person can still hear. If it refuses a requested unmute, it stays muted and says so. **[V]** (core tests)

## 2. Keypad tones

- `SendTones(s)` on the AudioGateway allows only `0-9`, `A-D`, `#` and `*`; anything else is `org.pipewire.Telephony.Error.InvalidFormat`, "Invalid phone number or tones" (`telephony.c:929,1017`). **[S]**
- `hfp_hf_send_tones` needs an `active` Call, else `InvalidState`. It sends `AT+VTS=<the whole string>` (`backend-native.c:1858-1879`). **[S]**
- Android's AG accepts `+VTS` only when the argument is exactly one character; otherwise it answers `+CME ERROR: 25`, invalid characters in text string (`bta_ag_cmd.cc:1110-1117`). `btif_hf.cc:573-574` passes on only `str[0]`. **[S]**
- PipeWire sends `AT+CMEE=1` (`backend-native.c:2463`), so that error comes back as `org.pipewire.Telephony.Error.CME` with the text above. **[S]**
- So the app sends one `SendTones` per tone, each after the previous reply, and stops at the first error. The Simulated Phone answers a longer string with that `CME` error, as the Pixel should. **[V]** (Simulated Phone), **[I]** (Pixel)
- The app accepts digits, `*` and `#`; it drops spaces, `-`, `.`, `(` and `)` from pasted text, and refuses anything else with a plain error. `A-D` are not offered: phones don't have them. **[V]**

## 3. Keyboard

- The Call Window's shortcuts are shown on its buttons, from the same GTK accelerator strings the shortcuts use: `M` Mute, `K` Keypad, `Ctrl+H` Hang up. Hang up needs Control so a stray key can't end a Call. They are caught before any control sees the key, so `M` and `K` never reach the tone entry. **[V]**
- Escape only gives keyboard focus back, as #8 made it; it never ends a Call. **[V]** (`escape_leaves_the_call_live`)
- Digits, `*` and `#` typed anywhere in the window, or pasted with Ctrl+V, are tones; other letters ring the error bell and send nothing. **[V]**
- The tone entry has the keyboard as soon as the Call connects. It must be enabled first: GTK won't focus a disabled widget, which first made Ctrl+V reach the window instead of the entry. **[V]**
- The screen scenarios paste with `wl-copy` from wl-clipboard, now in CI's packages. **[V]** (here)

## 4. CI-like timing

- `call_window_keyboard` passed 6 of 6 in each of three runs, the last after merging #8, under the loop in the [CI note](2026-11-04-ci-banner-latency.md): one CPU, four at a time, `/dev/dri` hidden, a 1 GB `/dev/shm`. The CPU was 15, not 0, because other work on this machine was pinned to CPU 0. **[V]**
- In the same runs, `incoming_call_surfaces` lost 2 or 3 of 11 to `the screen never showed banner within 5s`. `main` at `41cecbb` failed the same way with this branch's changes absent, so the cause is the machine's load at the time, not this branch. **[V]**
- The new scenarios give the screen 10 s to change, since one capture takes up to a second on a slow runner. **[I]** that this is enough in CI.

## 5. For the Real-Phone Checklist

None of these were tried on the Pixel:

1. The other person hears silence after Mute (`MicrophoneVolume=0`), and hears you again after unmuting. If the Pixel ignores `+VGM`, the ticket's follow-up mutes the Desktop's capture stream behind the same command.
2. Which `MicrophoneVolume` the Pixel reports at the start of a Call: a gain from 0 to 15, or 255 because it never sends `+VGM`.
3. Keypad tones reach the other party, one `AT+VTS` per tone, for example to an automated menu.
4. A multi-tone `SendTones` gets `org.pipewire.Telephony.Error.CME`, as AOSP's source says.
