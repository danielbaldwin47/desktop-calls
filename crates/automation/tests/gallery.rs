//! `automation screenshot`: the gallery of every surface in every state,
//! light and dark.

use std::process::Command;

#[test]
fn the_gallery_shows_every_state_in_light_and_dark() {
    let output = tempfile::tempdir().expect("create a gallery directory");

    let rendered = Command::new(env!("CARGO_BIN_EXE_automation"))
        .arg("screenshot")
        .arg("--output")
        .arg(output.path())
        .output()
        .expect("run automation screenshot");

    assert!(rendered.status.success(), "screenshot failed: {rendered:?}");
    let index = std::fs::read_to_string(output.path().join("index.html"))
        .expect("the gallery has an index");
    let states = [
        "bare-output",
        "incoming-call-banner",
        "call-window-live",
        "call-window-ended",
    ];
    let files = states
        .iter()
        .flat_map(|state| [format!("{state}-light.png"), format!("{state}-dark.png")]);
    for file in files {
        let file = file.as_str();
        assert!(index.contains(file), "the index shows {file}: {index}");
        let png = std::fs::read(output.path().join(file)).expect("the screenshot exists");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "{file} is a PNG");
        // Width and height from the PNG header: the 3840 by 2160 output.
        assert_eq!(&png[16..24], &[0, 0, 0x0f, 0, 0, 0, 0x08, 0x70], "{file}");
    }
}
