// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    ffi::{OsStr, OsString},
    future::Future,
    os::unix::ffi::OsStringExt as _,
    sync::atomic::Ordering,
};

use ashpd::desktop::{
    HandleToken,
    file_chooser::{Choice, FileFilter, OpenFileOptions},
};

use super::*;
use crate::model::{EntryKind, FileEntry, MetadataValue};

fn entry(path: &Path, directory: bool) -> FileEntry {
    FileEntry {
        location: Location::local(path),
        native_name: path.file_name().unwrap_or_default().to_owned(),
        display_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        kind: if directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
        is_hidden: false,
        mode: MetadataValue::Unknown,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
    }
}

#[test]
fn open_defaults_match_the_portal_contract() {
    let request = run_async(open_request(
        HandleToken::default().to_string(),
        None,
        "",
        OpenFileOptions::default(),
    ))
    .expect("open request");
    assert!(request.modal);
    assert_eq!(request.title, "Open Files");
    assert_eq!(request.accept_label, "Open");
    assert!(matches!(
        request.kind,
        ChooserKind::Open {
            directory: false,
            multiple: false
        }
    ));
}

#[test]
fn current_file_takes_precedence_over_folder_and_name() {
    let current = tempfile::tempdir().expect("current directory");
    let ignored = tempfile::tempdir().expect("ignored directory");
    let file = current.path().join("existing.txt");
    std::fs::write(&file, b"data").expect("fixture file");

    let suggestion = run_async(save_file_suggestion(
        Some(file),
        Some(ignored.path().to_path_buf()),
        Some("ignored.txt".to_owned()),
    ))
    .expect("save suggestion");
    assert_eq!(suggestion.0, current.path());
    assert_eq!(suggestion.1.as_deref(), Some(OsStr::new("existing.txt")));
}

#[test]
fn current_file_preserves_a_non_utf8_filename() {
    let current = tempfile::tempdir().expect("current directory");
    let name = OsString::from_vec(vec![b'n', 0xff]);
    let file = current.path().join(&name);
    std::fs::write(&file, b"data").expect("fixture file");

    let suggestion =
        run_async(save_file_suggestion(Some(file), None, None)).expect("save suggestion");
    assert_eq!(suggestion, (current.path().to_path_buf(), Some(name)));
}

#[test]
fn invalid_current_file_falls_back_without_using_lower_priority_suggestions() {
    let ignored = tempfile::tempdir().expect("ignored directory");
    let suggestion = run_async(save_file_suggestion(
        Some(PathBuf::from("relative/missing.txt")),
        Some(ignored.path().to_path_buf()),
        Some("ignored.txt".to_owned()),
    ))
    .expect("save suggestion");
    assert_eq!(suggestion, (crate::ui::home_directory(), None));
}

#[test]
fn local_uri_preserves_non_utf8_path_bytes() {
    let path = PathBuf::from("/tmp").join(OsString::from_vec(vec![b'n', 0xff]));
    let uri = local_uri(&path).expect("local URI");
    assert!(uri.as_str().starts_with("file://"));
    assert!(uri.as_str().to_ascii_uppercase().contains("%FF"));
}

#[test]
fn save_filenames_must_be_safe_basenames() {
    for unsafe_name in [
        OsStr::new(""),
        OsStr::new("."),
        OsStr::new(".."),
        OsStr::new("a/b"),
    ] {
        assert!(!safe_filename(unsafe_name), "{unsafe_name:?}");
    }
    assert!(safe_filename(OsStr::new("report.txt")));
    assert!(safe_filename(&OsString::from_vec(vec![b'n', 0xff])));
    assert!(validate_save_filenames(&[]).is_err());
    assert!(validate_save_filenames(&[OsString::from("../bad")]).is_err());
}

#[test]
fn save_files_preserve_order_and_report_collisions_once() {
    let folder = tempfile::tempdir().expect("destination");
    std::fs::write(folder.path().join("second"), b"existing").expect("collision");
    let names = vec![OsString::from("first"), OsString::from("second")];
    let checked = run_async(check_destinations(folder.path(), &names)).expect("safe destinations");
    assert_eq!(
        checked.paths,
        vec![folder.path().join("first"), folder.path().join("second")]
    );
    assert!(checked.existing_files);
}

#[test]
fn save_files_block_directory_collisions() {
    let folder = tempfile::tempdir().expect("destination");
    std::fs::create_dir(folder.path().join("reserved")).expect("collision directory");
    assert!(
        run_async(check_destinations(
            folder.path(),
            &[OsString::from("reserved")],
        ))
        .expect_err("directory collision")
        .contains("folder")
    );
}

#[test]
fn filters_and_choices_keep_input_order_and_current_filter() {
    let images = FileFilter::new("Images")
        .glob("*.png")
        .mimetype("image/jpeg");
    let text = FileFilter::new("Text").glob("*.txt");
    let encoding = Choice::new("encoding", "Encoding", "utf8")
        .insert("utf8", "UTF-8")
        .insert("latin1", "Latin-1");
    let request = run_async(open_request(
        HandleToken::default().to_string(),
        None,
        "Choose",
        OpenFileOptions::default()
            .set_filters([images.clone(), text.clone()])
            .set_current_filter(Some(text.clone()))
            .set_choices([Choice::boolean("readonly", "Read only", true), encoding]),
    ))
    .expect("open request");
    assert_eq!(request.filters, [images, text.clone()]);
    assert_eq!(request.current_filter, Some(text));
    assert_eq!(
        request.choices.iter().map(Choice::id).collect::<Vec<_>>(),
        ["readonly", "encoding"]
    );
}

