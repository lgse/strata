// SPDX-License-Identifier: MIT

use super::*;

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn replace_text_swaps_matches_and_keeps_order() {
    let planned = plan_batch_rename(
        &names(&["photo 1.jpg", "photo 2.jpg", "notes.txt"]),
        &BatchRenameMode::ReplaceText {
            find: "photo".to_owned(),
            replace_with: "image".to_owned(),
        },
        "2026-09-15",
    );
    assert_eq!(planned, names(&["image 1.jpg", "image 2.jpg", "notes.txt"]));
}

#[test]
fn replace_text_with_empty_pattern_changes_nothing() {
    let input = names(&["photo.jpg"]);
    let planned = plan_batch_rename(
        &input,
        &BatchRenameMode::ReplaceText {
            find: String::new(),
            replace_with: "image".to_owned(),
        },
        "2026-09-15",
    );
    assert_eq!(planned, input);
}

#[test]
fn add_text_before_name_prepends() {
    let planned = plan_batch_rename(
        &names(&["photo.jpg", "notes"]),
        &BatchRenameMode::AddText {
            text: "trip-".to_owned(),
            before_name: true,
        },
        "2026-09-15",
    );
    assert_eq!(planned, names(&["trip-photo.jpg", "trip-notes"]));
}

#[test]
fn add_text_after_name_inserts_before_the_extension() {
    let planned = plan_batch_rename(
        &names(&["photo.jpg", "archive.tar.gz", "README", ".gitignore"]),
        &BatchRenameMode::AddText {
            text: "_final".to_owned(),
            before_name: false,
        },
        "2026-09-15",
    );
    assert_eq!(
        planned,
        names(&[
            "photo_final.jpg",
            "archive.tar_final.gz",
            "README_final",
            ".gitignore_final"
        ])
    );
}

#[test]
fn format_counter_is_zero_padded_and_keeps_the_extension() {
    let planned = plan_batch_rename(
        &names(&["a.jpg", "b.png", "c"]),
        &BatchRenameMode::Format {
            custom_name: "trip".to_owned(),
            style: FormatStyle::Counter,
            start_number: 7,
            before_name: false,
        },
        "2026-09-15",
    );
    assert_eq!(
        planned,
        names(&["trip 00007.jpg", "trip 00008.png", "trip 00009"])
    );
}

#[test]
fn format_index_numbers_from_the_start_value() {
    let planned = plan_batch_rename(
        &names(&["a.jpg", "b.jpg"]),
        &BatchRenameMode::Format {
            custom_name: "trip".to_owned(),
            style: FormatStyle::Index,
            start_number: 99,
            before_name: false,
        },
        "2026-09-15",
    );
    assert_eq!(planned, names(&["trip 99.jpg", "trip 100.jpg"]));
}

#[test]
fn format_before_name_puts_the_number_first() {
    let planned = plan_batch_rename(
        &names(&["a.jpg", "b.jpg"]),
        &BatchRenameMode::Format {
            custom_name: "trip".to_owned(),
            style: FormatStyle::Index,
            start_number: 1,
            before_name: true,
        },
        "2026-09-15",
    );
    assert_eq!(planned, names(&["1 trip.jpg", "2 trip.jpg"]));
}

#[test]
fn format_date_stamps_every_item_with_the_given_timestamp() {
    let planned = plan_batch_rename(
        &names(&["a.jpg", "b.png", "c.jpg"]),
        &BatchRenameMode::Format {
            custom_name: "trip".to_owned(),
            style: FormatStyle::Date,
            start_number: 1,
            before_name: false,
        },
        "2026-09-15 at 10.30.00 AM",
    );
    // Stamps are identical for the whole batch, so every item also carries
    // its position number to stay unique.
    assert_eq!(
        planned,
        names(&[
            "trip 2026-09-15 at 10.30.00 AM 1.jpg",
            "trip 2026-09-15 at 10.30.00 AM 2.png",
            "trip 2026-09-15 at 10.30.00 AM 3.jpg"
        ])
    );
}

#[test]
fn format_with_empty_name_changes_nothing() {
    let input = names(&["a.jpg"]);
    let planned = plan_batch_rename(
        &input,
        &BatchRenameMode::Format {
            custom_name: String::new(),
            style: FormatStyle::Counter,
            start_number: 1,
            before_name: false,
        },
        "2026-09-15",
    );
    assert_eq!(planned, input);
}
