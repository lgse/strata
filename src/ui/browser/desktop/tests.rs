// SPDX-License-Identifier: MIT

use super::*;
use crate::model::{EntryKind, Location, MetadataValue};
use std::path::Path;

fn localsend_entry(location: Location, name: &str) -> FileEntry {
    FileEntry {
        location,
        native_name: name.into(),
        thumbnail_path: None,
        display_name: name.to_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[test]
fn localsend_targets_collects_every_local_selection() {
    let entries = vec![
        localsend_entry(Location::local("/tmp/report.pdf"), "report.pdf"),
        localsend_entry(Location::local("/tmp/photos"), "photos"),
    ];
    assert_eq!(
        localsend_send_targets(&entries),
        Some(vec![
            PathBuf::from("/tmp/report.pdf"),
            PathBuf::from("/tmp/photos"),
        ])
    );
}

#[test]
fn localsend_targets_rejects_empty_and_non_local_selections() {
    assert_eq!(localsend_send_targets(&[]), None);
    let entries = vec![
        localsend_entry(Location::local("/tmp/report.pdf"), "report.pdf"),
        localsend_entry(Location::uri("trash:///"), "report.pdf"),
    ];
    assert_eq!(localsend_send_targets(&entries), None);
    let entries = vec![localsend_entry(
        Location::uri("sftp://host/share/report.pdf"),
        "report.pdf",
    )];
    assert_eq!(localsend_send_targets(&entries), None);
}

#[test]
fn localsend_argv_passes_one_file_flag_per_target() {
    let cli = Path::new("/usr/bin/localsend-cli");
    let targets = vec![
        PathBuf::from("/tmp/report.pdf"),
        PathBuf::from("/tmp/my photos"),
    ];
    let argv: Vec<String> = localsend_send_argv(cli, &targets)
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        argv,
        vec![
            "/usr/bin/localsend-cli",
            "-f",
            "/tmp/report.pdf",
            "-f",
            "/tmp/my photos",
        ]
    );
}
