use strata_kit::overrides::Overrides;

use super::reload::{Change, Debounce, SETTLE};
use super::{FILE, FILE_BROKEN};

#[test]
fn the_fake_files_apply_and_the_broken_one_reports_its_row() {
    assert!(Overrides::from_bytes(FILE.as_bytes()).errors().is_empty());
    assert_eq!(
        Overrides::from_bytes(FILE_BROKEN.as_bytes()).errors().len(),
        1
    );
}

// settings/themes: "Changes to the file less than 75 ms apart trigger one reload, 75 ms after
// the last."
#[test]
fn changes_less_than_the_settle_apart_read_once_after_the_last() {
    let mut d = Debounce::default();
    d.changed(1.0);
    d.changed(1.05);
    assert!(
        !d.take_due(1.0 + SETTLE),
        "the second change moved the read"
    );
    assert!(d.take_due(1.05 + SETTLE));
    assert!(!d.take_due(2.0), "one read per burst");

    // Changes further apart than the settle each read.
    d.changed(3.0);
    assert!(d.take_due(3.1));
    d.changed(3.2);
    assert!(d.take_due(3.3));
}

#[test]
fn a_read_changes_the_look_only_when_the_overrides_differ() {
    let file = Overrides::from_bytes(FILE.as_bytes());
    let none = Overrides::default();
    assert_eq!(
        Change::of(&file, true, &file.clone(), true),
        Change::Nothing
    );
    assert_eq!(
        Change::of(&none, false, &none, true),
        Change::Presence,
        "an empty file appears"
    );
    assert_eq!(Change::of(&none, true, &none, false), Change::Presence);
    assert_eq!(Change::of(&file, true, &none, false), Change::Look);
    let broken = Overrides::from_bytes(FILE_BROKEN.as_bytes());
    assert_eq!(Change::of(&file, true, &broken, true), Change::Look);
}
