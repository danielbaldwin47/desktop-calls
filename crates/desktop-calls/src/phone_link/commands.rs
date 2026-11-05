//! Carrying out the Phone link's Phone commands: one D-Bus call at a time
//! to Telephony, or to BlueZ for the Handoff to the Phone, with the
//! fallbacks and error readings the Phones need.

use desktop_calls_core::{Input, PhoneCommand, PhoneRefusal};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures_util::StreamExt as _;
use zbus::{Connection, Variant};

use super::{AUDIO_GATEWAY, CALL, Resolved, SERVICE, TRANSPORT, Target};
use crate::bluez;
use crate::logging::EventKind;

const OFONO_CALL: &str = "org.ofono.VoiceCall";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// Carries out each batch of Phone commands in order, one D-Bus call at a
/// time, and reports each command done, or the first failure of each batch.
pub(super) async fn execute(
    session: &Connection,
    system: &Connection,
    mut batches: UnboundedReceiver<Vec<Resolved>>,
    inputs: &UnboundedSender<Input>,
) {
    while let Some(batch) = batches.next().await {
        for Resolved { command, target } in batch {
            let path = match target {
                Target::Object(path) => path,
                Target::Phone(address) => {
                    // The bounce logs its own failure.
                    let input = match bluez::bounce_hands_free(system, &address).await {
                        Ok(()) => {
                            tracing::debug!(
                                event_kind = %EventKind::PhoneCommandSent,
                                command = "MoveAudioToPhone",
                                "Hands-free dropped and connected again"
                            );
                            Input::PhoneCommandDone(command)
                        }
                        Err(_) => Input::PhoneCommandFailed(command),
                    };
                    let failed = matches!(input, Input::PhoneCommandFailed(_));
                    let _ = inputs.unbounded_send(input);
                    if failed {
                        break;
                    }
                    continue;
                }
            };
            let result = match &command {
                PhoneCommand::Answer(_) => call(session, &path, CALL, "Answer", &()).await,
                PhoneCommand::HangUp(_) => hang_up(session, &path).await,
                PhoneCommand::MoveAudioToDesktop => {
                    match set_reject_sco(session, &path, false).await {
                        Ok(()) => call(session, &path, TRANSPORT, "Activate", &()).await,
                        error => error,
                    }
                }
                PhoneCommand::KeepAudioOnPhone => set_reject_sco(session, &path, true).await,
                PhoneCommand::AllowAudioOnDesktop => set_reject_sco(session, &path, false).await,
                PhoneCommand::Dial(number) => {
                    tracing::debug!(
                        event_kind = %EventKind::PhoneCommandSent,
                        phone_number = %number,
                        "Asking the Phone to dial"
                    );
                    call(session, &path, AUDIO_GATEWAY, "Dial", &(number.as_str(),)).await
                }
                PhoneCommand::SetMicrophoneVolume(volume) => {
                    set_microphone_volume(session, &path, *volume).await
                }
                PhoneCommand::SendTones(tones) => send_tones(session, &path, tones).await,
                // PipeWire's AT+CHLD=2, AT+CHLD=1 and AT+CHLD=2 again: each
                // acts on whichever Calls are active, held and waiting.
                PhoneCommand::HoldAndAnswer(_) => {
                    call(session, &path, AUDIO_GATEWAY, "HoldAndAnswer", &()).await
                }
                PhoneCommand::EndAndAnswer(_) => {
                    call(session, &path, AUDIO_GATEWAY, "ReleaseAndAnswer", &()).await
                }
                PhoneCommand::Swap => call(session, &path, AUDIO_GATEWAY, "SwapCalls", &()).await,
                // Resolved to the Phone itself, above.
                PhoneCommand::MoveAudioToPhone => Ok(()),
            };
            let name = command_name(&command);
            match result {
                Ok(()) => {
                    tracing::debug!(
                        event_kind = %EventKind::PhoneCommandSent,
                        command = %name,
                        "The Phone did {name}"
                    );
                    let _ = inputs.unbounded_send(Input::PhoneCommandDone(command));
                }
                Err(error) => {
                    tracing::warn!(
                        event_kind = %EventKind::PhoneCommandFailed,
                        error_kind = error_kind(&error),
                        command = %name,
                        detail = %error,
                        "The Phone refused {name}"
                    );
                    let input = match command {
                        PhoneCommand::Dial(_) => Input::DialFailed(dial_refusal(&error)),
                        command => Input::PhoneCommandFailed(command),
                    };
                    let _ = inputs.unbounded_send(input);
                    break;
                }
            }
        }
    }
}

/// A Phone command as the log names it: a dialled number goes only in a
/// `phone_number` field, never in a command's name.
pub(super) fn command_name(command: &PhoneCommand) -> String {
    match command {
        PhoneCommand::Dial(_) => "Dial".to_owned(),
        command => format!("{command:?}"),
    }
}

/// Why the Phone refused to dial. PipeWire passes a `+CME ERROR` on only as
/// its description (`docs/research/2026-11-04-place-a-call.md`).
fn dial_refusal(error: &zbus::Error) -> PhoneRefusal {
    match error_name(error) {
        Some("org.pipewire.Telephony.Error.InvalidFormat") => PhoneRefusal::InvalidFormat,
        Some("org.pipewire.Telephony.Error.InvalidState") => PhoneRefusal::InvalidState,
        Some("org.pipewire.Telephony.Error.InProgress") => PhoneRefusal::InProgress,
        Some("org.pipewire.Telephony.Error.NotSupported") => PhoneRefusal::NotSupported,
        Some("org.pipewire.Telephony.Error.CME") => match error_text(error).as_str() {
            "dial string too long" | "invalid characters in dial string" => {
                PhoneRefusal::InvalidFormat
            }
            "no network service" | "network not allowed - Emergency calls only" => {
                PhoneRefusal::NoNetwork
            }
            "network Timeout" => PhoneRefusal::NetworkTimeout,
            _ => PhoneRefusal::Failed,
        },
        _ => PhoneRefusal::Failed,
    }
}

