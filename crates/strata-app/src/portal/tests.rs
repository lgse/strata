// SPDX-License-Identifier: MIT

use std::{
    ffi::{OsStr, OsString},
    future::Future,
    os::unix::ffi::OsStringExt as _,
    path::Path,
};

use strata_core::model::{EntryKind, FileEntry, MetadataValue};
use strata_test_support::PrivateContext;

use super::*;

mod bounds;

fn run_async<T>(future: impl Future<Output = T>) -> T {
    let context = PrivateContext::new();
    context
        .with_thread_default(|| context.block_on(future))
        .expect("test main context")
}

fn entry(path: &Path, directory: bool) -> FileEntry {
    FileEntry {
        location: Location::local(path),
        native_name: path.file_name().unwrap_or_default().to_owned(),
        thumbnail_path: None,
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
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        recent_uri: None,
    }
}

fn filter(label: &str, rules: Vec<FilterRule>) -> PortalFileFilter {
    PortalFileFilter {
        label: label.to_owned(),
        rules,
    }
}

fn boolean_choice(id: &str) -> PortalChoice {
    PortalChoice {
        id: id.to_owned(),
        label: "Choice".to_owned(),
        pairs: Vec::new(),
        initial_selection: "false".to_owned(),
    }
}

#[test]
fn folder_hints_override_defaults_and_invalid_hints_preserve_save_names() {
    let current = tempfile::tempdir().expect("current directory");
    for hint in [
        None,
        Some(PathBuf::from("relative")),
        Some(current.path().join("missing")),
        Some(current.path().to_path_buf()),
    ] {
        let expected = if hint.as_deref() == Some(current.path()) {
            current.path().to_path_buf()
        } else {
            default_save_folder()
        };
        assert_eq!(run_async(accessible_folder(hint.clone())), expected);
        assert_eq!(
            run_async(save_file_suggestion(
                None,
                hint,
                Some("report.txt".to_owned())
            )),
            (expected, Some(OsString::from("report.txt")))
        );
    }
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
    ));
    assert_eq!(
        suggestion,
        (
            current.path().to_path_buf(),
            Some(OsString::from("existing.txt"))
        )
    );
}

#[test]
fn current_file_keeps_its_name_whether_or_not_it_exists() {
    let current = tempfile::tempdir().expect("current directory");
    let existing = OsString::from_vec(vec![b'n', 0xff]);
    std::fs::write(current.path().join(&existing), b"data").expect("fixture file");
    for (name, exists) in [
        (existing, true),
        (OsString::from("packing list 格式(2).xls"), false),
        (OsString::from_vec(vec![b'm', 0xfe]), false),
    ] {
        let file = current.path().join(&name);
        let suggestion = run_async(save_file_suggestion(
            Some(file.clone()),
            Some(current.path().to_path_buf()),
            None,
        ));
        assert_eq!(suggestion, (current.path().to_path_buf(), Some(name)));
        assert_eq!(
            file.exists(),
            exists,
            "suggesting a name must not create a file"
        );
    }
}

#[test]
fn an_unusable_current_file_falls_back_without_the_lower_priority_hints() {
    let current = tempfile::tempdir().expect("current directory");
    let ignored = tempfile::tempdir().expect("ignored directory");
    for file in [
        current.path().to_path_buf(),
        current.path().join("missing/new.txt"),
        PathBuf::from("relative/missing.txt"),
    ] {
        let suggestion = run_async(save_file_suggestion(
            Some(file.clone()),
            Some(ignored.path().to_path_buf()),
            Some("ignored.txt".to_owned()),
        ));
        assert_eq!(suggestion, (default_save_folder(), None), "{file:?}");
    }
}

#[test]
fn local_uris_are_absolute_file_uris_that_keep_non_utf8_bytes() {
    let raw = PathBuf::from("/tmp").join(OsString::from_vec(vec![b'n', 0xff]));
    let uri = local_uri(&raw).expect("local URI");
    assert!(uri.starts_with("file://"));
    assert!(uri.to_ascii_uppercase().contains("%FF"));
    for path in [Path::new("/tmp/a"), Path::new("/tmp/a b")] {
        assert!(local_uri(path).expect("valid URI").starts_with("file://"));
    }
    assert!(matches!(
        local_uri(Path::new("relative")),
        Err(PortalError::InvalidArgument(_))
    ));
}

