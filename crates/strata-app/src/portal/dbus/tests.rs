// SPDX-License-Identifier: MIT

//! The FileChooser call parser and its replies, on `glib::Variant`s built by hand: no bus.

use std::{
    ffi::{OsStr, OsString},
    future::Future,
};

use glib::{Variant, VariantDict, VariantTy, variant::ObjectPath};
use strata_test_support::PrivateContext;

use super::*;
use crate::portal::default_save_folder;

const HANDLE: &str = "/org/freedesktop/portal/desktop/request/1_42/t1";

fn run_async<T>(future: impl Future<Output = T>) -> T {
    let context = PrivateContext::new();
    context
        .with_thread_default(|| context.block_on(future))
        .expect("test main context")
}

fn handle(path: &str) -> Variant {
    ObjectPath::try_from(path)
        .expect("object path")
        .to_variant()
}

/// The five arguments of `OpenFile`, `SaveFile` and `SaveFiles`.
fn call(handle: Variant, parent: &str, title: Variant, options: Variant) -> Variant {
    Variant::tuple_from_iter([
        handle,
        "org.example.App".to_variant(),
        parent.to_variant(),
        title,
        options,
    ])
}

fn options(entries: &[(&str, Variant)]) -> Variant {
    let dict = VariantDict::new(None);
    for (key, value) in entries {
        dict.insert_value(key, value);
    }
    dict.end()
}

/// A NUL-terminated `ay`, as the portal sends a path.
fn bytes(raw: &[u8]) -> Variant {
    let mut terminated = raw.to_vec();
    terminated.push(0);
    Variant::array_from_fixed_array(&terminated)
}

fn params(options: Variant) -> CallParams {
    unpack_call(&call(handle(HANDLE), "", "".to_variant(), options)).expect("well-formed call")
}

fn is_invalid<T>(result: &Result<T, PortalError>) -> bool {
    matches!(result, Err(PortalError::InvalidArgument(_)))
}

#[test]
fn a_call_unpacks_its_handle_parent_and_title() {
    let parsed = unpack_call(&call(
        handle(HANDLE),
        "wayland:abc",
        "Pick".to_variant(),
        options(&[]),
    ))
    .expect("well-formed call");
    assert_eq!(parsed.token, HANDLE);
    assert_eq!(parsed.app_id, "org.example.App");
    assert_eq!(parsed.parent.as_deref(), Some("wayland:abc"));
    assert_eq!(parsed.title, "Pick");
    assert_eq!(params(options(&[])).parent, None, "an empty parent is none");
}

#[test]
fn a_malformed_call_is_an_invalid_argument() {
    let four = Variant::tuple_from_iter([
        handle(HANDLE),
        "app".to_variant(),
        "".to_variant(),
        "Pick".to_variant(),
    ]);
    let cases = [
        ("four arguments", four),
        (
            "a handle that is a string",
            call(HANDLE.to_variant(), "", "Pick".to_variant(), options(&[])),
        ),
        (
            "a handle outside the request tree",
            call(
                handle("/org/freedesktop/portal/desktop"),
                "",
                "Pick".to_variant(),
                options(&[]),
            ),
        ),
        (
            "a title that is a number",
            call(handle(HANDLE), "", 7u32.to_variant(), options(&[])),
        ),
    ];
    for (why, variant) in cases {
        assert!(is_invalid(&unpack_call(&variant)), "{why}");
    }
}

#[test]
fn open_defaults_match_the_portal_contract() {
    let request = run_async(build_open_request(params(options(&[])))).expect("open request");
    assert_eq!(request.token, HANDLE);
    assert!(request.modal);
    assert_eq!(request.title, "Open Files");
    assert_eq!(request.accept_label, "Open");
    assert_eq!(request.initial_directory, default_save_folder());
    assert_eq!(
        request.kind,
        ChooserKind::Open {
            directory: false,
            multiple: false
        }
    );
    assert!(request.filters.is_empty() && request.choices.is_empty());
}

