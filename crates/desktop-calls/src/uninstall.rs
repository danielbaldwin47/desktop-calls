//! `desktop-calls calls on --uninstalling`: what removing Desktop Calls runs
//! first, while the binary is still installed (ticket #215).
//!
//! `Device1.Blocked` stays in BlueZ's storage after the app is gone
//! (ADR-0008), so a user who removed Desktop Calls with calls off would find
//! the Phone refusing the Desktop with no app left to say why. This turns
//! calls back on, for the Phone the app's record says it blocked, and for
//! no other: a Phone blocked with no record is the user's own setting.
//!
//! If the service is running it is asked to turn calls on, so that it
//! doesn't block the Phone again before it is stopped. Otherwise BlueZ is
//! asked directly, and nothing connects the Phone: the app is going away.
//! (The running service does connect it once unblocked, as it does
//! whenever calls come on; it is stopped next.) The service is never
//! started for this: a stopped service stays stopped.

use std::process::ExitCode;
use std::time::Duration;

use desktop_calls_core::{CallsOffRecord, messages};
use glib::MainContext;
use zbus::Connection;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;

use crate::bluez::{self, BlockError};
use crate::calls_off;
use crate::cli::INTERFACE;
use crate::control;
use crate::glib_runtime::GlibRuntime;

/// How long the running service may take to unblock the Phone: BlueZ
/// answers a `Set` at once, so this is for a Desktop that is struggling.
const SERVICE_DEADLINE: Duration = Duration::from_secs(10);
/// How often to look whether the service has finished.
const LOOK_EVERY: Duration = Duration::from_millis(50);

/// Turns calls back on for removal. Silent when there is nothing to put
/// right; fails only when BlueZ couldn't unblock the Phone, and says what
/// to run later.
pub fn calls_on() -> ExitCode {
    let context = MainContext::default();
    context.block_on(turn_calls_on(&GlibRuntime::new(context.clone())))
}

async fn turn_calls_on(runtime: &GlibRuntime) -> ExitCode {
    let Some(record) = calls_off::load() else {
        return ExitCode::SUCCESS;
    };
    // An "on" record is a turn-on stopped halfway: the Phone is blocked
    // still, and by the app.
    let (CallsOffRecord::Off { address } | CallsOffRecord::TurningOn { address }) = record;
    if service_turned_calls_on(runtime).await {
        println!("{}", messages::phone_unblocked_for_removal());
        return ExitCode::SUCCESS;
    }
    let unblocked = unblock(runtime, &address).await;
    // The app is going away: a record left behind would block the Phone
    // again if Desktop Calls were installed later, even after the user ran
    // the command below.
    calls_off::save(None);
    match unblocked {
        Unblocked::Done => {
            println!("{}", messages::phone_unblocked_for_removal());
            ExitCode::SUCCESS
        }
        Unblocked::PhoneGone => ExitCode::SUCCESS,
        Unblocked::Failed => {
            eprintln!("{}", messages::phone_left_blocked_by_removal(&address));
            ExitCode::FAILURE
        }
    }
}

/// Asks the service, if it is running, to turn calls on, and waits until it
/// has unblocked the Phone and removed the record. `false` if there is no
/// service, or it didn't finish.
async fn service_turned_calls_on(runtime: &GlibRuntime) -> bool {
    let Ok(session) = zbus::connection::Builder::session()
        .runtime(runtime.clone())
        .build()
        .await
    else {
        return false;
    };
    // Asking the bus, rather than calling the service, because a call
    // would start a stopped service through D-Bus activation.
    if !service_is_running(&session).await {
        return false;
    }
    let asked = session
        .call_method(
            Some(control::BUS_NAME),
            control::PATH,
            Some(INTERFACE),
            "TurnCallsOn",
            &(),
        )
        .await;
    if asked.is_err() {
        return false;
    }
    let looks = SERVICE_DEADLINE.as_millis() / LOOK_EVERY.as_millis();
    for _ in 0..looks {
        if !calls_off::is_kept() {
            return true;
        }
        glib::timeout_future(LOOK_EVERY).await;
    }
    false
}

async fn service_is_running(session: &Connection) -> bool {
    let Ok(name) = BusName::try_from(control::BUS_NAME) else {
        return false;
    };
    let Ok(bus) = DBusProxy::new(session).await else {
        return false;
    };
    bus.name_has_owner(name).await.unwrap_or(false)
}

/// How unblocking the Phone in BlueZ went.
enum Unblocked {
    Done,
    /// BlueZ has no such Phone any more: nothing is left blocked.
    PhoneGone,
    /// BlueZ can't be reached, or refused.
    Failed,
}

async fn unblock(runtime: &GlibRuntime, address: &str) -> Unblocked {
    let Ok(system) = zbus::connection::Builder::system()
        .runtime(runtime.clone())
        .build()
        .await
    else {
        return Unblocked::Failed;
    };
    match bluez::set_blocked(&system, address, false).await {
        Ok(()) => Unblocked::Done,
        Err(BlockError::PhoneNotFound) => Unblocked::PhoneGone,
        Err(BlockError::Refused) => Unblocked::Failed,
    }
}
