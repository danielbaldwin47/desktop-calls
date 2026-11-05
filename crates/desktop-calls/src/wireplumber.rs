//! The Desktop's audio settings for Calls, as `WirePlumber` 0.5 reads them
//! from the user's drop-ins, and setting them up.
//!
//! - Where: `$XDG_CONFIG_HOME/wireplumber/wireplumber.conf.d/*.conf`
//!   (`~/.config` without it), loaded in file-name order, a later value
//!   overriding an earlier one. The packaged `wireplumber.conf` sets none
//!   of the three properties below (`WirePlumber` 0.5.18 on Arch); system
//!   drop-ins in `/etc/wireplumber` aren't read.
//! - What Calls need:
//!   - `bluez5.roles` in `monitor.bluez.properties` includes `hfp_hf`, or
//!     is unset: its default includes it.
//!   - `bluez5.auto-connect`, set by a `monitor.bluez.rules` rule's
//!     `update-props`, includes `hfp_hf`. Unset, nothing connects by itself.
//!   - `bluez5.telephony.default-reject-sco` in `monitor.bluez.properties`
//!     is `true`, so a Call's audio stays on the Phone until asked for.
//! - Setting up writes one drop-in, [`DROP_IN`], holding each property it
//!   changes, and comments the same property out wherever else the user's
//!   drop-ins set it, so each is set in exactly one file and file-name
//!   order can't undo it. A property already set right elsewhere is left
//!   there. A [`DROP_IN`] that Desktop Calls didn't write (it lacks
//!   [`HEADER`]) is the user's: it is edited like any other, and the
//!   settings go in [`OTHER_DROP_IN`] instead, so nothing of the user's is
//!   lost.
//!
//! The files are SPA-JSON: JSON whose keys and strings needn't be quoted,
//! with `=` or `:` between key and value, optional commas, `#` comments,
//! and a file that is an object without its braces.

use std::env;
use std::fs;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// The drop-in setting up writes.
const DROP_IN: &str = "60-desktop-calls.conf";
/// The drop-in setting up writes when the user has a [`DROP_IN`] of their own.
const OTHER_DROP_IN: &str = "60-desktop-calls-audio.conf";
/// The first line of what setting up writes, which marks it as Desktop Calls'.
const HEADER: &str = "# Written by Desktop Calls when you chose Set up audio: hands-free with";
/// The hands-free role: the Desktop as the Phone's hands-free unit.
const HANDS_FREE: &str = "hfp_hf";

/// The user's drop-in folder.
pub fn drop_in_folder() -> Option<PathBuf> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .filter(|config| !config.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("wireplumber/wireplumber.conf.d"))
}

/// A property Calls need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Property {
    Roles,
    AutoConnect,
    RejectSco,
}

impl Property {
    const ALL: [Self; 3] = [Self::Roles, Self::AutoConnect, Self::RejectSco];

    fn key(self) -> &'static str {
        match self {
            Self::Roles => "bluez5.roles",
            Self::AutoConnect => "bluez5.auto-connect",
            Self::RejectSco => "bluez5.telephony.default-reject-sco",
        }
    }
}

/// One place a drop-in sets a property: the text from its key to the end
/// of its value, and the value's words.
#[derive(Clone, Debug)]
struct Setting {
    property: Property,
    span: Range<usize>,
    /// The value's words: an array's items, or the one word.
    words: Vec<String>,
}

/// One of the user's drop-ins.
struct DropIn {
    name: String,
    text: String,
    settings: Vec<Setting>,
}

/// The user's drop-ins, in the order `WirePlumber` loads them.
pub struct Settings {
    folder: PathBuf,
    drop_ins: Vec<DropIn>,
}

