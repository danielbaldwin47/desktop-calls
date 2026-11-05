# Contacts sync over PBAP: what obexd does, and what the Pixel did

For #9. Labels: **[V]** verified here (on this Desktop, the real Pixel, or in obexd's source as noted); **[I]** inferred.

## The real Pixel

- `org.bluez.obex` runs on the user's session bus, with the client object `/org/bluez/obex` and an ObjectManager at `/`. The client object has `Client1` and `AgentManager1`. Captured read-only into `docs/research/2026-11-04-dbus-snapshot/obex-client.json`, and checked by `obex_client_matches_the_pixel` in `tests/conformance.rs`. **[V]**
- With no session open, `GetManagedObjects` on `/` lists only `/org/bluez/obex`. **[V]**
- `CreateSession(address, {"Target": "pbap"})` failed twice on 2026-11-04 with `org.bluez.obex.Error.Failed: Timed out waiting for response`, about 10 s after each call. Nobody was holding the Phone, so the Phone's prompt to share Contacts was probably showing and unanswered. **[V]** for the error; **[I]** for the cause.

### The first pull, 2026-11-05

The user ran `tools/real-phone/pbap_probe.py` with the Phone in hand and accepted the prompt. The probe kept shapes only; no name or number left the Desktop. All **[V]**:

- The session opened in 0.45 s once the prompt had been accepted.
- **Contacts (`pb`).** 319 vCards, 688 KB, pulled in 5.35 s. Every one is vCard 3.0 with CRLF line ends. 73 have a `PHOTO`, and there are 8,401 folded lines, almost all inside photos. Fields present: `TEL`, `EMAIL`, `ADR`, `ORG`, `NOTE` and `BDAY`. `TEL` types include `CELL;PREF`, `HOME;PREF` and `VOICE;PREF`. One Contact has no number.
- **Call History.** `GetSize` gave 922 for `cch`, 303 for `ich`, 448 for `och` and 168 for `mch`, but each `PullAll` returned exactly 50 vCards. **The Pixel gives at most 50 Call History entries per pull.**
- Call History entries carry `FN` and `N` with `CHARSET=UTF-8` on 8 to 31 of each 50, and up to 5 of 50 have no name. `X-IRMC-CALL-DATETIME` carries `TYPE=DIALED`, `RECEIVED` or `MISSED`, with no time zone. `TEL` has `TYPE=0`.
- Number shapes: `+` and 11 digits, 10-digit national, 7 digits, and one of 17 digits.
- `Select` of a name that isn't a PBAP folder fails with `org.bluez.obex.Error.InvalidArguments`.
- The transfer, as `GetAll` returned it while queued, is in `docs/research/2026-11-04-dbus-snapshot/obex-transfer.json` (target file name redacted): `Status`, `Name` (`telecom/pb.vcf`), `Size` (`t`, 0), `Filename`, `Transferred` (`t`, 0) and `Session` (`o`), with `Suspend`, `Resume` and `Cancel`. `PullAll`'s reply carries the same properties except `Transferred`. Checked by `pbap_pull_matches_the_pixel`.

### What the app does about it

- **The 50-entry cap: read every page.** Each folder is read with `PullAll` and `Offset` 0, 50, 100, ... until `GetSize` is reached. A Phone that ignores `Offset` gives the first page again, which stops the reading, as does an empty page or a failed one (keeping what was read). Accepting the cap would leave the Call History Window with the last 50 Calls, a day or two for a busy Phone, so the cap is paged instead.
- **Recent Calls first.** A sync reports Contacts and the newest 50 Calls as soon as they are read, so the Banner can name callers and Missed Calls and the Dialer show recent Calls, then reads the remaining pages in the same session and reports the full Call History. On the Pixel that is 19 pages of `cch`.
- **After a Call, one page.** Refreshing the Call History pulls only the newest page and puts it in front of the held Calls older than anything on it. A full sync, on every connection, reads everything again.
- **The 5 s pull never holds up the Banner.** Pulls, parsing and indexing are off the main loop. `with_5000_contacts_the_banner_still_shows_within_a_second` rings while a 5 s pull of 5,001 Contacts is under way.
- **The parser already read every shape above:** `PREF` is ignored, `VOICE` and `TYPE=0` are labelled "other", folded photo lines are joined and skipped, `CHARSET=UTF-8` is decoded, and an entry with no name keeps its number. The Simulated Phone now serves the same shapes (`test-harness/src/vcards.rs`), caps Call History pulls at 50, honours `Offset` and `MaxCount`, and answers `GetSize`.
- Paging assumes `Offset` counts from the newest Call, as PBAP says and as AOSP's PBAP server implements it. **[I]**: the probe didn't pull a second page. Re-running the probe with `Offset` 50 on `cch` would confirm it.

## obexd 5.87, from its source

- obexd writes the pulled file with mode 0600, and reports the end of a transfer by a `PropertiesChanged` of `Status` before removing the object. So the app subscribes before calling `PullAll`. **[V]** source.
- `PullAll` and `GetSize` before `Select` give `Forbidden` "Call Select first of all". `Select` of a folder the Phone lacks gives `Failed` "Not Found"; an invalid name gives `InvalidArguments` "Invalid path" (seen on the Pixel too). `RemoveSession` of an unknown session gives `InvalidArguments`. **[V]** source.
- `PullAll` filters: `Format` (`vcard21` or `vcard30`), `Order`, `Offset` and `MaxCount` (`q`; `MaxCount` defaults to 65535), `Fields` and `FilterAll`. **[V]** source.
- How a refusal on the Phone reaches the app is **unverified**. obexd turns a failed OBEX Connect into `Failed` "OBEX Connect failed with 0x%02x", so a refusal may arrive as `0xc3` (Forbidden) or `0xc1` (Unauthorized), or as a plain `Failed`. The app treats `Error.Forbidden`, those codes, and messages containing "forbidden" or "unauthorized" as denied. The Simulated Phone's deny answers `Error.Forbidden`. **[I]**

## Android's prompt

- Android asks whether to share Contacts on the first PBAP connection, and the prompt lasts about 30 s. obexd gives up after 10 s, so the app retries `CreateSession` until 30 s have passed since the first try, reporting `permission-needed` with that deadline. **[I]** for the 30 s, from AOSP's PBAP server; not timed on the Pixel.
- Whether the Pixel's Contacts include an owner card (`0.vcf`), and what is in it, is unknown. The app skips vCards with neither a name nor a number. **[I]**

## Next

- With the Phone in hand: refuse the prompt once and record the real deny error, and pull `cch` with `Offset` 50 to confirm paging.
