//! `automation scorecard`: every polish-scorecard item with a status and its
//! evidence.

use std::process::Command;

#[test]
fn the_scorecard_lists_all_eight_items_with_status_and_evidence() {
    let printed = Command::new(env!("CARGO_BIN_EXE_automation"))
        .arg("scorecard")
        .output()
        .expect("run automation scorecard");
    assert!(printed.status.success(), "scorecard failed: {printed:?}");
    let text = String::from_utf8(printed.stdout).expect("the scorecard is text");

    // The agreed items, read from the wiki page that records them.
    let wiki = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/wiki/polish-scorecard.md"
    ))
    .expect("read the polish scorecard");
    let items: Vec<&str> = wiki
        .lines()
        .filter_map(|line| {
            let (number, item) = line.split_once(". ")?;
            number.parse::<u8>().ok().map(|_| item)
        })
        .collect();
    assert_eq!(items.len(), 8, "the wiki lists eight items: {items:#?}");

    let header = text
        .lines()
        .find(|line| line.contains("Item"))
        .expect("a header row");
    assert!(
        header.contains("Status") && header.contains("Evidence"),
        "{text}"
    );
    for (index, item) in items.iter().enumerate() {
        let number = index + 1;
        let row = text
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("{number} ")))
            .unwrap_or_else(|| panic!("no row for item {number}:\n{text}"));
        assert!(
            row.contains(item),
            "row {number} should read {item:?}: {row}"
        );
        assert!(
            row.contains("FAIL"),
            "item {number} is not passing yet: {row}"
        );
        assert!(row.ends_with("no evidence yet"), "item {number}: {row}");
    }
    assert!(text.contains("0 of 8 items pass"), "{text}");
}
