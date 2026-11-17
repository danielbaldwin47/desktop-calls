//! The Ringer Window, at the Call state machine's public interface (issue
//! #432): opened from the Indicator's Ringer… entry, it shows the Ring
//! Sound and volume set. Choosing a sound, or letting go of the volume,
//! saves at once and plays the sound once, which Stop or Escape stops; a
//! Call that starts ringing stops it too, so nothing plays over the Ringer.

#![expect(
    clippy::expect_used,
    reason = "test code: a failed helper step fails the test with its message"
)]

use desktop_calls_core::{
    CallId, CallState, Caller, Effect, IndicatorEntry, Input, Phone, RingSound, RingerSettings,
    RingerWindowView, State, Timestamp, messages, step,
};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
const FILE: &str = "/music/my ring.ogg";

fn run(state: State, inputs: impl IntoIterator<Item = Input>) -> (State, Vec<Effect>) {
    let mut state = state;
    let mut effects = Vec::new();
    for input in inputs {
        let (next, mut more) = step(state, input);
        state = next;
        effects.append(&mut more);
    }
    (state, effects)
}

fn connected() -> State {
    let phone = Phone {
        address: ADDRESS.to_owned(),
        name: "My Pixel".to_owned(),
    };
    run(
        State::default(),
        [
            Input::PhonePaired(phone),
            Input::PhoneConnected {
                address: ADDRESS.to_owned(),
            },
        ],
    )
    .0
}

/// Opens the window from the Indicator's menu.
fn open(state: State) -> State {
    let entry = state
        .indicator(&state.surfaces())
        .menu
        .into_iter()
        .find(|entry| *entry == IndicatorEntry::Ringer)
        .expect("the menu has Ringer…");
    run(state, [entry.intent().expect("it opens the window")]).0
}

fn window(state: &State) -> RingerWindowView {
    state
        .surfaces()
        .ringer_window
        .expect("the Ringer Window is open")
}

fn kept(effects: &[Effect]) -> Vec<RingerSettings> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::KeepRinger(settings) => Some(settings.clone()),
            _ => None,
        })
        .collect()
}

fn previews(effects: &[Effect]) -> Vec<RingerSettings> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::PreviewRingSound(settings) => Some(settings.clone()),
            _ => None,
        })
        .collect()
}

fn ring(state: State) -> (State, Vec<Effect>) {
    run(
        state,
        [Input::CallAdded {
            call: CallId(1),
            state: CallState::Ringing,
            caller: Caller::Number("+447700900123".to_owned()),
            at: Timestamp(1_000),
        }],
    )
}

#[test]
fn the_menu_entry_opens_it_on_the_settings_kept_and_names_it_ringer() {
    let state = connected();
    let view = state.indicator(&state.surfaces());
    let entry = view
        .menu
        .iter()
        .find(|entry| **entry == IndicatorEntry::Ringer)
        .expect("the menu has Ringer…");
    assert_eq!(messages::indicator_entry(entry, &view), "Ringer…");
    let at = |wanted: fn(&IndicatorEntry) -> bool| view.menu.iter().position(wanted);
    assert_eq!(
        at(|entry| matches!(entry, IndicatorEntry::QuietHours(_))).map(|at| at + 1),
        at(|entry| *entry == IndicatorEntry::Ringer),
        "just under Quiet hours…"
    );

    let kept_settings = RingerSettings::default()
        .with_sound(RingSound::Oberon)
        .with_volume(80);
    let (state, effects) = run(state, [Input::RingerKept(kept_settings.clone())]);
    assert_eq!(kept(&effects), Vec::new(), "nothing to keep again");
    let state = open(state);
    let view = window(&state);
    assert_eq!(view.settings, kept_settings);
    assert!(!view.previewing);
    assert!(!view.unplayable);
}

