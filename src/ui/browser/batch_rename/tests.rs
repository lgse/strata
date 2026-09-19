// SPDX-License-Identifier: MIT

use super::*;
use crate::model::{EntryKind, Location, MetadataValue};
use std::ffi::OsString;

fn test_entry(name: &str) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/test/{name}")),
        thumbnail_path: None,
        native_name: OsString::from(name),
        display_name: name.to_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        is_hidden: false,
    }
}

fn replace_input<'a>(find: &'a str, replace_with: &'a str) -> RenameDialogInput<'a> {
    RenameDialogInput {
        mode: RenameDialogMode::Replace,
        find,
        replace_with,
        text: "",
        before_name: false,
        base: "",
        style: FormatStyle::Counter,
        start: "1",
        format_before: false,
    }
}

fn add_input<'a>(text: &'a str, before_name: bool) -> RenameDialogInput<'a> {
    RenameDialogInput {
        mode: RenameDialogMode::Add,
        find: "",
        replace_with: "",
        text,
        before_name,
        base: "",
        style: FormatStyle::Counter,
        start: "1",
        format_before: false,
    }
}

fn format_input<'a>(
    base: &'a str,
    style: FormatStyle,
    start: &'a str,
    format_before: bool,
) -> RenameDialogInput<'a> {
    RenameDialogInput {
        mode: RenameDialogMode::Format,
        find: "",
        replace_with: "",
        text: "",
        before_name: false,
        base,
        style,
        start,
        format_before,
    }
}

#[test]
fn plan_preview_name_picks_first_changed_entry_when_earlier_entries_unaffected() {
    let entries = vec![
        test_entry("notes.txt"),
        test_entry("photo_1.jpg"),
        test_entry("photo_2.jpg"),
    ];
    let input = replace_input("photo", "image");
    let result = plan_preview_name(&entries, &input);
    assert_eq!(
        result,
        Ok(("photo_1.jpg".to_owned(), "image_1.jpg".to_owned()))
    );
}

#[test]
fn plan_preview_name_falls_back_to_first_entry_when_no_entries_match() {
    let entries = vec![test_entry("a.txt"), test_entry("b.txt")];
    let input = replace_input("missing", "replacement");
    let result = plan_preview_name(&entries, &input);
    assert_eq!(result, Ok(("a.txt".to_owned(), "a.txt".to_owned())));
}

#[test]
fn plan_preview_name_handles_empty_entries() {
    let entries: Vec<FileEntry> = Vec::new();
    let input = replace_input("a", "b");
    let result = plan_preview_name(&entries, &input);
    assert_eq!(result, Ok((String::new(), String::new())));
}

#[test]
fn dialog_rename_mode_validates_required_fields() {
    assert_eq!(
        dialog_rename_mode(&replace_input("", "bar")),
        Err(RenameDialogError::MissingFind)
    );
    assert_eq!(
        dialog_rename_mode(&add_input("", false)),
        Err(RenameDialogError::MissingText)
    );
    assert_eq!(
        dialog_rename_mode(&format_input("", FormatStyle::Counter, "1", false)),
        Err(RenameDialogError::MissingBase)
    );
    assert_eq!(
        dialog_rename_mode(&format_input(
            "name",
            FormatStyle::Counter,
            "not-a-number",
            false
        )),
        Err(RenameDialogError::BadStart)
    );
    assert_eq!(
        dialog_rename_mode(&format_input("name", FormatStyle::Index, "", false)),
        Err(RenameDialogError::BadStart)
    );
}

#[test]
fn dialog_rename_mode_allows_start_number_zero() {
    let mode = dialog_rename_mode(&format_input("frame", FormatStyle::Counter, "0", false));
    assert_eq!(
        mode,
        Ok(BatchRenameMode::Format {
            custom_name: "frame".to_owned(),
            style: FormatStyle::Counter,
            start_number: 0,
            before_name: false,
        })
    );

    let mode_index = dialog_rename_mode(&format_input("frame", FormatStyle::Index, "0", false));
    assert_eq!(
        mode_index,
        Ok(BatchRenameMode::Format {
            custom_name: "frame".to_owned(),
            style: FormatStyle::Index,
            start_number: 0,
            before_name: false,
        })
    );
}

#[test]
fn dialog_rename_mode_date_style_defaults_to_one_regardless_of_start_text() {
    let mode = dialog_rename_mode(&format_input("doc", FormatStyle::Date, "ignored", false));
    assert_eq!(
        mode,
        Ok(BatchRenameMode::Format {
            custom_name: "doc".to_owned(),
            style: FormatStyle::Date,
            start_number: 1,
            before_name: false,
        })
    );
}
