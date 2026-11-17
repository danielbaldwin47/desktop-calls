//! The Ringer's settings as the config file keeps them (issue #432): one
//! `key = value` line each for the Ring Sound, the volume and the user's own
//! file, written by the Ringer Window and readable by hand. Whatever in the
//! file can't be read falls back to the default for that line alone, and is
//! named, so the shell can log it; a Call is never silent because of it.

use desktop_calls_core::{RingSound, RingerSettings, RingerSettingsProblem};

#[test]
fn the_default_is_the_chime_at_60_percent_with_no_file() {
    let settings = RingerSettings::default();
    assert_eq!(settings.sound(), RingSound::Chime);
    assert_eq!(settings.volume(), 60);
    assert_eq!(settings.file(), None);
}

#[test]
fn settings_written_read_back_the_same() {
    for settings in [
        RingerSettings::default(),
        RingerSettings::default().with_sound(RingSound::Luna),
        RingerSettings::default().with_volume(85),
        RingerSettings::default().with_file("/music/a ring.ogg".to_owned()),
    ] {
        let text = settings.to_string();
        assert_eq!(
            RingerSettings::parse(&text),
            (settings.clone(), Vec::new()),
            "{text}"
        );
    }
}

#[test]
fn the_file_reads_as_one_line_per_setting() {
    let settings = RingerSettings::default()
        .with_file("/music/a.ogg".to_owned())
        .with_volume(45);
    assert_eq!(
        settings.to_string(),
        "sound = file\nvolume = 45\nfile = /music/a.ogg\n"
    );
    assert_eq!(
        RingerSettings::default().to_string(),
        "sound = chime\nvolume = 60\n"
    );
}

#[test]
fn every_ring_sound_has_its_own_key_that_reads_back() {
    let mut keys: Vec<&str> = RingSound::ALL.iter().map(|sound| sound.key()).collect();
    for sound in RingSound::ALL {
        assert_eq!(RingSound::from_key(sound.key()), Some(sound));
    }
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), RingSound::ALL.len(), "keys are distinct");
}

#[test]
fn six_sounds_are_made_here_and_six_come_from_android() {
    assert_eq!(RingSound::BUILT_IN.len(), 6);
    assert_eq!(
        RingSound::BUILT_IN[0],
        RingSound::Chime,
        "the default first"
    );
    assert_eq!(RingSound::ANDROID.len(), 6);
}

#[test]
fn spaces_comments_and_blank_lines_are_fine() {
    let text = "# my ringer\n\n  sound=bells  \nvolume =70\n";
    assert_eq!(
        RingerSettings::parse(text),
        (
            RingerSettings::default()
                .with_sound(RingSound::Bells)
                .with_volume(70),
            Vec::new()
        )
    );
}

#[test]
fn a_line_that_cant_be_read_falls_back_for_that_line_alone_and_is_named() {
    let (settings, problems) =
        RingerSettings::parse("sound = foghorn\nvolume = 40\nloud = yes\nnonsense\n");
    assert_eq!(settings, RingerSettings::default().with_volume(40));
    assert_eq!(
        problems,
        vec![
            RingerSettingsProblem::UnknownSound("foghorn".to_owned()),
            RingerSettingsProblem::UnknownLine("loud = yes".to_owned()),
            RingerSettingsProblem::UnknownLine("nonsense".to_owned()),
        ]
    );

    let (settings, problems) = RingerSettings::parse("sound = glass\nvolume = very\n");
    assert_eq!(
        settings,
        RingerSettings::default().with_sound(RingSound::Glass)
    );
    assert_eq!(
        problems,
        vec![RingerSettingsProblem::UnreadableVolume("very".to_owned())]
    );
}

#[test]
fn the_volume_is_kept_between_10_and_100_percent_so_a_call_is_never_silent() {
    assert_eq!(RingerSettings::default().with_volume(0).volume(), 10);
    assert_eq!(RingerSettings::default().with_volume(250).volume(), 100);
    let (settings, problems) = RingerSettings::parse("volume = 0\n");
    assert_eq!(settings.volume(), 10);
    assert_eq!(problems, Vec::new());
}

#[test]
fn the_own_file_without_a_file_rings_the_chime() {
    let (settings, problems) = RingerSettings::parse("sound = file\n");
    assert_eq!(settings.sound(), RingSound::Chime);
    assert_eq!(problems, vec![RingerSettingsProblem::NoFile]);
}

#[test]
fn a_chosen_file_is_remembered_while_another_sound_rings() {
    let settings = RingerSettings::default()
        .with_file("/music/a.ogg".to_owned())
        .with_sound(RingSound::Harp);
    assert_eq!(settings.sound(), RingSound::Harp);
    assert_eq!(settings.file(), Some("/music/a.ogg"));
    let (read, problems) = RingerSettings::parse(&settings.to_string());
    assert_eq!((read, problems), (settings, Vec::new()));
}
