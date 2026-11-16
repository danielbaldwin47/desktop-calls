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
            shown.starts_with(
                "org.freedesktop.DBus.Error.UnknownObject: no /org/bluez/hci0/dev_dev-"
            ),
            "{shown}"
        );
        assert!(shown.ends_with("dialling <number>"), "{shown}");
        for private in ["FC_41", "AA:BB", "555"] {
            assert!(!shown.contains(private), "{private} in {shown}");
        }
    }

    /// Each event's fields as `name=value `, kept for the test to read.
    #[derive(Clone, Default)]
    struct Fields(std::sync::Arc<std::sync::Mutex<String>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Fields {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _context: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut written = self.0.lock().expect("unpoisoned");
            event.record(&mut Visitor(&mut written));
        }
    }

    struct Visitor<'a>(&'a mut String);

    impl tracing::field::Visit for Visitor<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            use fmt::Write as _;
            let _ = write!(self.0, "{}={value:?} ", field.name());
        }
    }

    #[test]
    fn a_logged_errors_detail_has_its_addresses_and_numbers_hidden() {
        use tracing_subscriber::layer::SubscriberExt as _;
        let fields = Fields::default();
        let subscriber = tracing_subscriber::registry().with(fields.clone());
        let path = "/org/bluez/hci0/dev_FC_41_16_00_00_01";
        let answer: Result<(), &str> = tracing::subscriber::with_default(subscriber, || {
            zbus::block_on(logged(path, "org.bluez.Device1", "Connect", None, async {
                Err(
                    "org.bluez.Error.Failed: no /org/bluez/hci0/dev_FC_41_16_00_00_01 for FC:41:16:00:00:01, +1 (555) 123-4567",
                )
            }))
        });
        assert!(answer.is_err());
        let written = fields.0.lock().expect("unpoisoned").clone();
        assert!(
            written.contains("detail=org.bluez.Error.Failed: no /org/bluez/hci0/dev_dev-"),
            "{written}"
        );
        assert!(written.contains("<number>"), "{written}");
        for private in ["FC_41", "FC:41", "555"] {
            assert!(!written.contains(private), "{private} in {written}");
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