fn images_and_text() -> (Vec<RawFilter>, Vec<PortalFileFilter>) {
    let raw: Vec<RawFilter> = vec![
        (
            "Images".to_owned(),
            vec![(0, "*.png".to_owned()), (1, "image/jpeg".to_owned())],
        ),
        ("Text".to_owned(), vec![(0, "*.txt".to_owned())]),
    ];
    let parsed = vec![
        PortalFileFilter {
            label: "Images".to_owned(),
            rules: vec![
                FilterRule::Pattern("*.png".to_owned()),
                FilterRule::MimeType("image/jpeg".to_owned()),
            ],
        },
        PortalFileFilter {
            label: "Text".to_owned(),
            rules: vec![FilterRule::Pattern("*.txt".to_owned())],
        },
    ];
    (raw, parsed)
}

fn two_choices() -> Vec<RawChoice> {
    vec![
        (
            "readonly".to_owned(),
            "Read only".to_owned(),
            Vec::new(),
            "true".to_owned(),
        ),
        (
            "encoding".to_owned(),
            "Encoding".to_owned(),
            vec![
                ("utf8".to_owned(), "UTF-8".to_owned()),
                ("latin1".to_owned(), "Latin-1".to_owned()),
            ],
            "utf8".to_owned(),
        ),
    ]
}

#[test]
fn open_options_keep_filters_choices_and_flags_in_order() {
    let folder = tempfile::tempdir().expect("current folder");
    let (raw, filters) = images_and_text();
    let request = run_async(build_open_request(params(options(&[
        ("accept_label", "Attach".to_variant()),
        ("modal", false.to_variant()),
        ("multiple", true.to_variant()),
        ("directory", true.to_variant()),
        ("current_filter", raw[1].to_variant()),
        ("filters", raw.to_variant()),
        ("choices", two_choices().to_variant()),
        (
            "current_folder",
            bytes(folder.path().as_os_str().as_bytes()),
        ),
    ]))))
    .expect("open request");
    let ids: Vec<&str> = request.choices.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        (request.modal, request.accept_label.as_str(), &request.kind),
        (
            false,
            "Attach",
            &ChooserKind::Open {
                directory: true,
                multiple: true
            }
        )
    );
    assert_eq!(request.initial_directory, folder.path());
    assert_eq!(request.current_filter.as_ref(), Some(&filters[1]));
    assert_eq!(request.filters, filters);
    assert_eq!(
        (ids, request.choices[1].pairs.len()),
        (vec!["readonly", "encoding"], 2)
    );
}

#[test]
fn malformed_options_are_invalid_arguments() {
    let cases = [
        ("modal as a string", ("modal", "yes".to_variant())),
        ("multiple as a number", ("multiple", 1u32.to_variant())),
        (
            "accept_label as a bool",
            ("accept_label", true.to_variant()),
        ),
        ("filters as a string", ("filters", "*.png".to_variant())),
        (
            "an unknown filter rule kind",
            (
                "filters",
                vec![("Odd".to_owned(), vec![(2u32, "x".to_owned())])].to_variant(),
            ),
        ),
        (
            "current_filter as a list",
            ("current_filter", Vec::<RawFilter>::new().to_variant()),
        ),
        ("choices as a string", ("choices", "x".to_variant())),
        (
            "current_folder as a string",
            ("current_folder", "/tmp".to_variant()),
        ),
        (
            "current_folder with a NUL inside",
            ("current_folder", bytes(b"/tmp\0/x")),
        ),
        (
            "an oversized current_folder",
            ("current_folder", bytes(&[b'a'; MAX_STRING_BYTES + 1])),
        ),
        (
            "a glob with too many wildcard groups",
            (
                "filters",
                vec![("Bad".to_owned(), vec![(0u32, "*a*b*c*".to_owned())])].to_variant(),
            ),
        ),
    ];
    for (why, (key, value)) in cases {
        let built = run_async(build_open_request(params(options(&[(key, value)]))));
        assert!(is_invalid(&built), "{why}");
    }
}

#[test]
fn byte_strings_drop_one_terminator_and_keep_non_utf8_bytes() {
    let raw = [b'/', b't', b'm', b'p', b'/', b'n', 0xff];
    assert_eq!(
        decode_c_bytes(&bytes(&raw), "path"),
        Ok(OsStr::from_bytes(&raw).to_os_string())
    );
    assert_eq!(
        decode_c_bytes(&Variant::array_from_fixed_array(b"plain"), "path"),
        Ok(OsString::from("plain")),
        "a missing terminator is tolerated"
    );
    let dict = VariantDict::new(Some(&options(&[
        ("terminated", bytes(b"")),
        ("empty", Variant::array_from_fixed_array(&[] as &[u8])),
    ])));
    for key in ["terminated", "empty", "absent"] {
        assert_eq!(lookup_bytestring_path(&dict, key), Ok(None), "{key}");
    }
}