async fn call<B>(
    session: &Connection,
    path: &str,
    interface: &str,
    member: &str,
    body: &B,
) -> zbus::Result<()>
where
    B: zbus::export::serde::Serialize + zbus::DynamicType,
{
    session
        .call_method(Some(SERVICE), path, Some(interface), member, body)
        .await
        .map(drop)
}

async fn set_reject_sco(session: &Connection, path: &str, reject: bool) -> zbus::Result<()> {
    call(
        session,
        path,
        PROPERTIES,
        "Set",
        &(TRANSPORT, "RejectSCO", Variant::from(reject)),
    )
    .await
}

async fn set_microphone_volume(session: &Connection, path: &str, volume: u8) -> zbus::Result<()> {
    call(
        session,
        path,
        PROPERTIES,
        "Set",
        &(AUDIO_GATEWAY, "MicrophoneVolume", Variant::from(volume)),
    )
    .await
}

/// Plays `tones` one at a time, each once the Phone has played the last.
async fn send_tones(session: &Connection, path: &str, tones: &str) -> zbus::Result<()> {
    let mut tone = [0; 4];
    for character in tones.chars() {
        let tone: &str = character.encode_utf8(&mut tone);
        call(session, path, AUDIO_GATEWAY, "SendTones", &(tone,)).await?;
    }
    Ok(())
}

/// Hangs up the Call at `path`: through `Call1`, or `org.ofono.VoiceCall`
/// where `Call1`'s method doesn't exist. An error that says the Call is
/// already going, or gone, means it ended.
async fn hang_up(session: &Connection, path: &str) -> zbus::Result<()> {
    let result = match call(session, path, CALL, "Hangup", &()).await {
        Err(error) if is_missing_method(&error) => {
            // At info, so the journal records which hang-up path the Phone
            // needs; the Real-Phone Checklist reports it.
            tracing::info!(
                event_kind = %EventKind::PhoneCommandFallback,
                detail = %error,
                "Call1.Hangup doesn't exist here; hanging up through org.ofono.VoiceCall"
            );
            call(session, path, OFONO_CALL, "Hangup", &()).await
        }
        result => result,
    };
    match result {
        Err(error) if is_already_ending(&error) => {
            tracing::debug!(
                event_kind = %EventKind::PhoneCommandSent,
                detail = %error,
                "The Call was already ending"
            );
            Ok(())
        }
        result => result,
    }
}

fn error_name(error: &zbus::Error) -> Option<&str> {
    match error {
        zbus::Error::MethodError(name, _, _) => Some(name.as_str()),
        zbus::Error::FDO(error) => Some(match **error {
            zbus::fdo::Error::UnknownMethod(_) => "org.freedesktop.DBus.Error.UnknownMethod",
            zbus::fdo::Error::UnknownObject(_) => "org.freedesktop.DBus.Error.UnknownObject",
            zbus::fdo::Error::InvalidArgs(_) => "org.freedesktop.DBus.Error.InvalidArgs",
            _ => return None,
        }),
        _ => None,
    }
}

fn error_text(error: &zbus::Error) -> String {
    match error {
        zbus::Error::MethodError(_, Some(text), _) => text.clone(),
        zbus::Error::FDO(error) => error.to_string(),
        _ => String::new(),
    }
}

/// libdbus's reply for a method nobody handles: 'Method "Hangup" with
/// signature "" on interface "…" doesn't exist'.
fn is_missing_method(error: &zbus::Error) -> bool {
    error_name(error) == Some("org.freedesktop.DBus.Error.UnknownMethod")
        && error_text(error).contains("doesn't exist")
}

/// A hang-up error for a Call that is already being torn down or is gone:
/// "Too few arguments" while it tears down, `InvalidState` once it is
/// `disconnected`, or no object at all.
fn is_already_ending(error: &zbus::Error) -> bool {
    match error_name(error) {
        Some("org.freedesktop.DBus.Error.InvalidArgs") => {
            error_text(error).contains("Too few arguments")
        }
        Some(
            "org.pipewire.Telephony.Error.InvalidState"
            | "org.freedesktop.DBus.Error.UnknownObject"
            | "org.freedesktop.DBus.Error.UnknownMethod",
        ) => true,
        _ => false,
    }
}

/// The kind of a Phone command failure, for the log.
fn error_kind(error: &zbus::Error) -> &'static str {
    match error_name(error) {
        Some("org.pipewire.Telephony.Error.InvalidState") => "invalid-state",
        Some("org.pipewire.Telephony.Error.InvalidFormat") => "invalid-format",
        Some("org.pipewire.Telephony.Error.InProgress") => "in-progress",
        Some("org.pipewire.Telephony.Error.NotSupported") => "not-supported",
        Some("org.pipewire.Telephony.Error.CME") => "cme",
        Some("org.pipewire.Telephony.Error.Failed") => "failed",
        Some(_) => "dbus-error",
        None => "no-reply",
    }
}
