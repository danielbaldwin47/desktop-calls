//! The Call state machine's record of the Desktop's Do Not Disturb, at its
//! public interface.

use desktop_calls_core::{Input, Phone, State, step};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";

fn pixel() -> Phone {
    Phone {
        name: "My Pixel".to_owned(),
        address: ADDRESS.to_owned(),
    }
}

/// Feeds `inputs` to a fresh state machine and returns where it ends up.
fn run(inputs: impl IntoIterator<Item = Input>) -> State {
    inputs
        .into_iter()
        .fold(State::default(), |state, input| step(state, input).0)
}

#[test]
fn do_not_disturb_is_off_until_the_desktop_says_otherwise() {
    assert!(!run([]).do_not_disturb());
    assert!(run([Input::DoNotDisturbChanged(true)]).do_not_disturb());
    assert!(
        !run([
            Input::DoNotDisturbChanged(true),
            Input::DoNotDisturbChanged(false)
        ])
        .do_not_disturb()
    );
}

#[test]
fn do_not_disturb_stays_on_when_the_phone_is_unpaired() {
    let state = run([
        Input::PhonePaired(pixel()),
        Input::DoNotDisturbChanged(true),
        Input::PhoneUnpaired {
            address: ADDRESS.to_owned(),
        },
    ]);

    assert_eq!(state.phone(), None);
    assert!(state.do_not_disturb());
}