#[test]
fn save_files_names_decode_in_order_and_must_be_safe() {
    let names = |list: &[&[u8]]| {
        Variant::array_from_iter_with_type(
            VariantTy::BYTE_STRING,
            list.iter().map(|raw| bytes(raw)),
        )
    };
    let request = run_async(build_save_files_request(params(options(&[(
        "files",
        names(&[b"b.txt", b"a\xff"]),
    )]))))
    .expect("save files request");
    assert_eq!(request.title, "Save Files");
    assert_eq!(
        request.kind,
        ChooserKind::SaveFiles {
            names: vec![
                OsString::from("b.txt"),
                OsStr::from_bytes(b"a\xff").to_os_string()
            ]
        }
    );
    for (why, files) in [
        ("no names", names(&[])),
        ("a path, not a name", names(&[b"../escape"])),
        ("names as one string", "a.txt".to_variant()),
        ("a name with a NUL inside", names(&[b"a\0b"])),
    ] {
        let built = run_async(build_save_files_request(params(options(&[(
            "files", files,
        )]))));
        assert!(is_invalid(&built), "{why}");
    }
}

#[test]
fn save_file_takes_a_bounded_current_name() {
    let folder = tempfile::tempdir().expect("current folder");
    let request = run_async(build_save_file_request(params(options(&[
        ("current_name", "report.txt".to_variant()),
        (
            "current_folder",
            bytes(folder.path().as_os_str().as_bytes()),
        ),
    ]))))
    .expect("save request");
    assert_eq!(request.title, "Save File");
    assert_eq!(request.accept_label, "Save");
    assert_eq!(request.initial_directory, folder.path());
    assert_eq!(
        request.kind,
        ChooserKind::SaveFile {
            current_name: Some(OsString::from("report.txt"))
        }
    );
    let long = run_async(build_save_file_request(params(options(&[(
        "current_name",
        "x".repeat(256).to_variant(),
    )]))));
    assert!(is_invalid(&long));
}

#[test]
fn replies_carry_the_response_code_and_only_the_results_given() {
    let (code, results) = cancelled_reply()
        .get::<(u32, VariantDict)>()
        .expect("(ua{sv})");
    assert_eq!(code, 1);
    assert_eq!(results.end().n_children(), 0);

    let bare = ok_reply(SelectedResult {
        uris: vec!["file:///tmp/a".to_owned()],
        ..SelectedResult::default()
    });
    let (code, results) = bare.get::<(u32, VariantDict)>().expect("(ua{sv})");
    assert_eq!(code, 0);
    assert_eq!(
        results.lookup::<Vec<String>>("uris").expect("uris type"),
        Some(vec!["file:///tmp/a".to_owned()])
    );
    for key in ["choices", "current_filter", "writable"] {
        assert!(!results.contains(key), "{key} only when given");
    }

    let filter = PortalFileFilter {
        label: "Text".to_owned(),
        rules: vec![
            FilterRule::Pattern("*.txt".to_owned()),
            FilterRule::MimeType("text/plain".to_owned()),
        ],
    };
    let full = ok_reply(SelectedResult {
        uris: vec!["file:///tmp/a".to_owned(), "file:///tmp/b".to_owned()],
        choices: vec![("encoding".to_owned(), "utf8".to_owned())],
        current_filter: Some(filter.clone()),
        writable: Some(false),
    });
    let (_, results) = full.get::<(u32, VariantDict)>().expect("(ua{sv})");
    assert_eq!(
        results
            .lookup::<Vec<(String, String)>>("choices")
            .expect("a(ss)"),
        Some(vec![("encoding".to_owned(), "utf8".to_owned())])
    );
    let echoed = results
        .lookup::<RawFilter>("current_filter")
        .expect("(sa(us))")
        .expect("current filter");
    assert_eq!(parse_raw_filter(echoed), Ok(filter));
    assert_eq!(results.lookup::<bool>("writable").expect("b"), Some(false));
}
