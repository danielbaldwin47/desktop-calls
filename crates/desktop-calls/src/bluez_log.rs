//! Each method call the app makes to BlueZ, BlueZ's reply and its error,
//! as a debug line, for the agent reading a Real-Phone Checklist step:
//! without root the system bus refuses an outside monitor, so nothing else
//! can see them (#422).
//!
//! Off unless `RUST_LOG` turns on [`TARGET`] at debug, as
//! `RUST_LOG=info,bluez_dbus=debug` does (`just run` sets it). Off, nothing
//! is formatted or allocated for them: `tracing` evaluates an event's
//! fields only once the event is enabled.
//!
//! A line names BlueZ's object by its kind, never by its path, which holds
//! the Phone's address. An error's text has its addresses and numbers
//! hidden, as the checklist's captures do; that it names no person is
//! assumed (RPU-38 in `docs/real-phone-unknowns.md`).

use std::collections::HashMap;
use std::fmt;

use desktop_calls_core::redaction::{hide_addresses, hide_numbers};

use crate::logging::EventKind;

/// The lines' `tracing` target: the name `RUST_LOG` turns them on by. Kept
/// outside `desktop_calls::`, so `RUST_LOG=desktop_calls=debug` leaves them
/// off and this switch turns on nothing else.
pub const TARGET: &str = "bluez_dbus";

/// `call`, a method call to BlueZ's object at `path`, with a debug line as
/// it is made and another for its reply or error. `property` names the
/// property a `Properties` method reads or writes.
pub async fn logged<T, E: fmt::Display>(
    path: &str,
    interface: &str,
    method: &str,
    property: Option<&str>,
    call: impl Future<Output = Result<T, E>>,
) -> Result<T, E> {
    tracing::debug!(
        target: TARGET,
        event_kind = %EventKind::BluezMethodCalled,
        object = object_kind(path),
        interface,
        method,
        property,
        "Called BlueZ"
    );
    let answer = call.await;
    match &answer {
        Ok(_) => tracing::debug!(
            target: TARGET,
            event_kind = %EventKind::BluezMethodReplied,
            object = object_kind(path),
            interface,
            method,
            property,
            "BlueZ replied"
        ),
        Err(error) => tracing::debug!(
            target: TARGET,
            event_kind = %EventKind::BluezMethodFailed,
            object = object_kind(path),
            interface,
            method,
            property,
            detail = %Hidden(error),
            "BlueZ answered with an error"
        ),
    }
    answer
}

/// What BlueZ's object at `path` is, without the address a device's path
/// holds.
fn object_kind(path: &str) -> &'static str {
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    match parts.as_slice() {
        [] => "root",
        ["org", "bluez"] => "bluez",
        ["org", "bluez", adapter] if adapter.starts_with("hci") => "adapter",
        ["org", "bluez", _, device] if device.starts_with("dev_") => "device",
        ["org", "bluez", _, device, ..] if device.starts_with("dev_") => "device-part",
        _ => "other",
    }
}

/// An error's text, `org.bluez.Error.Failed: br-connection-page-timeout`,
/// with its addresses and numbers hidden.
struct Hidden<'a, E>(&'a E);

impl<E: fmt::Display> fmt::Display for Hidden<'_, E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = hide_addresses(&self.0.to_string(), &mut HashMap::new());
        formatter.write_str(&hide_numbers(&text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_errors_addresses_and_numbers_are_hidden() {
        let error = "org.freedesktop.DBus.Error.UnknownObject: no /org/bluez/hci0/dev_FC_41_16_00_00_01 for AA:BB:CC:DD:EE:FF, dialling +1 (555) 123-4567";
        let shown = Hidden(&error).to_string();
        assert!(
            shown.starts_with("org.freedesktop.DBus.Error.UnknownObject: no /org/bluez/hci0/dev_dev-"),
            "{shown}"
        );
        assert!(shown.ends_with("dialling <number>"), "{shown}");
        for private in ["FC_41", "AA:BB", "555"] {
            assert!(!shown.contains(private), "{private} in {shown}");
        }
    }

    #[test]
    fn an_object_is_named_by_its_kind() {
        for (path, kind) in [
            ("/", "root"),
            ("/org/bluez", "bluez"),
            ("/org/bluez/hci0", "adapter"),
            ("/org/bluez/hci0/dev_FC_41_16_00_00_01", "device"),
            ("/org/bluez/hci0/dev_FC_41_16_00_00_01/fd0", "device-part"),
            ("/io/github/danielbaldwin47/DesktopCalls/agent", "other"),
        ] {
            assert_eq!(object_kind(path), kind, "{path}");
        }
    }
}