impl Settings {
    /// Reads the drop-ins in `folder`; none if it isn't there.
    pub fn read(folder: &Path) -> io::Result<Self> {
        let mut drop_ins = Vec::new();
        let entries = match fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self {
                    folder: folder.to_owned(),
                    drop_ins,
                });
            }
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.')
                || Path::new(&name)
                    .extension()
                    .is_none_or(|extension| extension != "conf")
                // A link counts as the file it points to.
                || !fs::metadata(entry.path()).is_ok_and(|metadata| metadata.is_file())
            {
                continue;
            }
            let text = fs::read_to_string(entry.path())?;
            let settings = settings_in(&text);
            drop_ins.push(DropIn {
                name,
                text,
                settings,
            });
        }
        drop_ins.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Self {
            folder: folder.to_owned(),
            drop_ins,
        })
    }

    /// The value `WirePlumber` ends up with for `property`, if any drop-in
    /// sets it.
    fn effective(&self, property: Property) -> Option<&Setting> {
        self.drop_ins
            .iter()
            .flat_map(|drop_in| &drop_in.settings)
            .rfind(|setting| setting.property == property)
    }

    /// The words `property` needs, if it needs changing.
    fn needed(&self, property: Property) -> Option<Vec<String>> {
        let effective = self.effective(property);
        let words = effective.map(|setting| setting.words.clone());
        match property {
            Property::Roles => {
                let mut words = words?;
                (!words.iter().any(|word| word == HANDS_FREE)).then(|| {
                    words.push(HANDS_FREE.to_owned());
                    words
                })
            }
            Property::AutoConnect => {
                let mut words = words.unwrap_or_default();
                (!words.iter().any(|word| word == HANDS_FREE)).then(|| {
                    words.push(HANDS_FREE.to_owned());
                    words
                })
            }
            Property::RejectSco => (words.as_deref() != Some(&["true".to_owned()][..]))
                .then(|| vec!["true".to_owned()]),
        }
    }

    /// Whether the Desktop's audio is set up for Calls.
    pub fn set_up(&self) -> bool {
        Property::ALL
            .iter()
            .all(|property| self.needed(*property).is_none())
    }

    /// Sets the audio up for Calls: writes [`DROP_IN`] (or
    /// [`OTHER_DROP_IN`]), and comments out in the other drop-ins each
    /// property it now holds. Each file is replaced whole, that one first,
    /// so a failure to write it leaves the others as they were.
    pub fn write_set_up(&self) -> io::Result<()> {
        let users_own = self
            .drop_ins
            .iter()
            .any(|drop_in| drop_in.name == DROP_IN && !drop_in.text.starts_with(HEADER));
        let name = if users_own { OTHER_DROP_IN } else { DROP_IN };
        let mut ours: Vec<(Property, Vec<String>)> = Vec::new();
        for property in Property::ALL {
            let in_ours = self.drop_ins.iter().any(|drop_in| {
                drop_in.name == name
                    && drop_in
                        .settings
                        .iter()
                        .any(|setting| setting.property == property)
            });
            if let Some(words) = self.needed(property) {
                ours.push((property, words));
            } else if in_ours && let Some(setting) = self.effective(property) {
                ours.push((property, setting.words.clone()));
            }
        }
        fs::create_dir_all(&self.folder)?;
        let mut writes = vec![(name.to_owned(), drop_in_text(&ours))];
        for drop_in in self.drop_ins.iter().filter(|drop_in| drop_in.name != name) {
            let mut moved: Vec<&Setting> = drop_in
                .settings
                .iter()
                .filter(|setting| {
                    ours.iter()
                        .any(|(property, _)| *property == setting.property)
                })
                .collect();
            if moved.is_empty() {
                continue;
            }
            moved.sort_by_key(|setting| std::cmp::Reverse(setting.span.start));
            let mut text = drop_in.text.clone();
            for setting in moved {
                let replacement = commented_out(&drop_in.text, &setting.span, name);
                text.replace_range(setting.span.clone(), &replacement);
            }
            writes.push((drop_in.name.clone(), text));
        }
        for (name, text) in writes {
            replace(&self.folder, &name, &text)?;
        }
        Ok(())
    }
}

