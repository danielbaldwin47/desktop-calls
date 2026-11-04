//! The Messages module: every sentence the user reads.

use desktop_calls_core::messages::{self, Failure};
use desktop_calls_core::{ConnectionState, Phone};

fn pixel() -> Phone {
    Phone {
        name: "My Pixel".to_owned(),
        address: "AA:BB:CC:DD:EE:FF".to_owned(),
    }
}

#[test]
fn status_says_the_connection_state_and_names_the_phone() {
    assert_eq!(
        messages::status(&ConnectionState::Connected, Some(&pixel())),
        "Connected to My Pixel (AA:BB:CC:DD:EE:FF)."
    );
    assert_eq!(
        messages::status(&ConnectionState::Connecting, Some(&pixel())),
        "Connecting to My Pixel (AA:BB:CC:DD:EE:FF)."
    );
    assert_eq!(
        messages::status(&ConnectionState::Disconnected, Some(&pixel())),
        "My Pixel (AA:BB:CC:DD:EE:FF) is out of range or has Bluetooth off."
    );
    assert_eq!(
        messages::status(&ConnectionState::NotSetUp, None),
        "No Phone is set up yet."
    );
}

#[test]
fn a_failure_is_one_sentence() {
    let message = messages::failure(Failure::ServiceNotRunning);

    assert_eq!(message.text, "Desktop Calls isn't running.");
    assert!(message.next_action.is_some());
}
