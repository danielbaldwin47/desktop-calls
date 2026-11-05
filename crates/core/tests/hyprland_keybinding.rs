//! Whether the Hyprland config leaves the Desktop Calls keys free, from
//! what the shell read of Omarchy's `bindings.lua` and the files it
//! requires (ticket #22).

use desktop_calls_core::keybinding::required;
use desktop_calls_core::{ConfigFile, HyprlandConfig, KeybindingCheck, RequiredFile};

const FILE: &str = "/home/user/.config/hypr/bindings.lua";
const COMMAND: &str = "/usr/bin/desktop-calls";
const KEYS: &str = "SUPER + ALT + C";
const LINE: &str = r#"o.bind("SUPER + ALT + C", "Desktop Calls", "/usr/bin/desktop-calls")"#;

fn config(text: &str) -> HyprlandConfig {
    HyprlandConfig {
        command: COMMAND.to_owned(),
        bindings: Some(ConfigFile {
            path: FILE.to_owned(),
            text: Some(text.to_owned()),
            required: Vec::new(),
        }),
    }
}

fn check(text: &str) -> KeybindingCheck {
    KeybindingCheck::of(&config(text))
}

fn taken_in(file: &str) -> KeybindingCheck {
    KeybindingCheck::Taken {
        keys: KEYS.to_owned(),
        file: file.to_owned(),
        command: COMMAND.to_owned(),
    }
}

fn added() -> KeybindingCheck {
    KeybindingCheck::Added {
        keys: KEYS.to_owned(),
        file: FILE.to_owned(),
    }
}

#[test]
fn keys_in_another_order_or_case_are_the_same_keys() {
    for keys in [
        "ALT + SUPER + C",
        "super alt c",
        "SUPER+ALT+c",
        "MOD4 + MOD1 + C",
    ] {
        let text = format!("o.bind(\"{keys}\", \"Calculator\", \"gnome-calculator\")\n");
        assert_eq!(check(&text), taken_in(FILE), "{keys}");
    }
}

#[test]
fn other_keys_leave_them_free_and_the_line_goes_at_the_end() {
    let text = "o.bind(\"SUPER + ALT + D\", \"Docs\", \"xdg-open ~/Documents\")\n";
    assert_eq!(
        check(text),
        KeybindingCheck::Available {
            keys: KEYS.to_owned(),
            file: FILE.to_owned(),
            line: LINE.to_owned(),
            before_return: false,
            seen: text.to_owned(),
            added: format!("{text}{LINE}\n"),
        }
    );
}

#[test]
fn a_commented_out_binding_does_not_take_the_keys() {
    let text = "-- o.bind(\"SUPER + ALT + C\", \"Calculator\", \"gnome-calculator\")\n";
    assert!(matches!(check(text), KeybindingCheck::Available { .. }));
}

#[test]
fn keys_unbound_after_a_binding_are_free_again() {
    let text = "o.bind(\"SUPER + ALT + C\", \"Calculator\", \"gnome-calculator\")\nhl.unbind(\"SUPER + ALT + C\")\n";
    assert!(matches!(check(text), KeybindingCheck::Available { .. }));
}

#[test]
fn an_earlier_desktop_calls_line_from_another_path_counts_as_added() {
    let text = "o.bind(\"SUPER + ALT + C\", \"Phone\", \"/home/user/.cargo/bin/desktop-calls\")\n";
    assert_eq!(check(text), added());
}

#[test]
fn the_line_this_offers_is_found_added_once_written() {
    let KeybindingCheck::Available { added: written, .. } = check("-- mine\n") else {
        panic!("the line isn't offered");
    };
    assert_eq!(check(&written), added());
}

#[test]
fn a_file_without_a_final_newline_gets_one_before_the_line() {
    let KeybindingCheck::Available { added, .. } = check("-- mine") else {
        panic!("the line isn't offered");
    };
    assert_eq!(added, format!("-- mine\n{LINE}\n"));
}

#[test]
fn a_trailing_return_keeps_the_line_before_it() {
    let text = "local M = {}\nreturn M\n";
    let KeybindingCheck::Available {
        added,
        before_return,
        ..
    } = check(text)
    else {
        panic!("the line isn't offered");
    };
    assert!(before_return);
    assert_eq!(added, format!("local M = {{}}\n{LINE}\nreturn M\n"));
}

#[test]
fn keys_bound_in_a_required_file_are_taken_there_unless_unbound_after() {
    let more = "/home/user/.config/hypr/more.lua";
    let with = |text: &str| HyprlandConfig {
        command: COMMAND.to_owned(),
        bindings: Some(ConfigFile {
            path: FILE.to_owned(),
            text: Some(text.to_owned()),
            required: vec![RequiredFile {
                name: "hypr.more".to_owned(),
                path: more.to_owned(),
                text: "o.bind(\"SUPER + ALT + C\", \"Calculator\", \"gnome-calculator\")\n"
                    .to_owned(),
            }],
        }),
    };
    assert_eq!(
        KeybindingCheck::of(&with("require(\"hypr.more\")\n")),
        taken_in(more)
    );
    assert!(matches!(
        KeybindingCheck::of(&with(
            "require(\"hypr.more\")\nhl.unbind(\"SUPER + ALT + C\")\n"
        )),
        KeybindingCheck::Available { .. }
    ));
}

#[test]
fn required_files_are_named_by_require_pcall_and_dofile() {
    let text = "require(\"hypr.a\")\nlocal ok = pcall(require, 'hypr.b')\ndofile(\"/etc/c.lua\")\n-- require(\"hypr.d\")\n";
    assert_eq!(required(text), ["hypr.a", "hypr.b", "/etc/c.lua"]);
}

#[test]
fn a_bindings_file_that_cant_be_read_is_named_and_no_config_says_what_to_bind() {
    let unreadable = HyprlandConfig {
        command: COMMAND.to_owned(),
        bindings: Some(ConfigFile {
            path: FILE.to_owned(),
            text: None,
            required: Vec::new(),
        }),
    };
    assert_eq!(
        KeybindingCheck::of(&unreadable),
        KeybindingCheck::Unreadable {
            file: FILE.to_owned(),
            command: COMMAND.to_owned()
        }
    );
    assert_eq!(
        KeybindingCheck::of(&HyprlandConfig {
            command: COMMAND.to_owned(),
            bindings: None
        }),
        KeybindingCheck::NoConfig {
            command: COMMAND.to_owned()
        }
    );
}