#[test]
fn save_filenames_must_be_safe_bounded_basenames() {
    assert!(safe_filename(&OsString::from_vec(vec![b'n', 0xff])));
    for names in [
        Vec::new(),
        vec![OsString::from("../bad")],
        vec![OsString::from("file"); MAX_SAVE_FILES + 1],
        vec![OsString::from("x".repeat(MAX_FILENAME_BYTES + 1))],
    ] {
        assert!(
            validate_save_filenames(&names).is_err(),
            "{} names",
            names.len()
        );
    }
    assert!(validate_save_filenames(&[OsString::from("report.txt")]).is_ok());
}

#[test]
fn save_files_keep_their_order_and_report_each_collision_once() {
    let folder = tempfile::tempdir().expect("destination");
    std::fs::write(folder.path().join("second"), b"existing").expect("collision");
    let names = vec![OsString::from("first"), OsString::from("second")];
    let checked = run_async(check_destinations(folder.path(), &names)).expect("safe destinations");
    assert_eq!(
        checked.paths,
        vec![folder.path().join("first"), folder.path().join("second")]
    );
    assert_eq!(checked.existing, vec![folder.path().join("second")]);
}

#[test]
fn save_files_refuse_a_folder_or_a_link_to_one_in_the_way() {
    let folder = tempfile::tempdir().expect("destination");
    std::fs::create_dir(folder.path().join("reserved")).expect("collision directory");
    std::os::unix::fs::symlink(folder.path().join("reserved"), folder.path().join("linked"))
        .expect("link to the folder");
    for name in ["reserved", "linked"] {
        let refused = run_async(check_destinations(folder.path(), &[OsString::from(name)]));
        assert!(refused.expect_err(name).contains(name), "{name}");
    }
    let relative = run_async(check_destinations(
        Path::new("relative"),
        &[OsString::from("a")],
    ));
    assert!(relative.is_err());
}

#[test]
fn open_selection_validates_kind_cardinality_and_locality() {
    let current = Location::local("/tmp");
    let file = entry(Path::new("/tmp/file"), false);
    let folder = entry(Path::new("/tmp/folder"), true);
    let remote = FileEntry {
        location: Location::uri("smb://server/share/file"),
        ..entry(Path::new("/tmp/remote"), true)
    };
    let picked =
        |path: &str| -> Result<Vec<PathBuf>, &'static str> { Ok(vec![PathBuf::from(path)]) };
    let cases = [
        (vec![file.clone()], false, false, picked("/tmp/file")),
        (
            vec![file.clone(), folder],
            false,
            true,
            Err("Choose files only"),
        ),
        (
            vec![file.clone(), file.clone()],
            false,
            false,
            Err("Choose one item"),
        ),
        (vec![], false, false, Err("Choose a file")),
        (vec![], true, false, picked("/tmp")),
        (vec![file], true, false, Err("Choose folders only")),
        (vec![remote], true, false, Err("Choose local items only")),
    ];
    for (entries, directory, multiple, expected) in cases {
        assert_eq!(
            open_selection(&entries, &current, directory, multiple),
            expected,
            "{} entries, directory={directory}, multiple={multiple}",
            entries.len()
        );
    }
}

#[test]
fn cancellation_before_presentation_is_sticky_and_cleanup_is_race_safe() {
    let tracker = Arc::new(RequestTracker::default());
    let first = tracker.begin("same".into()).expect("first request");
    let twice = tracker.begin("same".into()).is_err();
    let closes = [tracker.cancel("same"), tracker.cancel("same")];
    assert_eq!(
        (twice, closes, first.cancelled.load(Ordering::SeqCst)),
        (true, [true, false], true),
        "one request per token; a second Close changes nothing; the request stays cancelled"
    );
    drop(first);
    let replacement = tracker.begin("same".into()).expect("replacement request");
    let fresh = !replacement.cancelled.load(Ordering::SeqCst);
    let closed = tracker.cancel("same");
    let cancelled = replacement.cancelled.load(Ordering::SeqCst);
    drop(replacement);
    assert_eq!(
        (fresh, closed, cancelled, tracker.cancel("same")),
        (true, true, true, false),
        "a replacement starts uncancelled, and a finished one leaves nothing to close"
    );
}

