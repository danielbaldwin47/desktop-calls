# Call audio travels over Bluetooth hands-free, not a Wi-Fi relay

On 2026-11-04 we decided the Desktop acts as a Bluetooth Hands-Free unit for the Phone (the role a car kit plays), carrying Call audio and call control over the Phone's built-in Bluetooth. A Mac relays iPhone calls over Wi-Fi, but Android does not let non-system apps capture or inject cellular call audio, so a Wi-Fi relay would need root or a system app. Windows Phone Link reached the same conclusion.

## Consequences

- v1 needs nothing installed on the Phone beyond Bluetooth pairing.
- Calls only work while the Phone is within Bluetooth range of the Desktop.
- A Companion App, planned after v1, adds features Bluetooth cannot provide (contact photos, richer state). It does not carry Call audio.