/// Replaces `folder/name` with `text` whole, by renaming a new file over it.
/// A link stays a link: the file it points to is replaced.
fn replace(folder: &Path, name: &str, text: &str) -> io::Result<()> {
    let mut path = folder.join(name);
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_symlink()) {
        path = fs::canonicalize(&path)?;
    }
    let file_name = path.file_name().map_or_else(
        || name.to_owned(),
        |file_name| file_name.to_string_lossy().into_owned(),
    );
    let temporary = path.with_file_name(format!(".{file_name}.desktop-calls"));
    fs::write(&temporary, text)
        .and_then(|()| keep_permissions(&path, &temporary))
        .and_then(|()| fs::rename(&temporary, &path))
        .inspect_err(|_| {
            let _ = fs::remove_file(&temporary);
        })
}

/// Gives `temporary` the permission bits `target` has, if it exists; a new
/// file keeps the ones it was made with.
fn keep_permissions(target: &Path, temporary: &Path) -> io::Result<()> {
    match fs::metadata(target) {
        Ok(existing) => fs::set_permissions(temporary, existing.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// What a moved setting leaves behind: itself, commented out, and the rest
/// of its last line on a line of its own.
fn commented_out(text: &str, span: &Range<usize>, to: &str) -> String {
    let mut replacement = format!(
        "# Moved to {to} by Desktop Calls: {}",
        text[span.clone()].replace('\n', "\n# ")
    );
    let rest_of_line = text[span.end..].split('\n').next().unwrap_or_default();
    if !rest_of_line.trim().is_empty() {
        replacement.push('\n');
    }
    replacement
}

/// The text of the drop-in setting up writes, holding `settings`.
fn drop_in_text(settings: &[(Property, Vec<String>)]) -> String {
    let value = |property: Property| {
        settings
            .iter()
            .find(|(setting, _)| *setting == property)
            .map(|(_, words)| words)
    };
    let mut lines: Vec<String> = [
        HEADER,
        "# your Phone connects by itself, and a Call's audio stays on the Phone",
        "# until you move it here. Each of these is set in this file alone.",
    ]
    .map(str::to_owned)
    .into();
    let roles = value(Property::Roles);
    let reject = value(Property::RejectSco);
    if roles.is_some() || reject.is_some() {
        lines.push("monitor.bluez.properties = {".to_owned());
        if let Some(roles) = roles {
            lines.push(format!(
                "  {} = [ {} ]",
                Property::Roles.key(),
                roles.join(" ")
            ));
        }
        if let Some(reject) = reject {
            lines.push(format!(
                "  {} = {}",
                Property::RejectSco.key(),
                reject.join(" ")
            ));
        }
        lines.push("}".to_owned());
    }
    if let Some(auto_connect) = value(Property::AutoConnect) {
        lines.extend(
            [
                "monitor.bluez.rules = [",
                "  {",
                "    matches = [ { device.name = \"~bluez_card.*\" } ]",
                "    actions = {",
                "      update-props = {",
            ]
            .map(str::to_owned),
        );
        lines.push(format!(
            "        {} = [ {} ]",
            Property::AutoConnect.key(),
            auto_connect.join(" ")
        ));
        lines.extend(["      }", "    }", "  }", "]"].map(str::to_owned));
    }
    lines.push(String::new());
    lines.join("\n")
}

/// Every setting of a property Calls need in a drop-in's `text`.
fn settings_in(text: &str) -> Vec<Setting> {
    let mut parser = Parser { text, at: 0 };
    let file = parser.members(None);
    let mut settings = Vec::new();
    for member in &file {
        match (member.key.as_str(), &member.value.kind) {
            ("monitor.bluez.properties", Kind::Object(properties)) => {
                for property in [Property::Roles, Property::RejectSco] {
                    settings.extend(find(properties, property));
                }
            }
            ("monitor.bluez.rules", Kind::Array(rules)) => {
                for rule in rules {
                    let Some(actions) = rule.get("actions") else {
                        continue;
                    };
                    let Some(Kind::Object(props)) =
                        actions.get("update-props").map(|props| &props.kind)
                    else {
                        continue;
                    };
                    settings.extend(find(props, Property::AutoConnect));
                }
            }
            _ => {}
        }
    }
    settings.sort_by_key(|setting| setting.span.start);
    settings
}

fn find(members: &[Member], property: Property) -> Vec<Setting> {
    members
        .iter()
        .filter(|member| member.key == property.key())
        .map(|member| Setting {
            property,
            span: member.start..member.value.end,
            words: member.value.words(),
        })
        .collect()
}

/// A key and its value.
struct Member {
    key: String,
    /// Where the key starts.
    start: usize,
    value: Node,
}

struct Node {
    kind: Kind,
    /// Where the value ends.
    end: usize,
}

enum Kind {
    Object(Vec<Member>),
    Array(Vec<Node>),
    Word(String),
}

impl Node {
    /// The member `key` of an object.
    fn get(&self, key: &str) -> Option<&Node> {
        match &self.kind {
            Kind::Object(members) => members
                .iter()
                .rfind(|member| member.key == key)
                .map(|member| &member.value),
            _ => None,
        }
    }

    fn words(&self) -> Vec<String> {
        match &self.kind {
            Kind::Word(word) => vec![word.clone()],
            Kind::Array(items) => items
                .iter()
                .filter_map(|item| match &item.kind {
                    Kind::Word(word) => Some(word.clone()),
                    _ => None,
                })
                .collect(),
            Kind::Object(_) => Vec::new(),
        }
    }
}

/// A forgiving SPA-JSON reader: what it can't make sense of, it skips.
struct Parser<'a> {
    text: &'a str,
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    /// Skips whitespace (any Unicode whitespace), comments and the
    /// separators `,` `:` `=`.
    fn skip(&mut self) {
        while let Some(character) = self.text[self.at..].chars().next() {
            match character {
                '#' => {
                    // Up to the newline, which is a whole character.
                    while self.peek().is_some_and(|byte| byte != b'\n') {
                        self.at += 1;
                    }
                }
                ',' | ':' | '=' => self.at += 1,
                _ if character.is_whitespace() => self.at += character.len_utf8(),
                _ => return,
            }
        }
    }

    /// An object's members, up to `close` or the end.
    fn members(&mut self, close: Option<u8>) -> Vec<Member> {
        let mut members = Vec::new();
        loop {
            self.skip();
            match self.peek() {
                None => return members,
                Some(byte) if Some(byte) == close => {
                    self.at += 1;
                    return members;
                }
                Some(b'}' | b']') => {
                    // Unbalanced: skip it.
                    self.at += 1;
                }
                Some(_) => {
                    let start = self.at;
                    let key = self.word();
                    self.skip();
                    if self.peek().is_none() {
                        return members;
                    }
                    let value = self.value();
                    members.push(Member { key, start, value });
                }
            }
        }
    }

    fn value(&mut self) -> Node {
        match self.peek() {
            Some(b'{') => {
                self.at += 1;
                let members = self.members(Some(b'}'));
                Node {
                    kind: Kind::Object(members),
                    end: self.at,
                }
            }
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.skip();
                    match self.peek() {
                        None => break,
                        Some(b']') => {
                            self.at += 1;
                            break;
                        }
                        Some(b'}') => self.at += 1,
                        Some(_) => items.push(self.value()),
                    }
                }
                Node {
                    kind: Kind::Array(items),
                    end: self.at,
                }
            }
            _ => {
                let word = self.word();
                Node {
                    kind: Kind::Word(word),
                    end: self.at,
                }
            }
        }
    }

    /// A quoted string or a bare word.
    fn word(&mut self) -> String {
        let mut word = String::new();
        if self.peek() == Some(b'"') {
            self.at += 1;
            let mut chars = self.text[self.at..].char_indices();
            while let Some((offset, character)) = chars.next() {
                match character {
                    '"' => {
                        self.at += offset + 1;
                        return word;
                    }
                    '\\' => {
                        if let Some((_, escaped)) = chars.next() {
                            word.push(escaped);
                        }
                    }
                    other => word.push(other),
                }
            }
            self.at = self.text.len();
            return word;
        }
        let rest = &self.text[self.at..];
        let length = rest
            .find(|character: char| character.is_whitespace() || "{}[],:=#\"".contains(character))
            .unwrap_or(rest.len());
        word.push_str(&rest[..length]);
        // Always moves on, by whole characters.
        self.at += if length == 0 {
            rest.chars().next().map_or(0, char::len_utf8)
        } else {
            length
        };
        word
    }
}