#[test]
fn readonly_state_maps_to_writable_result() {
    assert!(!writable_from_read_only(true));
    assert!(writable_from_read_only(false));
}

#[test]
fn open_selection_validates_kind_cardinality_and_locality() {
    let current = Location::local("/tmp");
    let file = entry(Path::new("/tmp/file"), false);
    let folder = entry(Path::new("/tmp/folder"), true);
    assert_eq!(
        open_selection(std::slice::from_ref(&file), &current, false, false).expect("single file"),
        [PathBuf::from("/tmp/file")]
    );
    assert!(open_selection(&[file.clone(), folder.clone()], &current, false, true).is_err());
    assert!(open_selection(&[file.clone(), file], &current, false, false).is_err());
    assert_eq!(
        open_selection(&[], &current, true, false).expect("current folder"),
        [PathBuf::from("/tmp")]
    );
    let remote = FileEntry {
        location: Location::uri("smb://server/share/file"),
        ..folder
    };
    assert!(open_selection(&[remote], &current, true, false).is_err());
}

#[test]
fn cancellation_before_presentation_is_sticky_and_cleanup_is_race_safe() {
    let tracker = Arc::new(RequestTracker::default());
    let first = tracker.begin("same".into()).expect("first request");
    assert!(tracker.begin("same".into()).is_err());
    assert!(tracker.cancel("same"));
    assert!(first.cancelled.load(Ordering::SeqCst));

    drop(first);
    let replacement = tracker.begin("same".into()).expect("replacement request");
    assert!(tracker.cancel("same"));
    assert!(replacement.cancelled.load(Ordering::SeqCst));
    drop(replacement);
    assert!(!tracker.cancel("same"));
}

#[test]
fn untrusted_request_inputs_are_bounded() {
    let too_long = "x".repeat(MAX_STRING_BYTES + 1);
    assert!(
        validate_common_request(&too_long, None, None, &[]).is_err(),
        "titles must be bounded"
    );

    let choices = (0..=MAX_CHOICES)
        .map(|index| Choice::boolean(&format!("choice-{index}"), "Choice", false))
        .collect::<Vec<_>>();
    assert!(validate_choices(&choices).is_err());
    let choice = (0..=MAX_CHOICE_OPTIONS).fold(
        Choice::new("choice", "Choice", "option-0"),
        |choice, index| choice.insert(&format!("option-{index}"), "Option"),
    );
    assert!(validate_choices(&[choice]).is_err());

    let filters = (0..=MAX_FILTERS)
        .map(|index| FileFilter::new(&format!("Filter {index}")))
        .collect::<Vec<_>>();
    assert!(validate_filters(&filters, None).is_err());
    let filter = (0..=MAX_FILTER_RULES).fold(FileFilter::new("Filter"), |filter, index| {
        filter.glob(&format!("*.{index}"))
    });
    assert!(validate_filters(&[filter], None).is_err());
    assert!(
        validate_filters(&[FileFilter::new("Filter").glob("*a*a*a*z")], None).is_err(),
        "backtracking-heavy globs must be rejected"
    );
    assert!(
        validate_filters(&[FileFilter::new("Filter").glob("*a*.txt")], None).is_ok(),
        "two wildcard groups remain supported"
    );
    assert!(
        validate_filters(
            &[FileFilter::new("Filter").glob(&"a".repeat(MAX_GLOB_BYTES + 1))],
            None,
        )
        .is_err(),
        "glob length must be bounded"
    );

    let filenames = vec![OsString::from("file"); MAX_SAVE_FILES + 1];
    assert!(validate_save_filenames(&filenames).is_err());
    assert!(
        validate_save_filenames(&[OsString::from("x".repeat(MAX_FILENAME_BYTES + 1))]).is_err()
    );
    assert!(validate_path(Some(Path::new(&"x".repeat(MAX_STRING_BYTES + 1))), "path").is_err());
}

#[test]
fn active_request_count_is_bounded() {
    let tracker = Arc::new(RequestTracker::default());
    let requests = (0..MAX_ACTIVE_REQUESTS)
        .map(|index| {
            tracker
                .begin(format!("request{index}"))
                .expect("request within limit")
        })
        .collect::<Vec<_>>();
    assert!(tracker.begin("one-too-many".into()).is_err());
    drop(requests);
    assert!(tracker.begin("available-again".into()).is_ok());
}

#[test]
fn backend_version_and_success_uri_scheme_are_fixed() {
    assert_eq!(FILE_CHOOSER_VERSION, 4);
    for path in [Path::new("/tmp/a"), Path::new("/tmp/a b")] {
        assert!(
            local_uri(path)
                .expect("valid URI")
                .as_str()
                .starts_with("file://")
        );
    }
}

fn run_async<T>(future: impl Future<Output = T>) -> T {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| context.block_on(future))
        .expect("test main context")
}