#[test]
fn a_finished_request_does_not_free_the_slot_of_its_replacement() {
    let tracker = Arc::new(RequestTracker::default());
    let first = tracker.begin("same".into()).expect("first request");
    let stale = first.cancelled.clone();
    drop(first);
    let replacement = tracker.begin("same".into()).expect("replacement request");
    tracker.finish("same", &stale);
    assert!(
        tracker.begin("same".into()).is_err(),
        "the replacement still holds it"
    );
    drop(replacement);
}

#[test]
fn backend_idles_only_after_the_last_request_finishes() {
    let tracker = Arc::new(RequestTracker::default());
    let first = tracker.begin("first".into()).expect("first request");
    let second = tracker.begin("second".into()).expect("second request");
    let old = Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1);
    let age = |tracker: &RequestTracker| {
        tracker.state.lock().expect("request state").last_activity = old;
    };
    age(&tracker);
    assert!(!tracker.begin_shutdown_if_idle(Instant::now()));
    drop(first);
    assert!(!tracker.begin_shutdown_if_idle(Instant::now()));
    drop(second);
    assert!(!tracker.begin_shutdown_if_idle(Instant::now()));
    age(&tracker);
    let third = tracker
        .begin("third".into())
        .expect("new request resets idle");
    assert!(!tracker.begin_shutdown_if_idle(Instant::now()));
    drop(third);
    assert!(!tracker.begin_shutdown_if_idle(Instant::now()));
    age(&tracker);
    assert!(tracker.begin_shutdown_if_idle(Instant::now()));
    assert!(matches!(
        tracker.begin("too-late".into()),
        Err(PortalError::Failed(_))
    ));
}

#[test]
fn active_request_count_and_token_length_are_bounded() {
    let tracker = Arc::new(RequestTracker::default());
    let requests = (0..MAX_ACTIVE_REQUESTS)
        .map(|index| {
            tracker
                .begin(format!("request{index}"))
                .expect("request within limit")
        })
        .collect::<Vec<_>>();
    assert!(matches!(
        tracker.begin("one-too-many".into()),
        Err(PortalError::Failed(_))
    ));
    drop(requests);
    assert!(tracker.begin("available-again".into()).is_ok());
    assert!(matches!(
        tracker.begin("x".repeat(MAX_STRING_BYTES + 1)),
        Err(PortalError::InvalidArgument(_))
    ));
}

#[test]
fn long_filter_lists_are_accepted() {
    let filters = (0..256)
        .map(|index| {
            filter(
                &format!("Filter {index}"),
                vec![FilterRule::MimeType(format!(
                    "application/x-upload-{index}"
                ))],
            )
        })
        .collect::<Vec<_>>();
    assert!(
        validate_filters(&filters, None).is_ok(),
        "a large well-formed filter list must open the chooser"
    );
    let current = filter(
        "Selected",
        vec![FilterRule::MimeType("application/x-selected".to_owned())],
    );
    assert!(
        validate_filters(&filters, Some(&current)).is_ok(),
        "a current filter outside a large list must be accepted"
    );
}

#[test]
fn a_blank_title_or_label_takes_the_portal_default() {
    assert_eq!(
        [
            request_title("  ", "Open Files"),
            request_title("Pick", "Open Files"),
            default_accept_label(None, "Open"),
            default_accept_label(Some("Attach"), "Open"),
        ],
        ["Open Files", "Pick", "Open", "Attach"]
    );
}

#[test]
fn safe_filename_rejects_empty_slashes_dots_and_nul() {
    assert!(safe_filename(OsStr::new("report.png")));
    for bad in ["", ".", "..", "a/b", "a\0b"] {
        assert!(!safe_filename(OsStr::new(bad)), "expected unsafe: {bad:?}");
    }
}

