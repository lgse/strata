// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{ArchiveDirectory, ArchiveFormat, ArchiveNode, ArchivePreviewTree};

const DAMAGED_ARCHIVE_MESSAGE: &str = "This file is not a valid archive or is damaged.";

fn archive_file(name: &str, size: u64) -> ArchiveNode {
    ArchiveNode::File {
        name: name.to_owned(),
        size,
    }
}

fn archive_directory(name: &str, children: Vec<ArchiveNode>) -> ArchiveNode {
    ArchiveNode::Directory(ArchiveDirectory {
        name: name.to_owned(),
        children,
    })
}

fn archive_tree(children: Vec<ArchiveNode>, file_count: usize) -> ArchivePreviewTree {
    ArchivePreviewTree {
        root: ArchiveDirectory {
            name: String::new(),
            children,
        },
        file_count,
    }
}

fn list_archive_directly(
    path: &Path,
    operation: ParseOperation,
    _: i32,
    _: MediaPreviewBackend,
    _: &Cancellation,
) -> Result<crate::sandbox::ParseOutput, String> {
    let ParseOperation::ArchiveList { format, password } = operation else {
        panic!("archive previews must request an archive listing");
    };
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let result = crate::adapters::local_operations::list_archive_entries_direct(
        path,
        format,
        password.as_ref().map(crate::services::SecretString::expose),
        &cancelled,
    );
    Ok(parse_output(
        crate::sandbox::archive_listing::encode_archive_result(&result),
        0,
        0,
    ))
}

fn report_unsupported_listing(
    _: &Path,
    operation: ParseOperation,
    _: i32,
    _: MediaPreviewBackend,
    _: &Cancellation,
) -> Result<crate::sandbox::ParseOutput, String> {
    let ParseOperation::ArchiveList { .. } = operation else {
        panic!("archive previews must request an archive listing");
    };
    let listing = crate::adapters::local_operations::ArchiveListing {
        status: crate::adapters::local_operations::ArchiveListingStatus::Unsupported,
        entries: Vec::new(),
    };
    Ok(parse_output(
        crate::sandbox::archive_listing::encode_archive_result(&Ok(listing)),
        0,
        0,
    ))
}

fn load_archive(
    path: &Path,
    name: &str,
    password: Option<&str>,
    render: fn(
        &Path,
        ParseOperation,
        i32,
        MediaPreviewBackend,
        &Cancellation,
    ) -> Result<crate::sandbox::ParseOutput, String>,
) -> Vec<PreviewEvent> {
    let request = PreviewRequest {
        archive_password: password
            .map(|password| crate::services::SecretString::new(password.to_owned())),
        ..preview_request(crate::model::Location::local(path), name)
    };
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("main context owner");
    load_until_first_event(&software_provider(), &context, request, render)
}

fn preview_archive(path: &Path, name: &str) -> Vec<PreviewEvent> {
    preview_archive_with_password(path, name, None)
}

fn preview_archive_with_password(
    path: &Path,
    name: &str,
    password: Option<&str>,
) -> Vec<PreviewEvent> {
    load_archive(path, name, password, list_archive_directly)
}

fn only_event(events: &[PreviewEvent]) -> &PreviewEvent {
    assert_eq!(events.len(), 1);
    &events[0]
}

fn expect_archive_tree(events: &[PreviewEvent]) -> &ArchivePreviewTree {
    let PreviewEvent::Ready(preview) = only_event(events) else {
        panic!("archive preview failed: {:?}", events[0]);
    };
    let PreviewContent::Archive { tree } = &preview.content else {
        panic!("expected an archive tree, got {:?}", preview.content);
    };
    tree
}

fn expect_failure<'a>(events: &'a [PreviewEvent], expected: &str) -> &'a str {
    let PreviewEvent::Failed { message, .. } = only_event(events) else {
        panic!("expected {expected}, got {:?}", events[0]);
    };
    message
}

