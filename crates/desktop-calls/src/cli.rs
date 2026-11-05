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

/// Why a command the CLI sent didn't happen.
enum Refusal {
    /// The CLI couldn't talk to the long-running process.
    Failure(Failure),
    /// The long-running process refused, with this sentence for the user.
    Refused(String),
}

/// `desktop-calls mute`: mutes the Desktop's microphone, or unmutes it if
/// it is muted.
pub fn mute() -> ExitCode {
    let context = MainContext::default();
    match context.block_on(toggle_mute(GlibRuntime::new(context.clone()))) {
        Ok(muted) => {
            println!("{}", messages::mute_toggled(muted));
            ExitCode::SUCCESS
        }
        Err(Refusal::Failure(failure)) => report(failure),
        Err(Refusal::Refused(text)) => {
            eprintln!("{text}");
            ExitCode::FAILURE
        }
    }
}

/// Flips `Muted`, and returns what it is now.
async fn toggle_mute(runtime: GlibRuntime) -> Result<bool, Refusal> {
    let session = connect(runtime).await.map_err(Refusal::Failure)?;
    let properties = get_all(&session)
        .await
        .map_err(|error| Refusal::Failure(failure_of(&error)))?;
    let muted = properties
        .get("Muted")
        .cloned()
        .and_then(|value| bool::try_from(value).ok())
        .ok_or(Refusal::Failure(Failure::UnexpectedReply))?;
    let reply = session
        .call_method(
            Some(control::BUS_NAME),
            control::PATH,
            Some(INTERFACE),
            "SetMuted",
            &(!muted,),
        )
        .await;
    match reply {
        Ok(_) => Ok(!muted),
        Err(zbus::Error::MethodError(name, Some(text), _))
            if name.starts_with(control::ERROR_PREFIX) =>
        {
            Err(Refusal::Refused(text))
        }
        Err(error) => Err(Refusal::Failure(failure_of(&error))),
    }
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

/// `desktop-calls dial <number|tel: link>`: asks the service to Call it.
/// Silent once the Phone has been asked to dial; otherwise says why not.
pub fn dial(number: &str) -> ExitCode {
    let context = MainContext::default();
    let outcome = context.block_on(request_dial(GlibRuntime::new(context.clone()), number));
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(Refusal::Refused(sentence)) => {
            eprintln!("{sentence}");
            ExitCode::FAILURE
        }
        Err(Refusal::Failure(failure)) => report(failure),
    }
}

async fn request_dial(runtime: GlibRuntime, number: &str) -> Result<(), Refusal> {
    let session = connect(runtime).await.map_err(Refusal::Failure)?;
    let reply = session
        .call_method(
            Some(control::BUS_NAME),
            control::PATH,
            Some(INTERFACE),
            "Dial",
            &(number,),
        )
        .await;
    match reply {
        Ok(_) => Ok(()),
        Err(zbus::Error::MethodError(name, Some(sentence), _))
            if name.starts_with(control::ERROR_PREFIX) =>
        {
            Err(Refusal::Refused(sentence))
        }
        Err(error) => Err(Refusal::Failure(failure_of(&error))),
    }
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
