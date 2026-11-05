# Coding standards

These rules bind every change to Desktop Calls. Most are enforced by `just check`, which CI also runs. Where a rule cannot be enforced by tooling, it says so. Source: spec issue #1, "Foundation work" item 1.

## Words: the glossary, verbatim

- Use the terms in [`GLOSSARY.md`](GLOSSARY.md) exactly, in code (types, functions, fields, modules), UI text, tests, log events, commit messages and PR text: `Call`, `Phone`, `Desktop`, `Handoff`, `Audio Location`, `Missed Call`, `Incoming Call Banner`, `Call Window`, `Dialer`, `Indicator`, `Setup Window`, `Simulated Phone`.
- Never use a word from a term's _Avoid_ list: no `session` or `line` for a Call, no `device` or `handset` for the Phone, no `transfer` for a Handoff, no `popup` or `toast` for the Incoming Call Banner, no `mock phone` for the Simulated Phone.
- Names from external APIs (`AudioGateway`, `org.ofono.VoiceCall`, `PhonebookAccess1`, `Device1`) stay as they are, but only inside the adapter that talks to that API. They never leak into the core or the control interface.
- If you need a concept the glossary lacks, stop and report it. Don't invent a term.

## Safety

- No `unwrap()` or `expect()` outside tests. Clippy denies both (`clippy::unwrap_used`, `clippy::expect_used`); `clippy.toml` allows them in `#[test]` and `#[cfg(test)]` code. Handle the failure, or return a typed error.
- No `unsafe`, anywhere. The workspace sets `unsafe_code = "forbid"`, which no attribute can override. If a dependency is the only way, it must be a reviewed crate, not code here.
- Clippy pedantic is on as warnings, and `just check` turns every warning into an error. Never suppress a lint without a reason. Use `#[expect(clippy::lint_name, reason = "why this case is right")]`, not `#[allow]`, so the suppression fails once it stops being needed.

## Errors

- Every module interface returns a typed error: an `enum` of the few failure kinds a caller can act on. No `Box<dyn Error>`, `anyhow` or strings across a module boundary, and never a raw D-Bus, BlueZ or obexd error.
- An adapter maps raw failures to its kinds at its edge and logs the raw detail to the journal, with an `error_kind` field.
- User-facing text comes only from the Messages module in the core (failure kind in, one plain sentence and an optional next action out). No other code writes a sentence the user will read.

## Policy lives in the Call state machine

- Every decision ("does this Call ring?", "where does the audio go?", "is this a Missed Call?") is made in the core's Call state machine: one function from (state, input) to (new state, effects).
- Shell adapters (Phone link, BlueZ, obexd, GTK surfaces, Indicator, alerts, watchers) only turn the outside world into inputs and carry out effects. An `if` in an adapter that chooses behaviour is a policy leak; move it to the core.
- The core (`crates/core`) is pure: no I/O, no clock, no environment, no threads, no GTK, D-Bus or logging. Time arrives as a timer-tick input. `crates/core/clippy.toml` disallows the std I/O, clock and environment APIs, and `just check` fails if GTK, GLib, zbus, an async runtime, `tracing` or `libc` appears anywhere in its dependency tree.

## Tests

- Tests go through one seam: the real `desktop-calls` binary running against the Simulated Phone, driven and observed only through the control interface, synthetic keyboard and pointer input, the desktop stand-ins, and screenshots. There are no tests of internals. The pure core modules (Call state machine, matching, Messages) may also be tested at their public interface.
- A test states a user-visible scenario and asserts only external behaviour: control-interface state and signals, what the Simulated Phone recorded it was asked to do, what the stand-ins received, and what is on screen.
- Wait on observable state with a deadline, for example "until `ConnectionState` is `connected`, at most 5 s". Never a fixed sleep.
- Each test gets its own private buses and temporary home, so tests run in parallel, and keeps its logs when it fails (`KEEP_SCENARIO_LOGS=1` keeps them when it passes too). Nothing it starts outlives it, even when its process is killed.

