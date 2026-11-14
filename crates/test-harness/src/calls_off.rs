//! Driving Calls Off and Quiet Hours through the app's control interface,
//! as the CLI and the Indicator do.

use super::Harness;
use super::app::AppError;

impl Harness {
    /// Asks the app to turn calls off until `until`, a Unix time in seconds
    /// (0 for until they are turned on), as the CLI and the Indicator do.
    ///
    /// # Errors
    ///
    /// The app's refusal: its D-Bus error name and plain-language message.
    pub fn turn_calls_off(&self, until: u64) -> Result<(), AppError> {
        self.try_app_method("TurnCallsOff", &(until,))
    }

    /// Asks the app to set Quiet Hours from `start` to `end`, in minutes
    /// after midnight, as the CLI does.
    ///
    /// # Errors
    ///
    /// The app's refusal: its D-Bus error name and plain-language message.
    pub fn set_quiet_hours(&self, start: u16, end: u16) -> Result<(), AppError> {
        self.try_app_method("SetQuietHours", &(start, end))
    }

    /// Asks the app to turn Quiet Hours off.
    ///
    /// # Errors
    ///
    /// The app's refusal: its D-Bus error name and plain-language message.
    pub fn clear_quiet_hours(&self) -> Result<(), AppError> {
        self.try_app_method("ClearQuietHours", &())
    }
}
