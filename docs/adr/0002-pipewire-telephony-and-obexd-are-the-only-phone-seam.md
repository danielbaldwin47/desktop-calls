---
status: amended by ADR-0004
---

# PipeWire Telephony and obexd are the only way the app reaches the Phone

On 2026-11-04 we decided Desktop Calls never speaks Bluetooth directly. Call control and call state go through PipeWire's `org.pipewire.Telephony` D-Bus API (PipeWire ≥ 1.4, native `hfp_hf` role), Call audio is routed by WirePlumber, and Contacts and Call History come from BlueZ obexd's `org.bluez.obex.PhonebookAccess1`. Both are D-Bus interfaces, so the Simulated Phone is a fake implementation of those same interfaces on a private bus, and almost all behaviour can be tested without hardware.

## Considered options

- **oFono hands-free plus PipeWire's ofono backend**: rejected. In 2026 oFono is AUR-only and its hands-free code is barely maintained.
- **Our own hands-free AT-command handler over BlueZ `Profile1`**: rejected. BlueZ refuses a second registration of the Handsfree UUID that PipeWire already owns, and we would be reimplementing a mature stack.

## Consequences

- PipeWire 1.6.9's API has `Activate()` to bring Call audio to the Desktop but no `Deactivate()`. Handoff back to the Phone needs a workaround or an upstream patch; see [research](../research/2026-11-04-hfp-stack.md) §5.
- If BlueZ's experimental built-in hands-free (5.86+) is ever enabled, it competes for the same UUID. The app must detect this at startup and say so.
- The API is young (2025) and has seen little field testing, so expect handset quirks. Real-Phone Checklists exist to catch them.
