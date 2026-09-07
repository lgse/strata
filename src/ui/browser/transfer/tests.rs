// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::model::{FileEntry, Location};
use crate::services::{DropCommit, TransferKind};
use std::path::Path;

#[test]
fn drop_confirmation_presents_copy_move_and_cancel() {
    let source = include_str!("../transfer.rs");
    assert!(source.contains("\"Copy or move?\""));
    assert!(source.contains("\"Copy\""));
    assert!(source.contains("with_label(\"Move\")"));
    assert!(source.contains("layout.cancel"));
    assert!(source.contains("confirm_cross_volume_drop"));
    assert!(source.contains("start_transfer"));
}

#[test]
fn commit_file_drop_routes_copy_move_ask_and_forbidden() {
    let source = include_str!("../transfer.rs");
    let commit_fn = {
        let start = source
            .find("fn commit_file_drop")
            .expect("commit_file_drop");
        &source[start..]
    };
    assert!(commit_fn.contains("DropCommit::Copy => self.start_transfer"));
    assert!(commit_fn.contains("DropCommit::Move => self.start_transfer"));
    assert!(commit_fn.contains("confirm_cross_volume_drop"));
    assert!(commit_fn.contains("DropCommit::Forbidden"));
    assert_eq!(DropCommit::Copy.transfer_kind(), TransferKind::Copy);
    assert_eq!(DropCommit::Move.transfer_kind(), TransferKind::Move);
    assert_eq!(
        DropCommit::Ask {
            default: TransferKind::Copy,
        }
        .transfer_kind(),
        TransferKind::Copy
    );
}

#[test]
fn duplicate_transfer_uses_the_selected_entries_parent() {
    let entry = |path: &str| FileEntry {
        location: Location::local(path),
        native_name: Path::new(path).file_name().unwrap_or_default().to_owned(),
        thumbnail_path: None,
        display_name: path.to_owned(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        mode: crate::model::MetadataValue::Unknown,
        is_hidden: false,
    };
    let first = entry("/fixture/selected/first.txt");
    let second = entry("/fixture/selected/second.txt");

    assert_eq!(
        duplicate_transfer(&[first.clone(), second.clone()]),
        Some((
            Location::local("/fixture/selected"),
            vec![first.location, second.location]
        ))
    );
    assert_eq!(
        duplicate_transfer(&[entry("/fixture/one.txt"), entry("/other/two.txt")]),
        None
    );
    assert_eq!(duplicate_transfer(&[]), None);
    for uri in ["trash:///file.txt", "trash:///folder/file.txt"] {
        let trashed = FileEntry {
            location: Location::uri(uri),
            ..entry("file.txt")
        };
        assert_eq!(duplicate_transfer(&[trashed]), None);
    }
}

#[test]
fn transfer_collisions_detect_existing_destination_items() -> Result<(), Box<dyn std::error::Error>>
{
    let root = std::env::temp_dir().join(format!("strata-collision-test-{}", std::process::id()));
    let _ignored = std::fs::remove_dir_all(&root);
    let source_dir = root.join("source");
    let destination = root.join("destination");
    std::fs::create_dir_all(&source_dir)?;
    std::fs::create_dir_all(&destination)?;
    let source = source_dir.join("photo.jpg");
    std::fs::write(&source, b"new")?;

    assert!(!transfer_has_collision(
        &Location::local(&source),
        &Location::local(&destination)
    ));
    assert!(!transfer_has_collision(
        &Location::local(&source),
        &Location::local(&source_dir)
    ));
    std::fs::write(destination.join("photo.jpg"), b"old")?;
    assert!(transfer_has_collision(
        &Location::local(&source),
        &Location::local(&destination)
    ));

    std::fs::remove_dir_all(root)?;
    Ok(())
}