#[test]
fn validate_filters_and_choices_enforce_limits_and_glob_rules() {
    let ok = PortalFileFilter {
        label: "Images".to_owned(),
        rules: vec![
            FilterRule::Pattern("*.png".to_owned()),
            FilterRule::MimeType("image/png".to_owned()),
        ],
    };
    assert!(validate_filters(&[ok], None).is_ok());

    let too_many_stars = PortalFileFilter {
        label: "Bad".to_owned(),
        rules: vec![FilterRule::Pattern("*a*b*c*".to_owned())],
    };
    assert!(matches!(
        validate_filters(&[too_many_stars], None),
        Err(PortalError::InvalidArgument(_))
    ));

    let valid_choice = PortalChoice {
        id: "encoding".to_owned(),
        label: "Encoding".to_owned(),
        pairs: vec![("utf8".to_owned(), "UTF-8".to_owned())],
        initial_selection: "utf8".to_owned(),
    };
    assert!(validate_common_request("Choose", Some("Open"), None, &[valid_choice]).is_ok());
}

#[test]
fn hyprland_window_size_matches_single_mapped_visible_client() {
    let valid =
        br#"[{"class":"zen","initialClass":"zen","mapped":true,"hidden":false,"size":[960,1000]}]"#;
    assert_eq!(
        window_geometry::application_window_size(valid, "ZEN"),
        Some((960, 1000))
    );

    let duplicate = br#"[
        {"class":"zen","initialClass":"zen","mapped":true,"hidden":false,"size":[960,1000]},
        {"class":"zen","initialClass":"zen","mapped":true,"hidden":false,"size":[800,600]}
    ]"#;
    assert_eq!(
        window_geometry::application_window_size(duplicate, "zen"),
        None
    );
    for (reply, why) in [
        (
            &br#"[{"class":"zen","initialClass":"zen","mapped":true,"hidden":true,"size":[960,1000]}]"#[..],
            "a hidden client",
        ),
        (
            br#"[{"class":"zen","initialClass":"zen","mapped":false,"hidden":false,"size":[960,1000]}]"#,
            "an unmapped client",
        ),
        (
            br#"[{"class":"zen","initialClass":"zen","mapped":true,"hidden":false,"size":[0,1000]}]"#,
            "a client with no width",
        ),
        (
            br#"[{"class":"zen","mapped":true,"hidden":false,"size":[960,1000]}]"#,
            "a client without an initial class",
        ),
        (
            br#"[{"class":"zen","initialClass":"zen","mapped":true,"hidden":false,"size":[960]}]"#,
            "a size that is not a pair",
        ),
        (
            br#"[{"class":"x","initialClass":"x","mapped":1,"hidden":false,"size":[9,9]},{"class":"zen","initialClass":"zen","mapped":true,"hidden":false,"size":[960,1000]}]"#,
            "a reply with a malformed client",
        ),
        (br#"{"class":"zen"}"#, "a reply that is not a list"),
    ] {
        assert_eq!(
            window_geometry::application_window_size(reply, "zen"),
            None,
            "{why}"
        );
    }
    let by_initial_class = br#"[{"class":"other","initialClass":"Zen","mapped":true,"hidden":false,"size":[800,600]}]"#;
    assert_eq!(
        window_geometry::application_window_size(by_initial_class, "zen"),
        Some((800, 600))
    );
}

#[test]
fn hyprland_socket_needs_an_absolute_runtime_and_a_plain_signature() {
    let runtime = Path::new("/run/user/1000");
    assert_eq!(
        window_geometry::socket_path(runtime, "abc_12-x"),
        Some(runtime.join("hypr/abc_12-x/.socket.sock"))
    );
    for (runtime, signature) in [
        ("/run/user/1000", "../bad"),
        ("/run/user/1000", ""),
        ("run/user/1000", "abc"),
    ] {
        assert_eq!(
            window_geometry::socket_path(Path::new(runtime), signature),
            None,
            "{runtime:?} {signature:?}"
        );
    }
}