#[test]
fn choosing_a_sound_saves_it_and_plays_it_once() {
    let state = open(connected());
    let (state, effects) = run(state, [Input::ChooseRingSound(RingSound::Bells)]);
    let bells = RingerSettings::default().with_sound(RingSound::Bells);
    assert_eq!(kept(&effects), vec![bells.clone()]);
    assert_eq!(previews(&effects), vec![bells.clone()]);
    assert!(window(&state).previewing);
    assert_eq!(state.ringer(), &bells);

    // Choosing it again plays it again, and saves nothing new.
    let (state, effects) = run(state, [Input::ChooseRingSound(RingSound::Bells)]);
    assert_eq!(kept(&effects), Vec::new());
    assert_eq!(previews(&effects), vec![bells]);

    // Once it has played, the window says so.
    let (state, _) = run(state, [Input::PreviewEnded]);
    assert!(!window(&state).previewing);
}

#[test]
fn letting_go_of_the_volume_saves_it_and_plays_the_sound_at_it() {
    let state = open(connected());
    let (state, effects) = run(state, [Input::SetRingerVolume(35)]);
    let quieter = RingerSettings::default().with_volume(35);
    assert_eq!(kept(&effects), vec![quieter.clone()]);
    assert_eq!(previews(&effects), vec![quieter]);
    assert_eq!(window(&state).settings.volume(), 35);
    assert_eq!(messages::ringer_window(&window(&state)).volume, "35 %");

    // Never so low that a Call is silent.
    let (state, effects) = run(state, [Input::SetRingerVolume(0)]);
    assert_eq!(kept(&effects)[0].volume(), 10);
    assert_eq!(state.ringer().volume(), 10);
}

#[test]
fn stop_and_escape_stop_the_sound_and_escape_then_closes() {
    let state = open(connected());
    let (state, _) = run(state, [Input::ChooseRingSound(RingSound::Glass)]);
    let (state, effects) = run(state, [Input::StopPreview]);
    assert!(effects.contains(&Effect::StopPreview));
    assert!(!window(&state).previewing);

    // Play plays the sound chosen once more.
    let (state, effects) = run(state, [Input::PreviewRingSound]);
    assert_eq!(
        previews(&effects),
        vec![RingerSettings::default().with_sound(RingSound::Glass)]
    );
    assert_eq!(messages::ringer_window(&window(&state)).play, "Stop");
    assert_eq!(
        messages::ringer_window(&window(&state)).keys.last(),
        Some(&(["Esc"].as_slice(), "Stop"))
    );

    // Escape stops it and keeps the window open; a second closes it.
    let (state, effects) = run(state, [Input::RingerWindowEscape]);
    assert!(effects.contains(&Effect::StopPreview));
    assert!(!window(&state).previewing);
    assert_eq!(messages::ringer_window(&window(&state)).play, "Play");
    let (state, effects) = run(state, [Input::RingerWindowEscape]);
    assert!(!effects.contains(&Effect::StopPreview));
    assert!(state.surfaces().ringer_window.is_none());
}

#[test]
fn closing_the_window_stops_the_sound() {
    let state = open(connected());
    let (state, _) = run(state, [Input::ChooseRingSound(RingSound::Harp)]);
    let (state, effects) = run(state, [Input::CloseRingerWindow]);
    assert!(effects.contains(&Effect::StopPreview));
    assert!(state.surfaces().ringer_window.is_none());
    assert_eq!(
        state.ringer().sound(),
        RingSound::Harp,
        "closing loses nothing"
    );
}

#[test]
fn a_call_that_starts_ringing_stops_the_sound_and_nothing_plays_over_the_ringer() {
    let state = open(connected());
    let (state, _) = run(state, [Input::ChooseRingSound(RingSound::Purr)]);
    let (state, effects) = ring(state);
    assert!(effects.contains(&Effect::StartRinging));
    assert!(effects.contains(&Effect::StopPreview));
    assert!(!window(&state).previewing);

    // While it rings, choosing still saves, but plays nothing.
    let (state, effects) = run(state, [Input::ChooseRingSound(RingSound::Drops)]);
    assert_eq!(
        kept(&effects),
        vec![RingerSettings::default().with_sound(RingSound::Drops)]
    );
    assert_eq!(previews(&effects), Vec::new());
    let (_, effects) = run(state, [Input::PreviewRingSound]);
    assert_eq!(previews(&effects), Vec::new());
}

