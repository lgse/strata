// SPDX-License-Identifier: MIT

//! Which files a player counts: a name joins only once its probe or its bytes back it, the
//! shown file always counts, unread files are read nearest first, and an answer holds only
//! while the file is the one that was read.

use std::path::PathBuf;

use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};

use super::{Checks, Family, Fingerprint, counted, entry_family};

fn file(name: &str) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/media/{name}")),
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.to_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
    }
}

fn with(
    name: &str,
    duration: MetadataValue<u64>,
    dimensions: MetadataValue<(u32, u32)>,
) -> FileEntry {
    FileEntry {
        duration_seconds: duration,
        image_dimensions: dimensions,
        ..file(name)
    }
}

fn names(files: &[FileEntry]) -> Vec<&str> {
    files
        .iter()
        .map(|file| file.display_name.as_str())
        .collect()
}

fn paths(unread: &[(PathBuf, Fingerprint)]) -> Vec<String> {
    unread
        .iter()
        .map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a_name_counts_unless_its_probe_found_no_media() {
    use MetadataValue::{Known, Unavailable, Unknown};
    let folder = FileEntry {
        kind: EntryKind::Directory,
        ..file("clips.mp4")
    };
    let cases = [
        (file("clip.mp4"), Some(Family::Video), "not probed yet"),
        (
            with("clip.mp4", Known(4), Unknown),
            Some(Family::Video),
            "a length",
        ),
        (
            with("clip.mp4", Unavailable, Unknown),
            None,
            "no media in it",
        ),
        (
            with("clip.h264", Unavailable, Known((320, 180))),
            Some(Family::Video),
            "a raw stream: dimensions, no length",
        ),
        (
            with("song.flac", Unavailable, Known((500, 500))),
            None,
            "audio with only a cover",
        ),
        (
            file("song.flac"),
            Some(Family::Audio),
            "audio not probed yet",
        ),
        (file("notes.txt"), None, "not a media name"),
        (folder, None, "a folder"),
    ];
    for (entry, family, why) in cases {
        assert_eq!(entry_family(&entry), family, "{why}");
    }
}

#[test]
fn unread_files_stay_out_and_are_read_nearest_first() {
    use MetadataValue::{Known, Unknown};
    let named = vec![
        file("a.mp4"),
        with("b.mp4", Known(4), Unknown),
        file("c.mp4"),
        file("shown.mp4"),
        file("d.mp4"),
        file("e.mp4"),
    ];
    let current = Location::local("/media/shown.mp4");
    let (files, unread) = counted(&Checks::default(), named, Family::Video, &current);
    assert_eq!(
        names(&files),
        ["b.mp4", "shown.mp4"],
        "the probed one and the shown one"
    );
    assert_eq!(
        paths(&unread),
        ["shown.mp4", "d.mp4", "c.mp4", "e.mp4", "a.mp4"],
        "the shown one, then outward, the next before the previous, as GTK reads them"
    );
}

#[test]
fn an_answer_holds_while_the_file_is_the_one_read() {
    let read = FileEntry {
        size: MetadataValue::Known(10),
        ..file("clip.mp4")
    };
    let mut checks = Checks::default();
    let path = PathBuf::from("/media/clip.mp4");
    checks.answers.insert(path, (Fingerprint::of(&read), false));
    let shown = Location::local("/media/shown.mp4");
    let cases = [
        (read.clone(), 0, "the same file: text, left out"),
        (
            file("clip.mp4"),
            0,
            "its size not listed yet: the answer still holds",
        ),
        (
            FileEntry {
                size: MetadataValue::Known(20),
                ..file("clip.mp4")
            },
            1,
            "changed since: read again",
        ),
    ];
    for (entry, unread_count, why) in cases {
        let (files, unread) = counted(&checks, vec![entry], Family::Video, &shown);
        assert!(files.is_empty(), "{why}");
        assert_eq!(unread.len(), unread_count, "{why}");
    }
}
