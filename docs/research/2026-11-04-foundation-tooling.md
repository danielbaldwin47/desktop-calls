# Foundation tooling findings (ticket #2, 2026-11-04)

Findings from building the cargo workspace, `just check`, CI and journal logging. Verified on the user's Desktop unless marked otherwise.

## Tools on the Desktop

- `just` and `cargo-deny` were not installed, and `sudo` is off limits, so they were installed per user with mise: `mise use -g just@1.59.0 cargo-deny@0.20.2` (recorded in `~/.config/mise/config.toml`, on `PATH` through mise shims). Arch's `extra` repo packages both, which CI uses.
- Rust 1.98.0 via rustup; `rust-toolchain.toml` pins `1.98.0` with rustfmt and clippy.

## Journal

- A user process writing the journal's native protocol to `/run/systemd/journal/socket`, outside any systemd unit, shows up in `journalctl --user` (verified: `journalctl --user -t desktop-calls -o json`). Entries get `_TRANSPORT=journal`, `_UID`, `_PID`, and our fields upper-cased (`EVENT_KIND`, `LOG_DESTINATION`).
- The spec's `journalctl --user -u desktop-calls` will only match once the systemd user unit exists (ticket #20). `-t desktop-calls` (the `SYSLOG_IDENTIFIER`) works from any launch.
- `tracing-journald` formats event fields itself, so a level-aware redaction step cannot sit in front of it. Desktop Calls writes the native protocol itself instead (about 30 lines, `std::os::unix::net::UnixDatagram`, no `unsafe`). It sends with `send_to` on an unbound socket, so a journald restart doesn't strand it. Large payloads would need the memfd path, which needs `unsafe`; the app's events are small, and a failed send falls back to standard error.
- Redaction verified: an info event with `phone_number = "+15555550123"` and `contact_name = "Ada Lovelace"` wrote `[redacted]` for both, and the same at debug (`RUST_LOG=debug`) wrote them in full. The probe was removed afterwards.

## Hiding the journal without a test-only path

- `unshare -rm` followed by `mount -t tmpfs none /run/systemd/journal` works unprivileged on this Desktop. The app then finds no journal and writes `$XDG_STATE_HOME/desktop-calls/desktop-calls.log` (verified: directory 0700, file 0600, JSON lines, same redaction). This gives the test harness (ticket #4) a per-test log file with no test-only code.
- Unverified: whether unprivileged user namespaces are allowed in the GitHub Actions container. A container usually has no journald at all, so the app falls back to the file there anyway.

## Lints and checks

- Clippy reads only the nearest `clippy.toml` (no merging), so `crates/core/clippy.toml` repeats the workspace's `allow-unwrap-in-tests` and `allow-expect-in-tests`.
- `cargo clippy -- -D warnings` also denies rustc warnings. Adding `RUSTFLAGS=-D warnings` to the build step would have invalidated the build cache every time someone alternated between `just check` and a plain `cargo build`, so it isn't used.
- Probes, each tried once and reverted: an `unwrap()` in non-test code, an `unsafe` block, a rustfmt violation, a bare `#[allow]`, and `Instant::now()` in the core each fail; an `unwrap()` inside a `#[cfg(test)]` module passes.
- cargo-deny warns about two `syn` versions (2.x through `tracing-attributes`, 3.x through `clap_derive`). It is a warning, not a failure.