fn encrypted_archive_fixture(
    directory: &tempfile::TempDir,
    name: &str,
    format: ArchiveFormat,
) -> PathBuf {
    let source = directory.path().join("folder");
    fs::create_dir_all(&source).expect("create fixture source");
    if !source.join("item.txt").exists() {
        fs::write(source.join("item.txt"), b"contents").expect("write fixture source");
    }
    let path = directory.path().join(name);
    crate::adapters::local_operations::write_compression_fixture(
        &path,
        &[source],
        format,
        Some("s3cret"),
    )
    .expect("write encrypted fixture");
    path
}

fn write_sample_zip(path: &Path) {
    let mut writer = zip::ZipWriter::new(fs::File::create(path).expect("create zip fixture"));
    let _ = writer.start_file("a.txt", zip::write::SimpleFileOptions::default());
    let _ = std::io::Write::write_all(&mut writer, b"hello");
    let _ = writer.start_file("folder/b.txt", zip::write::SimpleFileOptions::default());
    let _ = std::io::Write::write_all(&mut writer, b"abc");
    writer.finish().expect("write zip fixture");
}

fn write_sample_tar_gz(path: &Path) {
    let file = fs::File::create(path).expect("create tar.gz fixture");
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        file,
        flate2::Compression::default(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_mode(0o644);
    header.set_size(2);
    header.set_entry_type(tar::EntryType::Regular);
    builder
        .append_data(&mut header, "inner/readme.txt", &b"hi"[..])
        .expect("write tar.gz fixture");
    let encoder = builder.into_inner().expect("finish tar.gz fixture");
    encoder.finish().expect("finish gzip trailer");
}

#[test]
fn archives_list_member_trees_as_preview_content() {
    crate::test_support::isolated_test(
        "adapters::local_preview::tests::archive::archives_list_member_trees_as_preview_content",
        || {
            let directory = tempfile::tempdir().expect("preview fixture directory");
            let path = directory.path().join("sample.zip");
            write_sample_zip(&path);

            let events = preview_archive(&path, "sample.zip");
            assert_eq!(
                expect_archive_tree(&events),
                &archive_tree(
                    vec![
                        archive_directory("folder", vec![archive_file("b.txt", 3)]),
                        archive_file("a.txt", 5),
                    ],
                    2,
                )
            );
        },
    );
}

#[test]
fn tar_gz_archives_list_member_trees_as_preview_content() {
    crate::test_support::isolated_test(
        "adapters::local_preview::tests::archive::tar_gz_archives_list_member_trees_as_preview_content",
        || {
            let directory = tempfile::tempdir().expect("preview fixture directory");
            let path = directory.path().join("docs.tar.gz");
            write_sample_tar_gz(&path);

            let events = preview_archive(&path, "docs.tar.gz");
            assert_eq!(
                expect_archive_tree(&events),
                &archive_tree(
                    vec![archive_directory(
                        "inner",
                        vec![archive_file("readme.txt", 2)]
                    )],
                    1,
                )
            );
        },
    );
}

#[test]
fn corrupt_archives_report_a_failed_preview() {
    crate::test_support::isolated_test(
        "adapters::local_preview::tests::archive::corrupt_archives_report_a_failed_preview",
        || {
            let directory = tempfile::tempdir().expect("preview fixture directory");
            let path = directory.path().join("fake.zip");
            fs::write(&path, b"PK\x03\x04\0\0\0\0not a zip archive")
                .expect("write corrupt fixture");
            let events = preview_archive(&path, "fake.zip");
            assert_eq!(
                expect_failure(&events, "a failed archive preview"),
                DAMAGED_ARCHIVE_MESSAGE
            );
        },
    );
}

#[test]
fn encrypted_archive_preview_prompts_for_a_password() {
    crate::test_support::isolated_test(
        "adapters::local_preview::tests::archive::encrypted_archive_preview_prompts_for_a_password",
        || {
            let directory = tempfile::tempdir().expect("preview fixture directory");
            let zip = encrypted_archive_fixture(&directory, "secret.zip", ArchiveFormat::Zip);
            let events = preview_archive(&zip, "secret.zip");
            let PreviewEvent::NeedsPassword { entry, .. } = only_event(&events) else {
                panic!("expected a password request, got {:?}", events[0]);
            };
            assert_eq!(entry.native_name, "secret.zip");

            let seven_z = encrypted_archive_fixture(&directory, "secret.7z", ArchiveFormat::SevenZ);
            let events = preview_archive(&seven_z, "secret.7z");
            assert!(
                matches!(only_event(&events), PreviewEvent::NeedsPassword { .. }),
                "expected a password request, got {:?}",
                events[0]
            );
        },
    );
}

#[test]
fn encrypted_zip_preview_unlocks_with_the_correct_password() {
    crate::test_support::isolated_test(
        "adapters::local_preview::tests::archive::encrypted_zip_preview_unlocks_with_the_correct_password",
        || {
            let directory = tempfile::tempdir().expect("preview fixture directory");
            let zip = encrypted_archive_fixture(&directory, "secret.zip", ArchiveFormat::Zip);
            let events = preview_archive_with_password(&zip, "secret.zip", Some("s3cret"));
            let tree = expect_archive_tree(&events);
            assert_eq!(tree.file_count, 1);
            assert_eq!(
                tree,
                &archive_tree(
                    vec![archive_directory(
                        "folder",
                        vec![archive_file("item.txt", 8)]
                    )],
                    1,
                )
            );
        },
    );
}

#[test]
fn encrypted_zip_preview_rejects_incorrect_and_empty_passwords() {
    crate::test_support::isolated_test(
        "adapters::local_preview::tests::archive::encrypted_zip_preview_rejects_incorrect_and_empty_passwords",
        || {
            let directory = tempfile::tempdir().expect("preview fixture directory");
            let zip = encrypted_archive_fixture(&directory, "secret.zip", ArchiveFormat::Zip);
            for password in ["wrong", ""] {
                let events = preview_archive_with_password(&zip, "secret.zip", Some(password));
                assert_eq!(
                    expect_failure(&events, "an incorrect-password failure"),
                    crate::services::INCORRECT_ARCHIVE_PASSWORD
                );
            }
        },
    );
}

#[test]
fn corrupt_encrypted_zip_preview_reports_a_failed_preview() {
    crate::test_support::isolated_test(
        "adapters::local_preview::tests::archive::corrupt_encrypted_zip_preview_reports_a_failed_preview",
        || {
            let directory = tempfile::tempdir().expect("preview fixture directory");
            let zip = encrypted_archive_fixture(&directory, "secret.zip", ArchiveFormat::Zip);
            let bytes = fs::read(&zip).expect("read fixture");
            fs::write(
                &zip,
                &bytes[..bytes.len().saturating_sub(64).max(bytes.len() / 2)],
            )
            .expect("truncate central directory");
            let events = preview_archive(&zip, "secret.zip");
            assert_eq!(
                expect_failure(&events, "a failed archive preview"),
                DAMAGED_ARCHIVE_MESSAGE
            );
        },
    );
}

#[test]
fn unsupported_archive_preview_reports_unsupported_format() {
    crate::test_support::isolated_test(
        "adapters::local_preview::tests::archive::unsupported_archive_preview_reports_unsupported_format",
        || {
            let directory = tempfile::tempdir().expect("preview fixture directory");
            let path = directory.path().join("sample.zip");
            fs::write(&path, b"PK\x03\x04\0\0\0\0placeholder").expect("write placeholder");
            let events = load_archive(&path, "sample.zip", None, report_unsupported_listing);
            assert_eq!(
                expect_failure(&events, "an unsupported-format failure"),
                crate::adapters::ARCHIVE_UNSUPPORTED_MESSAGE
            );
        },
    );
}
