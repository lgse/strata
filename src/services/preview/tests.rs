// SPDX-License-Identifier: MIT

use super::{
    PreviewContent, content_family, has_csv_extension, has_plain_text_extension,
    is_extensionless_dotfile, is_non_executable_extensionless_dotfile, parse_csv_table,
};

#[test]
fn recognizes_configuration_files_as_plain_text() {
    assert!(has_plain_text_extension(std::ffi::OsStr::new(
        "settings.conf"
    )));
    assert!(has_plain_text_extension(std::ffi::OsStr::new(
        "SETTINGS.INI"
    )));
    assert!(!has_plain_text_extension(std::ffi::OsStr::new(
        "archive.zip"
    )));
}

#[test]
fn recognizes_extensionless_dotfiles() {
    assert!(is_extensionless_dotfile(std::ffi::OsStr::new(".steampath")));
    assert!(!is_extensionless_dotfile(std::ffi::OsStr::new("steampath")));
    assert!(!is_extensionless_dotfile(std::ffi::OsStr::new(
        ".settings.toml"
    )));
}

#[test]
fn recognizes_non_executable_extensionless_dotfiles() {
    let name = std::ffi::OsStr::new(".steamid");

    assert!(is_non_executable_extensionless_dotfile(
        name,
        Some(0o100644)
    ));
    assert!(!is_non_executable_extensionless_dotfile(
        name,
        Some(0o100755)
    ));
    assert!(!is_non_executable_extensionless_dotfile(name, None));
}

#[test]
fn classifies_common_preview_content_types() {
    assert_eq!(content_family("image/png"), PreviewContent::Image);
    assert_eq!(content_family("image/gif"), PreviewContent::Media);
    assert_eq!(content_family("video/mp4"), PreviewContent::Media);
    assert!(matches!(
        content_family("application/pdf"),
        PreviewContent::Pdf { .. }
    ));
    assert!(matches!(
        content_family("text/x-rust"),
        PreviewContent::Text { .. }
    ));
    assert!(matches!(
        content_family("application/problem+json"),
        PreviewContent::Text { .. }
    ));
    assert_eq!(
        content_family("application/octet-stream"),
        PreviewContent::Unsupported
    );
    assert!(matches!(
        content_family("text/csv"),
        PreviewContent::Table { .. }
    ));
}

#[test]
fn recognizes_csv_extension() {
    assert!(has_csv_extension(std::ffi::OsStr::new("expenses.CSV")));
    assert!(!has_csv_extension(std::ffi::OsStr::new("notes.txt")));
}

#[test]
fn parses_headers_and_quoted_fields() {
    let (headers, rows, truncated) =
        parse_csv_table("name,description\nwidget,\"a, tricky value\"\ngizmo,plain\n");

    assert_eq!(headers, vec!["name", "description"]);
    assert_eq!(
        rows,
        vec![
            vec!["widget".to_owned(), "a, tricky value".to_owned()],
            vec!["gizmo".to_owned(), "plain".to_owned()],
        ]
    );
    assert!(!truncated);
}

#[test]
fn truncates_rows_past_the_limit() {
    let mut content = String::from("id\n");
    for row in 0..250 {
        content.push_str(&format!("{row}\n"));
    }

    let (_, rows, truncated) = parse_csv_table(&content);

    assert_eq!(rows.len(), super::TABLE_ROW_LIMIT);
    assert!(truncated);
}

#[test]
fn tolerates_ragged_rows() {
    let (headers, rows, truncated) = parse_csv_table("a,b,c\n1,2\n3,4,5,6\n");

    assert_eq!(headers, vec!["a", "b", "c"]);
    assert_eq!(
        rows,
        vec![
            vec!["1".to_owned(), "2".to_owned()],
            vec![
                "3".to_owned(),
                "4".to_owned(),
                "5".to_owned(),
                "6".to_owned()
            ],
        ]
    );
    assert!(!truncated);
}
