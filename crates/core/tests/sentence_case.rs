//! Ticket #237: the interface's copy is in sentence case. The glossary's
//! capitals (Call, Phone, Contacts, Waiting Call…) stay in docs, code and
//! test names; on screen they are lower case unless they start a sentence.
//!
//! Messages is the one place on-screen text lives, so this reads its
//! source: every string literal outside its tests and comments.

/// Messages' source files, by name.
const MESSAGES: &[(&str, &str)] = &[
    ("messages.rs", include_str!("../src/messages.rs")),
    ("banner.rs", include_str!("../src/messages/banner.rs")),
    ("call_pill.rs", include_str!("../src/messages/call_pill.rs")),
    ("calls_off.rs", include_str!("../src/messages/calls_off.rs")),
    (
        "call_window.rs",
        include_str!("../src/messages/call_window.rs"),
    ),
    ("checks.rs", include_str!("../src/messages/checks.rs")),
    ("dialer.rs", include_str!("../src/messages/dialer.rs")),
    ("problems.rs", include_str!("../src/messages/problems.rs")),
    (
        "quiet_hours.rs",
        include_str!("../src/messages/quiet_hours.rs"),
    ),
    (
        "setup_parts.rs",
        include_str!("../src/messages/setup_parts.rs"),
    ),
    ("setup.rs", include_str!("../src/messages/setup.rs")),
    ("when.rs", include_str!("../src/messages/when.rs")),
];

/// The glossary's terms, longest first, so "Waiting Call" is judged before
/// "Call".
const TERMS: &[&str] = &[
    "Incoming Call Banner",
    "Quiet Hours Window",
    "Audio Location",
    "Missed Calls",
    "Missed Call",
    "Hold & Answer",
    "End & Answer",
    "Call History",
    "Waiting Call",
    "Call Window",
    "Setup Window",
    "Quiet Hours",
    "State Line",
    "Held Call",
    "Calls Off",
    "Call Pill",
    "Contacts",
    "Contact",
    "Calls",
    "Call",
    "Phone",
    "Desktop",
    "Handoff",
    "Dialer",
    "Banner",
    "Indicator",
    "Problems",
    "Problem",
];

/// Capitals that stay, mid-sentence too.
const ALLOWED: &[&str] = &[
    // The app's name.
    "Desktop Calls",
    // Omarchy's own feature, by its name there.
    "Do Not Disturb",
    // The Indicator's menu entry, quoted by name.
    "Phone setup",
    // The phone's Bluetooth settings, quoted as the phone shows them.
    "Phone calls",
    "Contacts and call history",
];

/// The string literals of `source` outside its tests and line comments.
fn literals(source: &str) -> Vec<String> {
    let code = source.split("#[cfg(test)]").next().unwrap_or_default();
    let mut found = Vec::new();
    for line in code.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            if c != '"' {
                continue;
            }
            let mut literal = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        literal.push(c);
                        literal.extend(chars.next());
                    }
                    '"' => break,
                    _ => literal.push(c),
                }
            }
            found.push(literal);
        }
    }
    found
}

/// The terms `literal` writes with a capital that sentence case lowers:
/// anywhere but at the start of a sentence, and a several-word term's
/// later words anywhere.
fn capitals(literal: &str) -> Vec<&'static str> {
    let mut masked = literal.to_owned();
    for allowed in ALLOWED {
        masked = masked.replace(allowed, &"_".repeat(allowed.len()));
    }
    let mut found = Vec::new();
    for term in TERMS {
        while let Some(at) = whole_word(&masked, term) {
            let before = masked[..at].trim_end();
            let starts = before.is_empty() || before.ends_with(['.', '!', '?']);
            if !starts || term.contains(' ') {
                found.push(*term);
            }
            masked.replace_range(at..at + term.len(), &"_".repeat(term.len()));
        }
    }
    found
}

/// Where `term` first stands as whole words in `text`.
fn whole_word(text: &str, term: &str) -> Option<usize> {
    text.match_indices(term).map(|(at, _)| at).find(|&at| {
        let before = text[..at].chars().next_back();
        let after = text[at + term.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

#[test]
fn no_glossary_capital_stands_in_the_middle_of_an_on_screen_sentence() {
    let mut wrong = Vec::new();
    for (file, source) in MESSAGES {
        for literal in literals(source) {
            let capitals = capitals(&literal);
            if !capitals.is_empty() {
                wrong.push(format!("{file}: {literal:?} has {capitals:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_check_finds_a_capital_mid_sentence_and_lets_one_start_a_sentence() {
    assert_eq!(capitals("Hang up the current Call first."), ["Call"]);
    assert_eq!(
        capitals("Calls stay on. The call ends."),
        Vec::<&str>::new()
    );
    assert_eq!(capitals("Waiting Call"), ["Waiting Call"]);
    assert_eq!(capitals("Pin Desktop Calls in the bar"), Vec::<&str>::new());
}
