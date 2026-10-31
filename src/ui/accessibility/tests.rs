// SPDX-License-Identifier: MIT

use std::ffi::OsString;

use super::*;
use crate::model::{Location, MetadataValue};

fn entry(kind: EntryKind) -> FileEntry {
    FileEntry {
        location: Location::local("/tmp/entry"),
        native_name: OsString::from("entry"),
        thumbnail_path: None,
        display_name: "entry".to_owned(),
        kind,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        recent_uri: None,
    }
}

#[test]
fn every_entry_kind_has_a_spoken_name() {
    let kinds = [
        EntryKind::Directory,
        EntryKind::DirectorySymbolicLink,
        EntryKind::File,
        EntryKind::FileSymbolicLink,
        EntryKind::SymbolicLink,
        EntryKind::Other,
    ];
    for kind in kinds {
        assert!(!entry_kind_name(&entry(kind)).is_empty());
    }
}

#[test]
fn directories_and_files_are_distinguishable() {
    assert_eq!(entry_kind_name(&entry(EntryKind::Directory)), "Folder");
    assert_eq!(entry_kind_name(&entry(EntryKind::File)), "File");
    assert_ne!(
        entry_kind_name(&entry(EntryKind::SymbolicLink)),
        entry_kind_name(&entry(EntryKind::FileSymbolicLink))
    );
}

#[test]
fn each_presentation_has_a_distinct_view_name() {
    let names = [
        view_name(BrowserMode::Columns),
        view_name(BrowserMode::Icons),
        view_name(BrowserMode::List),
    ];
    assert_eq!(names.len(), std::collections::BTreeSet::from(names).len());
}

#[cfg(feature = "qa")]
#[test]
fn qa_reads_back_what_the_naming_helpers_set() {
    crate::test_support::gtk_test(
        "ui::accessibility::tests::qa_reads_back_what_the_naming_helpers_set",
        || {
            let pane = pane_box();
            describe_pane(&pane, "documents", BrowserMode::Icons);
            assert_eq!(
                qa::naming(&pane),
                Some(qa::Naming {
                    label: Some("documents".to_owned()),
                    description: Some(view_name(BrowserMode::Icons)),
                    mark: Some(qa::Mark::Pane("Icons")),
                    checked: None,
                    pressed: None,
                    selected: None,
                    busy: None,
                })
            );

            let button = gtk::Button::new();
            set_label(&button, "Keep Both");
            set_description(&button, Some("Copies with a new name"));
            set_description(&button, None);
            let naming = qa::naming(&button).expect("the button was named");
            assert_eq!(naming.label.as_deref(), Some("Keep Both"));
            assert_eq!(naming.description, None);

            assert_eq!(qa::naming(&gtk::Button::new()), None);

            let option = gtk::ToggleButton::new();
            set_checked(&option, true);
            set_pressed(&option, false);
            set_selected(&option, true);
            let title = gtk::Label::new(Some("List view"));
            set_labelled_by(&option, &[title.upcast_ref(), option.upcast_ref()]);
            assert_eq!(
                qa::naming(&option).and_then(|naming| naming.checked),
                Some(true)
            );
            assert_eq!(
                qa::naming(&option).and_then(|naming| naming.selected),
                Some(true)
            );
            assert_eq!(
                qa::labelled_by(&option),
                [title.upcast::<gtk::Widget>(), option.upcast()]
            );

            let picture = gtk::Picture::new();
            set_busy(&picture, true);
            set_busy(&picture, false);
            assert_eq!(
                qa::naming(&picture).and_then(|naming| naming.busy),
                Some(false)
            );
        },
    );
}
