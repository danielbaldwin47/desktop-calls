//! What the Call Window says: its state line, its notice while contact
//! with the Call is lost, and its timer.

use super::{Failure, Label, caller_name, failure, label};
use crate::call::Caller;
use crate::machine::StateLine;
use crate::matching::ContactMatch;

/// What the Call Window's state line says for `line`, about a Call with
/// `caller`, who is the Contact `contact` if Contacts say.
#[must_use]
pub fn state_line(line: StateLine, caller: &Caller, contact: Option<&ContactMatch>) -> String {
    match line {
        StateLine::Connecting => label(Label::Connecting).to_owned(),
        StateLine::ConnectingToPhone => "Connecting to your phone…".to_owned(),
        StateLine::Calling => label(Label::Calling).to_owned(),
        StateLine::Ringing => label(Label::Ringing).to_owned(),
        StateLine::OnThisComputer { .. } => "On this computer".to_owned(),
        StateLine::Muted => format!("Muted. {} can't hear you", first_name(caller, contact)),
        StateLine::OnYourPhone => "On your phone. Mute there, or move the audio here".to_owned(),
        StateLine::OnHold => label(Label::OnHold).to_owned(),
        StateLine::Ended { lasted: None } => label(Label::CallEnded).to_owned(),
        StateLine::Ended {
            lasted: Some(seconds),
        } => format!("{} · {}", label(Label::CallEnded), elapsed(seconds)),
    }
}

/// The other party as a sentence names them: a Contact by their first
/// name, and anyone else as "They".
fn first_name(caller: &Caller, contact: Option<&ContactMatch>) -> String {
    let named = contact.filter(|contact| !contact.name.trim().is_empty());
    match named {
        Some(_) => caller_name(caller, named)
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned(),
        None => "They".to_owned(),
    }
}

/// What the Call Window's notice says while the Desktop has lost contact
/// with the Call, which carries on on the Phone, for `notice`: a heading,
/// and what to do.
#[must_use]
pub fn contact_lost(notice: Failure) -> (&'static str, &'static str) {
    let next = match notice {
        Failure::ConnectionLost => "The Call continues on your phone. Hang up there.",
        other => failure(other).text,
    };
    (label(Label::ConnectionToPhoneLost), next)
}

/// The Call timer's text for `seconds`: `m:ss`, or `h:mm:ss` from an hour.
#[must_use]
pub fn elapsed(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}
