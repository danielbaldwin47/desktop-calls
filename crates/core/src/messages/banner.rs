//! What the Incoming Call Banner says beyond its labels: what each Waiting
//! Call choice does to the user's other Call, and its footer.

use super::{caller_name, setup::shown_keys};
use crate::machine::{BannerChoice, BannerFooter, BannerView, OtherCall, WaitingOptions};

/// One piece of the Banner's footer line: a key, in a keycap, or words
/// between keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FooterPart {
    Key(String),
    Words(&'static str),
}

/// What `choice` does to the user's other Call, said under its label: "Alice
/// Smith waits on hold", "Ends the Call with Alice". `None` for a choice
/// that leaves it as it is, and for the only Call ringing.
#[must_use]
pub fn banner_consequence(view: &BannerView, choice: BannerChoice) -> Option<String> {
    let other = view.other.as_ref()?;
    match (view.waiting?, choice) {
        (WaitingOptions::HoldOrEnd, BannerChoice::HoldAndAnswer) => {
            Some(format!("{} waits on hold", full_name(other)))
        }
        (WaitingOptions::HoldOrEnd | WaitingOptions::End, BannerChoice::EndAndAnswer) => {
            Some(format!("Ends the Call with {}", first_name(other)))
        }
        (WaitingOptions::Answer, BannerChoice::Answer) => {
            Some(format!("{} stays on hold", full_name(other)))
        }
        _ => None,
    }
}

/// A Contact's name, or the number as shown, held together by no-break
/// spaces so a wrapping line never splits it.
fn full_name(other: &OtherCall) -> String {
    let name = caller_name(&other.caller, other.contact.as_ref());
    match &other.contact {
        Some(contact) if !contact.name.trim().is_empty() => name,
        _ => name.replace(' ', "\u{a0}"),
    }
}

/// A Contact's first name, or the number as shown.
fn first_name(other: &OtherCall) -> String {
    let name = full_name(other);
    match &other.contact {
        Some(contact) if !contact.name.trim().is_empty() => name
            .split_whitespace()
            .next()
            .map_or_else(|| name.clone(), str::to_owned),
        _ => name,
    }
}

/// The footer line, with `letters` the keys of the Banner's choices in
/// order: "Super Alt C then A or D", "No keybinding set. Add one in Connect
/// your phone…", or "Esc gives the keyboard back; the Call keeps ringing".
#[must_use]
pub fn banner_footer(footer: &BannerFooter, letters: &[&str]) -> Vec<FooterPart> {
    match footer {
        BannerFooter::Keybinding(keys) => {
            let mut parts: Vec<FooterPart> = keys
                .iter()
                .map(|key| FooterPart::Key(shown_keys(key)))
                .collect();
            parts.push(FooterPart::Words("then"));
            for (at, letter) in letters.iter().enumerate() {
                if at > 0 {
                    parts.push(FooterPart::Words(if at + 1 == letters.len() {
                        "or"
                    } else {
                        ","
                    }));
                }
                parts.push(FooterPart::Key((*letter).to_owned()));
            }
            parts
        }
        BannerFooter::NoKeybinding => vec![FooterPart::Words(NO_KEYBINDING)],
        BannerFooter::GiveKeyboardBack => vec![
            FooterPart::Key("Esc".to_owned()),
            FooterPart::Words("gives the keyboard back; the Call keeps ringing"),
        ],
    }
}

/// What the footer says with no keybinding set: where to add one, in the
/// Indicator's own words for the entry (a test holds them alike).
const NO_KEYBINDING: &str = "No keybinding set. Add one in Connect your phone…";

/// The footer as one line of text, as the control interface reports it: a
/// comma sits against the key before it.
#[must_use]
pub fn footer_text(parts: &[FooterPart]) -> String {
    let mut text = String::new();
    for part in parts {
        let words = match part {
            FooterPart::Key(key) => key.as_str(),
            FooterPart::Words(words) => words,
        };
        if !text.is_empty() && *words != *"," {
            text.push(' ');
        }
        text.push_str(words);
    }
    text
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_no_keybinding_footer_names_the_indicators_own_entry() {
        assert!(super::NO_KEYBINDING.ends_with(super::super::CONNECT_YOUR_PHONE));
    }
}
