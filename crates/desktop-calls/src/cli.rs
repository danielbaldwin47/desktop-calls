//! The CLI: a thin client of the control interface.

use std::collections::HashMap;
use std::process::ExitCode;

use desktop_calls_core::clock::MORNING;
use desktop_calls_core::messages::{self, Failure, NextAction};
use desktop_calls_core::{AudioLocation, ConnectionState, Phone, TimeOfDay, Timestamp};
use glib::MainContext;
use zbus::fdo::PropertiesProxy;
use zbus::names::InterfaceName;
use zbus::{Connection, OwnedVariant};

use crate::glib_runtime::GlibRuntime;
use crate::{clock, control};

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
            let (state, until, _) = &status.calls_off;
            let until = (*until != 0).then(|| clock::local_time(Timestamp(until * 1_000)));
            match messages::calls_off_status(state == "off", until) {
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

/// `desktop-calls calls off [--for LENGTH | --until HH:MM |
/// --until-morning]`: turns calls off until the user turns them on, or
/// until the deadline asked for. The local times become an instant here,
/// by the Desktop's clock; the app decides the rest.
pub fn calls_off(length: Option<&str>, until: Option<&str>, until_morning: bool) -> ExitCode {
    let now = clock::now();
    let deadline = if let Some(length) = length {
        let Some(length) = length_ms(length) else {
            eprintln!("{}", messages::calls_off_for_too_long());
            return ExitCode::FAILURE;
        };
        Some(Timestamp(now.0.saturating_add(length)))
    } else if let Some(until) = until {
        let Some(until) = TimeOfDay::parse(until) else {
            eprintln!("{}", messages::calls_off_until_unreadable());
            return ExitCode::FAILURE;
        };
        Some(clock::next(until, now))
    } else {
        until_morning.then(|| clock::next(MORNING, now))
    };
    let seconds = deadline.map_or(0, |deadline| deadline.0 / 1_000);
    let shown = deadline.map(|_| clock::local_time(Timestamp(seconds * 1_000)));
    turn_calls(
        "TurnCallsOff",
        &(seconds,),
        &messages::calls_turned(true, shown),
    )
}

/// `desktop-calls calls on`: turns calls on.
pub fn calls_on() -> ExitCode {
    turn_calls("TurnCallsOn", &(), &messages::calls_turned(false, None))
}

/// The milliseconds `length` names, such as `90m` or `2h`: a whole number
/// of minutes or hours, from 1 minute to 24 hours.
fn length_ms(length: &str) -> Option<u64> {
    let unit = match length.chars().last()? {
        'm' => 60_000,
        'h' => 3_600_000,
        _ => return None,
    };
    let count = &length[..length.len() - 1];
    if count.is_empty() || !count.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let length = count.parse::<u64>().ok()?.checked_mul(unit)?;
    (1..=24 * 3_600_000).contains(&length).then_some(length)
}

/// Calls `method` with `body` to turn calls off or on, and prints `said`
/// once the app took it, or why it didn't.
fn turn_calls<B>(method: &str, body: &B, said: &str) -> ExitCode
where
    B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
{
    let context = MainContext::default();
    let turned = context.block_on(async {
        let session = connect(GlibRuntime::new(context.clone()))
            .await
            .map_err(Refusal::Failure)?;
        let reply = session
            .call_method(
                Some(control::BUS_NAME),
                control::PATH,
                Some(INTERFACE),
                method,
                body,
            )
            .await;
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
            println!("{said}");
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