#[test]
fn a_file_chosen_rings_and_is_remembered_beside_the_other_sounds() {
    let state = open(connected());
    let (state, effects) = run(state, [Input::ChooseRingerFile(FILE.to_owned())]);
    let own = RingerSettings::default().with_file(FILE.to_owned());
    assert_eq!(own.sound(), RingSound::File);
    assert_eq!(kept(&effects), vec![own.clone()]);
    assert_eq!(previews(&effects), vec![own]);
    let text = messages::ringer_window(&window(&state));
    assert_eq!(text.file.as_deref(), Some("my ring.ogg"));

    // Another sound, then the file again, with no need to choose it anew.
    let (state, _) = run(state, [Input::ChooseRingSound(RingSound::Luna)]);
    assert_eq!(state.ringer().file(), Some(FILE));
    let (state, _) = run(state, [Input::ChooseRingSound(RingSound::File)]);
    assert_eq!(state.ringer().sound(), RingSound::File);
}

#[test]
fn the_file_as_a_sound_with_no_file_chosen_does_nothing() {
    let state = open(connected());
    let (state, effects) = run(state, [Input::ChooseRingSound(RingSound::File)]);
    assert_eq!(kept(&effects), Vec::new());
    assert_eq!(previews(&effects), Vec::new());
    assert_eq!(state.ringer().sound(), RingSound::Chime);
}

#[test]
fn a_file_that_cant_play_says_so_and_that_the_chime_rings_instead() {
    let state = open(connected());
    let (state, _) = run(state, [Input::ChooseRingerFile(FILE.to_owned())]);
    let (state, _) = run(
        state,
        [Input::RingSoundUnplayable {
            file: FILE.to_owned(),
        }],
    );
    assert!(window(&state).unplayable);
    let text = messages::ringer_window(&window(&state));
    assert_eq!(text.notice, Some("Can't play this file"));
    assert_eq!(text.notice_detail, Some("Chime rings instead"));

    // Another file, or another sound, and the notice goes.
    let (state, _) = run(state, [Input::ChooseRingSound(RingSound::Chime)]);
    assert!(!window(&state).unplayable);

    // A report about a file no longer chosen changes nothing.
    let (state, _) = run(
        state,
        [Input::RingSoundUnplayable {
            file: "/old.ogg".to_owned(),
        }],
    );
    assert!(!window(&state).unplayable);
}

#[test]
fn opening_it_closes_the_dialer_so_it_has_the_keyboard() {
    let (state, _) = run(
        connected(),
        [
            Input::OpenDialer {
                query: String::new(),
                select_call: None,
            },
            Input::OpenRingerWindow,
        ],
    );
    assert!(state.surfaces().dialer.is_none());
    assert!(state.surfaces().ringer_window.is_some());
}

#[test]
fn every_label_is_a_few_words() {
    let state = open(connected());
    let (state, _) = run(state, [Input::ChooseRingerFile(FILE.to_owned())]);
    let text = messages::ringer_window(&window(&state));
    let mut labels = vec![
        text.heading,
        text.built_in,
        text.android,
        text.own_file,
        text.choose_file,
        text.volume_heading,
        text.play,
    ];
    labels.extend(text.built_in_sounds.iter().map(|(_, name)| *name));
    labels.extend(text.android_sounds.iter().map(|(_, name)| *name));
    for label in labels {
        assert!(
            label.split_whitespace().count() <= 3 && !label.ends_with('.'),
            "{label:?} reads at a glance"
        );
    }
    assert_eq!(text.built_in_sounds[0], (RingSound::Chime, "Chime"));
}
