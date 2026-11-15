//! Every Calls Off line of the Indicator's menu fits Omarchy's tray menu
//! whole (#313): the tray elides a longer label, and the cut fell on the
//! one fact each line carries, the time or what happens after the Call.
//!
//! Omarchy 4.5's tray menu is 232 logical pixels wide; a label starts 28
//! in and keeps 8 clear on its right, or 26 beside an entry's submenu
//! glyph, in the monospace font at 11 px, about 6.6 wide a character
//! (`plugins/bar/widgets/Tray.qml`, drawn by the gallery's stand-in in
//! `crates/test-harness/src/tray_menu.rs`). So 29 characters fit, or 26
//! beside the glyph.

use desktop_calls_core::{
    Deadline, IndicatorEntry, IndicatorStatus, QuietHours, State, TimeOfDay, Timestamp, messages,
};

/// The characters a label of Omarchy's tray menu shows whole.
const FITS: usize = 29;
/// The same, beside an entry's submenu glyph.
const FITS_BESIDE_SUBMENU: usize = 26;

/// The widest time the menu shows.
const LATE: TimeOfDay = TimeOfDay {
    hour: 23,
    minute: 59,
};

fn at(shown: TimeOfDay) -> Deadline {
    Deadline {
        at: Timestamp(0),
        shown,
    }
}

fn assert_fits(label: &str, room: usize) {
    let length = label.chars().count();
    assert!(
        length <= room,
        "{label:?} is {length} characters; Omarchy's tray shows {room} whole"
    );
}

#[test]
fn every_calls_off_status_fits_the_tray_menu_whole() {
    for status in [
        IndicatorStatus::CallsOff {
            until: Some(LATE),
            quiet_hours: true,
        },
        IndicatorStatus::CallsOff {
            until: Some(LATE),
            quiet_hours: false,
        },
        IndicatorStatus::CallsOff {
            until: None,
            quiet_hours: false,
        },
        IndicatorStatus::CallsOffPending,
        IndicatorStatus::CallsOffAgain,
    ] {
        assert_fits(&messages::indicator_status(&status), FITS);
    }
}

#[test]
fn every_calls_off_entry_and_choice_fits_the_tray_menu_whole() {
    let state = State::default();
    let view = state.indicator(&state.surfaces());
    let hours = QuietHours::new(LATE, TimeOfDay { hour: 7, minute: 0 });
    for entry in [
        IndicatorEntry::CallsOffUntilTurnedOn,
        IndicatorEntry::CallsOffForAnHour(at(LATE)),
        IndicatorEntry::CallsOffUntilMorning(at(LATE)),
        IndicatorEntry::TurnCallsOn,
        IndicatorEntry::KeepCallsOn,
        IndicatorEntry::QuietHours(hours),
        IndicatorEntry::QuietHours(None),
    ] {
        assert_fits(&messages::indicator_entry(&entry, &view), FITS);
    }
    assert_fits(
        &messages::indicator_entry(
            &IndicatorEntry::TurnCallsOff {
                enabled: true,
                for_an_hour: None,
                until_morning: None,
            },
            &view,
        ),
        FITS_BESIDE_SUBMENU,
    );
}
