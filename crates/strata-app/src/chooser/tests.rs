// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};

use super::{
    geometry::{FALLBACK_CHOOSER_HEIGHT, FALLBACK_CHOOSER_WIDTH, chooser_initial_dimensions},
    options::{ChoiceControl, build_choices, normalize_portal_filters},
    source::{CompiledFilter, chooser_entry_allowed, glob_matches},
};
use crate::portal::{FilterRule, PortalChoice, PortalFileFilter};

fn file_entry(path: &str, kind: EntryKind) -> FileEntry {
    let pb = PathBuf::from(path);
    let name = pb.file_name().unwrap_or_default().to_os_string();
    FileEntry {
        location: Location::local(&pb),
        thumbnail_path: None,
        native_name: name.clone(),
        display_name: name.to_string_lossy().into_owned(),
        kind,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
        recent_uri: None,
    }
}

#[test]
fn glob_and_mime_filters_match_files_and_keep_directories() {
    assert!(glob_matches("*.rs", "main.rs"));
    assert!(!glob_matches("*.rs", "main.txt"));
    assert!(glob_matches("report-[0-9]?.csv", "report-1a.csv"));
    assert!(!glob_matches("report-[!0-9].csv", "report-3.csv"));

    let png_filter = CompiledFilter::new(PortalFileFilter {
        label: "PNG images".into(),
        rules: vec![FilterRule::MimeType("image/png".into())],
    });
    let png = file_entry("/tmp/photo.png", EntryKind::File);
    let txt = file_entry("/tmp/readme.txt", EntryKind::File);
    let dir = file_entry("/tmp/pictures", EntryKind::Directory);

    assert!(chooser_entry_allowed(Some(&png_filter), false, None, &png));
    assert!(!chooser_entry_allowed(Some(&png_filter), false, None, &txt));
    assert!(chooser_entry_allowed(Some(&png_filter), false, None, &dir));
    assert!(!chooser_entry_allowed(None, true, None, &png));
    assert!(chooser_entry_allowed(
        None,
        false,
        Some(PathBuf::from("/media/usb").as_path()),
        &file_entry("/media/usb/photo.png", EntryKind::File),
    ));
    assert!(!chooser_entry_allowed(
        None,
        false,
        Some(PathBuf::from("/media/usb").as_path()),
        &png,
    ));
}

#[test]
fn portal_filters_and_choices_normalize_initial_selections() {
    let text = PortalFileFilter {
        label: "Text".into(),
        rules: vec![FilterRule::Pattern("*.txt".into())],
    };
    let extra = PortalFileFilter {
        label: "Images".into(),
        rules: vec![FilterRule::MimeType("image/*".into())],
    };
    let (compiled, selected) = normalize_portal_filters(&[text], Some(&extra));
    assert_eq!(compiled.len(), 2);
    assert_eq!(selected, Some(1));

    let choices = build_choices(&[
        PortalChoice {
            id: "ro".into(),
            label: "Read only".into(),
            pairs: Vec::new(),
            initial_selection: "true".into(),
        },
        PortalChoice {
            id: "enc".into(),
            label: "Encoding".into(),
            pairs: vec![
                ("utf8".into(), "UTF-8".into()),
                ("latin1".into(), "ISO-8859-1".into()),
            ],
            initial_selection: "latin1".into(),
        },
    ]);
    assert_eq!(
        choices.iter().map(ChoiceControl::value).collect::<Vec<_>>(),
        vec![
            ("ro".to_owned(), "true".to_owned()),
            ("enc".to_owned(), "latin1".to_owned()),
        ]
    );
}

#[test]
fn chooser_dimensions_clamp_to_monitor_and_parent_bounds() {
    assert_eq!(
        chooser_initial_dimensions(None, None),
        (FALLBACK_CHOOSER_WIDTH, FALLBACK_CHOOSER_HEIGHT)
    );
    assert_eq!(
        chooser_initial_dimensions(Some((1920, 1080)), None),
        (1000, 680)
    );
    assert_eq!(
        chooser_initial_dimensions(Some((1920, 1080)), Some((800, 600))),
        (640, 468)
    );
}
