//! The CLI: a thin client of the control interface.

use std::collections::HashMap;
use std::process::ExitCode;

use desktop_calls_core::messages::{self, Failure, NextAction};
use desktop_calls_core::{ConnectionState, Phone};
use glib::MainContext;
use zbus::fdo::PropertiesProxy;
use zbus::names::InterfaceName;
use zbus::{Connection, OwnedVariant};

use crate::control;
use crate::glib_runtime::GlibRuntime;

const INTERFACE: &str = "io.github.danielbaldwin47.DesktopCalls1";

/// The state `status` reports, read from the control interface.
struct Status {
    connection_state: ConnectionState,
    phone: Option<Phone>,
}

/// `desktop-calls status [--json]`: prints whether the Phone is connected.
pub fn status(json: bool) -> ExitCode {
    let context = MainContext::default();
    let status = context.block_on(read_status(GlibRuntime::new(context.clone())));
    match status {
        Ok(status) if json => {
            let phone = status.phone.map_or(
                serde_json::Value::Null,
                |phone| serde_json::json!({ "name": phone.name, "address": phone.address }),
            );
            let printed = serde_json::json!({
                "connection_state": status.connection_state.as_str(),
                "phone": phone,
            });
            println!("{printed}");
            ExitCode::SUCCESS
        }
        Ok(status) => {
            println!(
                "{}",
                messages::status(&status.connection_state, status.phone.as_ref())
            );
            ExitCode::SUCCESS
        }
        Err(failure) => report(failure),
    }
}

/// `desktop-calls` with no subcommand, which the keybinding runs: focuses
/// the Incoming Call Banner while a Call rings, or the Call Window while one
/// is live.
pub fn activate() -> ExitCode {
    let context = MainContext::default();
    match context.block_on(call_activate(GlibRuntime::new(context.clone()))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => report(failure),
    }
}

async fn call_activate(runtime: GlibRuntime) -> Result<(), Failure> {
    let session = connect(runtime).await?;
    session
        .call_method(
            Some(control::BUS_NAME),
            control::PATH,
            Some(INTERFACE),
            "Activate",
            &(),
        )
        .await
        .map(drop)
        .map_err(|error| failure_of(&error))
}

async fn connect(runtime: GlibRuntime) -> Result<Connection, Failure> {
    zbus::connection::Builder::session()
        .runtime(runtime)
        .build()
        .await
        .map_err(|_| Failure::ServiceNotRunning)
}

/// What a failed call to the long-running process means for the user.
fn failure_of(error: &zbus::Error) -> Failure {
    match error {
        zbus::Error::FDO(fdo)
            if matches!(
                **fdo,
                zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_)
            ) =>
        {
            Failure::ServiceNotRunning
        }
        zbus::Error::MethodError(name, _, _)
            if matches!(
                name.as_str(),
                "org.freedesktop.DBus.Error.ServiceUnknown"
                    | "org.freedesktop.DBus.Error.NameHasNoOwner"
            ) =>
        {
            Failure::ServiceNotRunning
        }
        _ => Failure::UnexpectedReply,
    }
}

async fn read_status(runtime: GlibRuntime) -> Result<Status, Failure> {
    let session = connect(runtime).await?;
    let properties = get_all(&session)
        .await
        .map_err(|error| failure_of(&error))?;
    let text = |name: &str| -> Result<String, Failure> {
        properties
            .get(name)
            .cloned()
            .and_then(|value| String::try_from(value).ok())
            .ok_or(Failure::UnexpectedReply)
    };
    let connection_state = text("ConnectionState")?
        .parse()
        .map_err(|_| Failure::UnexpectedReply)?;
    let (name, address) = properties
        .get("Phone")
        .cloned()
        .and_then(|value| <(String, String)>::try_from(value).ok())
        .ok_or(Failure::UnexpectedReply)?;
    Ok(Status {
        connection_state,
        phone: (!address.is_empty()).then_some(Phone { name, address }),
    })
}

async fn get_all(session: &Connection) -> zbus::Result<HashMap<String, OwnedVariant>> {
    let properties = PropertiesProxy::builder(session)
        .destination(control::BUS_NAME)
        .path(control::PATH)
        .build()
        .await?;
    properties
        .get_all(InterfaceName::try_from(INTERFACE)?)
        .await
        .map_err(Into::into)
}

/// Tells the user about `failure` on standard error.
fn report(failure: Failure) -> ExitCode {
    let message = messages::failure(failure);
    eprintln!("{}", message.text);
    if let Some(NextAction::Command(command)) = message.next_action {
        eprintln!("  {command}");
    }
    ExitCode::FAILURE
}