## No test-only code paths

- Production code never asks whether it is under test: no `#[cfg(test)]` branches in production logic, no environment variable or flag that only tests set, no hidden "simulation mode".
- Bus addresses come only from `DBUS_SESSION_BUS_ADDRESS` and `DBUS_SYSTEM_BUS_ADDRESS`. Paths come only from the XDG variables (`XDG_STATE_HOME`, `XDG_CACHE_HOME`, `XDG_CONFIG_HOME`, `XDG_RUNTIME_DIR`) and `HOME`. Tests point these at private buses and a temporary home; the app cannot tell the difference.
- The Simulated Phone is a separate binary that implements the same D-Bus interfaces as the real stack. Never add a code path to the app for it.

## Logging and privacy

- Log through `tracing` only (no `println!` in the service). `crates/desktop-calls/src/logging.rs` installs the subscriber.
- Every event carries an `event_kind` field from the `EventKind` enum: `event_kind = %EventKind::ServiceStarted`. Add a variant for a new kind; never pass a free-form string.
- Use these field names, snake_case, with the same meaning everywhere:

  | Field | Value |
  | --- | --- |
  | `event_kind` | an `EventKind` variant |
  | `call_id` | the app-assigned stable Call id |
  | `phone_address` | the Phone's Bluetooth address |
  | `error_kind` | the typed error's kind |
  | `phone_number` | a phone number (sensitive) |
  | `contact_name` | a person's name from Contacts (sensitive) |

- Phone numbers and names go **only** in the `phone_number` and `contact_name` fields, never in the message text or any other field. The logging layer writes those fields as `[redacted]` on info, warn and error events, and in full only on debug and trace events. To add another sensitive field, add its name to `SENSITIVE_FIELDS` in `logging.rs`.
- Levels: `error` for a failure the user notices, `warn` for a recovered failure, `info` for state changes worth reading after the fact (connected, Call started, Handoff), `debug` for detail an agent needs when diagnosing.
- Destination, chosen once at startup with no test-only branch: the systemd journal when its socket is reachable; otherwise `$XDG_STATE_HOME/desktop-calls/desktop-calls.log` (directory 0700, file 0600, one JSON object per line); otherwise standard error. `RUST_LOG` sets the level filter (default `info`).
- Read the journal with `journalctl --user -t desktop-calls -o json`. Fields appear upper-cased (`EVENT_KIND`, `CALL_ID`).

## D-Bus naming

- The app's control interface: bus name `io.github.danielbaldwin47.DesktopCalls`, object `/io/github/danielbaldwin47/DesktopCalls`, interface `io.github.danielbaldwin47.DesktopCalls1`.
- The Simulated Phone's control interface: `io.github.danielbaldwin47.DesktopCalls.SimulatedPhone1`, on the private test bus.
- Interface names end in a version number; an incompatible change adds a new interface (`...2`) rather than changing the old one. Methods, properties and signals are PascalCase (`OpenDialer`, `ConnectionState`, `MissedCall`). Enumerated string values are lower-case (`"desktop"`, `"phone"`, `"connected"`).

## Tooling

- `just check` must pass before you push: rustfmt, clippy with `-D warnings`, the build, `cargo deny`, the core-purity check and the file-length check. `just --list` shows every recipe.
- No `.rs` file is over 1,000 lines, so an agent can read a whole file without spending its context on it. `just check` fails and names any file over. Split it into modules along its seams, keeping its public paths with `pub use`.
- The toolchain is pinned in `rust-toolchain.toml`. Change it in its own commit.
- Every dependency must be compatible with GPL-3.0-or-later, free of known advisories, and from crates.io; `deny.toml` holds the licence list. Add a licence to it only after checking its compatibility, with a comment.
- Lockfile changes are committed; `--locked` is used everywhere.
