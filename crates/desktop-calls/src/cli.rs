//! The CLI: a thin client of the control interface.

use std::collections::HashMap;
use std::process::ExitCode;

use desktop_calls_core::messages::{self, Failure, NextAction};
use desktop_calls_core::{AudioLocation, ConnectionState, Phone};
use glib::MainContext;
use zbus::fdo::PropertiesProxy;
use zbus::names::InterfaceName;
use zbus::{Connection, OwnedVariant};

use crate::control;
use crate::glib_runtime::GlibRuntime;

pub(crate) const INTERFACE: &str = "io.github.danielbaldwin47.DesktopCalls1";

/// The state `status` reports, read from the control interface.
struct Status {
    connection_state: ConnectionState,
    phone: Option<Phone>,
    /// The Phone is in range, without hands-free.
    hands_free_lost: bool,
    /// Calls Off, as the control interface's `CallsOff` has it.
    calls_off: (String, u64, bool),
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
            let (state, until, pending) = &status.calls_off;
            let printed = serde_json::json!({
                "connection_state": status.connection_state.as_str(),
                "phone": phone,
                "calls_off": { "state": state, "until": until, "pending": pending },
            });
            println!("{printed}");
            ExitCode::SUCCESS
        }
        Ok(status) => {
            // While calls are off the Phone is disconnected by the user's
            // choice, not out of range: that is all the status says.
            match messages::calls_off_status(status.calls_off.0 == "off") {
                Some(line) => println!("{line}"),
                None => println!(
                    "{}",
                    messages::status(
                        &status.connection_state,
                        status.phone.as_ref(),
                        status.hands_free_lost
                    )
                ),
            }
            ExitCode::SUCCESS
        }
        Err(failure) => report(failure),
    }
}

/// `desktop-calls` with no subcommand, which the keybinding runs: focuses
/// the Incoming Call Banner while a Call rings, or the Call Window while one
/// is live, and otherwise opens the Dialer.
pub fn activate() -> ExitCode {
    call_app("Activate", &())
}

/// `desktop-calls dialer [QUERY]`: opens the Dialer with `query` typed.
pub fn dialer(query: &str) -> ExitCode {
    // No Call to select.
    call_app("OpenDialer", &(query, 0_u32))
}

/// `desktop-calls connect`: opens the Setup Window, to pair a Phone, pair
/// it again, or switch to another.
pub fn connect_phone() -> ExitCode {
    call_app("OpenConnectWindow", &())
}

/// `desktop-calls calls off` or `calls on`: turns calls off until the
/// user turns them on, or turns them on.
pub fn calls(off: bool) -> ExitCode {
    let context = MainContext::default();
    let turned = context.block_on(async {
        let session = connect(GlibRuntime::new(context.clone()))
            .await
            .map_err(Refusal::Failure)?;
        let reply = if off {
            session
                .call_method(
                    Some(control::BUS_NAME),
                    control::PATH,
                    Some(INTERFACE),
                    "TurnCallsOff",
                    &(0_u64,),
                )
                .await
        } else {
            session
                .call_method(
                    Some(control::BUS_NAME),
                    control::PATH,
                    Some(INTERFACE),
                    "TurnCallsOn",
                    &(),
                )
                .await
        };
        match reply {
            Ok(_) => Ok(()),
            Err(zbus::Error::MethodError(name, Some(text), _))
                if name.starts_with(control::ERROR_PREFIX) =>
            {
                Err(Refusal::Refused(text))
            }
            Err(error) => Err(Refusal::Failure(failure_of(&error))),
        }
    });
    match turned {
        Ok(()) => {
            println!("{}", messages::calls_turned(off));
            ExitCode::SUCCESS
        }
        Err(Refusal::Failure(failure)) => report(failure),
        Err(Refusal::Refused(text)) => {
            eprintln!("{text}");
            ExitCode::FAILURE
        }
    }
}

/// Calls `method` on the control interface, for a command with nothing to
/// print.
fn call_app<B>(method: &str, body: &B) -> ExitCode
where
    B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
{
    let context = MainContext::default();
    let called = context.block_on(async {
        let session = connect(GlibRuntime::new(context.clone())).await?;
        session
            .call_method(
                Some(control::BUS_NAME),
                control::PATH,
                Some(INTERFACE),
                method,
                body,
            )
            .await
            .map(drop)
            .map_err(|error| failure_of(&error))
    });
    match called {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => report(failure),
    }
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

/// `desktop-calls audio here` or `audio phone`: a Handoff of the Call's
/// audio to `location`.
pub fn audio(location: AudioLocation) -> ExitCode {
    let context = MainContext::default();
    match context.block_on(move_audio(GlibRuntime::new(context.clone()), location)) {
        Ok(already_there) => {
            println!("{}", messages::audio_requested(location, already_there));
            ExitCode::SUCCESS
        }
        Err(Refusal::Failure(failure)) => report(failure),
        Err(Refusal::Refused(text)) => {
            eprintln!("{text}");
            ExitCode::FAILURE
        }
    }
}

/// Asks for the audio at `location`, and returns whether it was there
/// already.
async fn move_audio(runtime: GlibRuntime, location: AudioLocation) -> Result<bool, Refusal> {
    let session = connect(runtime).await.map_err(Refusal::Failure)?;
    let properties = get_all(&session)
        .await
        .map_err(|error| Refusal::Failure(failure_of(&error)))?;
    let already_there = properties
        .get("AudioLocation")
        .cloned()
        .and_then(|value| String::try_from(value).ok())
        .ok_or(Refusal::Failure(Failure::UnexpectedReply))?
        == location.as_str();
    let reply = session
        .call_method(
            Some(control::BUS_NAME),
            control::PATH,
            Some(INTERFACE),
            "MoveAudio",
            &(location.as_str(),),
        )
        .await;
    match reply {
        Ok(_) => Ok(already_there),
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
    // An app from before ticket #216 doesn't publish it.
    let hands_free_lost = properties
        .get("HandsFreeLost")
        .and_then(|value| bool::try_from(value).ok())
        .unwrap_or(false);
    // Nor does one from before ticket #202.
    let calls_off = properties
        .get("CallsOff")
        .cloned()
        .and_then(|value| <(String, u64, bool)>::try_from(value).ok())
        .unwrap_or_else(|| ("on".to_owned(), 0, false));
    Ok(Status {
        connection_state,
        phone: (!address.is_empty()).then_some(Phone { name, address }),
        hands_free_lost,
        calls_off,
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

/// `desktop-calls dial <number|tel: link>`, and a `tel:` link opened: asks
/// the service to Call it. Silent, and successful, once the Phone reports the
/// Call; otherwise fails with the sentence the Call Window shows, once the
/// Phone refuses or the service gives up waiting for it.
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
