// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn remote_file_url_recognizes_web_urls() {
    assert_eq!(
        remote_file_url("https://example.com/report.pdf"),
        Some("https://example.com/report.pdf".to_owned())
    );
    assert_eq!(
        remote_file_url("  http://files.example.com/a/b.bin?x=1  "),
        Some("http://files.example.com/a/b.bin?x=1".to_owned())
    );
    assert_eq!(
        remote_file_url("HTTPS://EXAMPLE.COM/f.iso"),
        Some("HTTPS://EXAMPLE.COM/f.iso".to_owned())
    );
}

#[test]
fn remote_file_url_rejects_non_web_and_malformed_inputs() {
    for input in [
        "https://",
        "http:///path",
        "ftp://example.com/file",
        "file:///tmp/x",
        "/home/user/file.txt",
        "smb://server/share/f",
        "https:example.com",
        "example.com/file.pdf",
        "javascript:alert(1)",
        "https://user:pass@example.com/f",
        "https://user@example.com/f",
    ] {
        assert_eq!(remote_file_url(input), None, "{input}");
    }
}

#[test]
fn remote_file_name_uses_last_path_segment() {
    assert_eq!(
        remote_file_name("https://example.com/a/b/report.pdf"),
        Some("report.pdf".to_owned())
    );
    assert_eq!(
        remote_file_name("https://example.com/a/b/report%20final.pdf?dl=1"),
        Some("report final.pdf".to_owned())
    );
    assert_eq!(remote_file_name("https://example.com/"), None);
    assert_eq!(remote_file_name("https://example.com"), None);
    // `%2F` can't smuggle traversal out of the temp folder.
    assert_eq!(
        remote_file_name("https://example.com/..%2Fetc"),
        Some("etc".to_owned())
    );
}

#[test]
fn filename_from_disposition_prefers_filename_star() {
    assert_eq!(
        filename_from_disposition(
            "attachment; filename=\"a.txt\"; filename*=UTF-8''%E2%82%AC%20rates.txt"
        ),
        Some("€ rates.txt".to_owned())
    );
}

#[test]
fn filename_from_disposition_handles_quoted_and_escaped_names() {
    assert_eq!(
        filename_from_disposition("attachment; filename=\"quarterly report.pdf\""),
        Some("quarterly report.pdf".to_owned())
    );
    assert_eq!(
        filename_from_disposition("attachment; filename=plain.csv"),
        Some("plain.csv".to_owned())
    );
    assert_eq!(
        filename_from_disposition("attachment; filename=\"with;semi.txt\""),
        Some("with;semi.txt".to_owned())
    );
    assert_eq!(filename_from_disposition("attachment"), None);
}

#[test]
fn sanitize_file_name_strips_directories_and_rejects_unsafe_names() {
    assert_eq!(sanitize_file_name("/etc/passwd"), Some("passwd".to_owned()));
    assert_eq!(
        sanitize_file_name("..\\..\\evil.dll"),
        Some("evil.dll".to_owned())
    );
    assert_eq!(sanitize_file_name("."), None);
    assert_eq!(sanitize_file_name(".."), None);
    assert_eq!(sanitize_file_name("  "), None);
    assert_eq!(sanitize_file_name("a\0b"), None);
    assert_eq!(sanitize_file_name("a\nb"), None);
}

#[test]
fn truncate_name_preserves_extension_on_long_names() {
    let long = format!("{}.txt", "a".repeat(300));
    let truncated = truncate_name(&long);
    assert!(truncated.len() <= MAX_NAME_BYTES);
    assert!(truncated.ends_with(".txt"));
    assert!(truncated.starts_with('a'));

    let no_ext = truncate_name(&"b".repeat(300));
    assert_eq!(no_ext.len(), MAX_NAME_BYTES);
}

#[test]
fn download_writes_body_and_names_file_from_disposition() {
    let base = crate::test_support::serve_http_once(
        b"HTTP/1.1 200 OK\r\ncontent-length: 6\r\ncontent-disposition: attachment; filename=\"report.pdf\"\r\n\r\nbinary".to_vec(),
    );
    let receiver = download_remote(format!("{base}/ignored"), Arc::new(AtomicBool::new(false)));
    let path = loop {
        match receiver.recv().expect("download event") {
            RemoteDownload::Finished(path) => break path,
            RemoteDownload::Progress { .. } | RemoteDownload::Named(_) => continue,
            RemoteDownload::Failed(message) => panic!("download failed: {message}"),
        }
    };
    let directory = path.parent().expect("temp directory");
    assert!(
        directory
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(DOWNLOAD_PREFIX))
    );
    assert_eq!(
        path.file_name().expect("file name").to_string_lossy(),
        "report.pdf"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("downloaded body"),
        "binary"
    );
    // The temp folder persists for the requesting app to consume.
    assert!(directory.exists());
    let _cleanup = fs::remove_dir_all(directory);
}

#[test]
fn download_reports_http_errors() {
    let base = crate::test_support::serve_http_once(
        b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n".to_vec(),
    );
    let receiver = download_remote(format!("{base}/missing"), Arc::new(AtomicBool::new(false)));
    let event = loop {
        match receiver.recv().expect("download event") {
            RemoteDownload::Progress { .. } | RemoteDownload::Named(_) => continue,
            event => break event,
        }
    };
    match event {
        RemoteDownload::Failed(message) => assert!(message.contains("404"), "{message}"),
        RemoteDownload::Finished(path) => panic!("expected failure, got {path:?}"),
        _ => unreachable!(),
    }
}

#[test]
fn download_honors_precancelled_flag() {
    let cancelled = Arc::new(AtomicBool::new(true));
    let receiver = download_remote("https://example.com/never".to_owned(), cancelled);
    match receiver.recv().expect("download event") {
        RemoteDownload::Failed(message) => assert!(message.contains("cancel"), "{message}"),
        RemoteDownload::Named(_)
        | RemoteDownload::Progress { .. }
        | RemoteDownload::Finished(_) => {
            panic!("cancelled download should not produce progress or a file")
        }
    }
}

#[test]
fn prune_removes_only_stale_strata_download_dirs() {
    let tmp = tempfile::tempdir().expect("temp fixture");
    let fresh_ours = tmp.path().join(format!("{DOWNLOAD_PREFIX}fresh"));
    let unrelated_dir = tmp.path().join("other-app-dir");
    let stray_file = tmp.path().join(format!("{DOWNLOAD_PREFIX}file"));
    fs::create_dir(&fresh_ours).expect("fresh dir");
    fs::create_dir(&unrelated_dir).expect("unrelated dir");
    fs::write(&stray_file, b"x").expect("stray file");

    prune_stale_downloads_in(tmp.path());

    assert!(fresh_ours.exists());
    assert!(unrelated_dir.exists());
    assert!(stray_file.exists());
}
