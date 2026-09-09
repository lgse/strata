// SPDX-License-Identifier: MIT

use std::io::Write;

use super::{
    EXCEL_BYTE_LIMIT, PreviewContent, content_family, has_csv_extension, has_excel_extension,
    has_plain_text_extension, has_tsv_extension, is_extensionless_dotfile,
    is_non_executable_extensionless_dotfile, parse_delimited_table, parse_excel_table,
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
    assert!(matches!(
        content_family("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        PreviewContent::Table { .. }
    ));
    assert!(matches!(
        content_family("application/vnd.ms-excel"),
        PreviewContent::Table { .. }
    ));
    assert!(matches!(
        content_family("application/vnd.oasis.opendocument.spreadsheet"),
        PreviewContent::Table { .. }
    ));
    assert!(matches!(
        content_family("text/tab-separated-values"),
        PreviewContent::Table { .. }
    ));
}

#[test]
fn recognizes_csv_extension() {
    assert!(has_csv_extension(std::ffi::OsStr::new("expenses.CSV")));
    assert!(!has_csv_extension(std::ffi::OsStr::new("notes.txt")));
}

#[test]
fn recognizes_tsv_extension() {
    assert!(has_tsv_extension(std::ffi::OsStr::new("expenses.TSV")));
    assert!(!has_tsv_extension(std::ffi::OsStr::new("notes.txt")));
}

#[test]
fn parses_headers_and_quoted_fields() {
    let (headers, rows, truncated) = parse_delimited_table(
        "name,description\nwidget,\"a, tricky value\"\ngizmo,plain\n",
        b',',
    );

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
fn parses_tab_delimited_rows() {
    let (headers, rows, truncated) =
        parse_delimited_table("name\tvalue\nalpha\t1\nbeta\t2\n", b'\t');

    assert_eq!(headers, vec!["name", "value"]);
    assert_eq!(
        rows,
        vec![
            vec!["alpha".to_owned(), "1".to_owned()],
            vec!["beta".to_owned(), "2".to_owned()],
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

    let (_, rows, truncated) = parse_delimited_table(&content, b',');

    assert_eq!(rows.len(), super::TABLE_ROW_LIMIT);
    assert!(truncated);
}

#[test]
fn tolerates_ragged_rows() {
    let (headers, rows, truncated) = parse_delimited_table("a,b,c\n1,2\n3,4,5,6\n", b',');

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

#[test]
fn recognizes_excel_extensions() {
    assert!(has_excel_extension(std::ffi::OsStr::new("book.XLSX")));
    assert!(has_excel_extension(std::ffi::OsStr::new("book.xls")));
    assert!(has_excel_extension(std::ffi::OsStr::new("book.ods")));
    assert!(!has_excel_extension(std::ffi::OsStr::new("notes.txt")));
}

fn string_rows(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|row| row.iter().map(ToString::to_string).collect())
        .collect()
}

/// Writes a minimal, valid single-sheet XLSX workbook using inline string
/// cells, so tests don't need a bundled binary fixture or a writer library.
fn write_minimal_xlsx(
    path: &std::path::Path,
    rows: &[Vec<String>],
) -> Result<(), Box<dyn std::error::Error>> {
    let options = zip::write::SimpleFileOptions::default();
    let mut writer = zip::ZipWriter::new(std::fs::File::create(path)?);

    writer.start_file("[Content_Types].xml", options)?;
    writer.write_all(
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#,
    )?;

    writer.start_file("_rels/.rels", options)?;
    writer.write_all(
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#,
    )?;

    writer.start_file("xl/workbook.xml", options)?;
    writer.write_all(
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#,
    )?;

    writer.start_file("xl/_rels/workbook.xml.rels", options)?;
    writer.write_all(
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#,
    )?;

    let column_letter = |index: usize| -> String {
        let mut n = index + 1;
        let mut letters = Vec::new();
        while n > 0 {
            let remainder = (n - 1) % 26;
            letters.push((b'A' + u8::try_from(remainder).unwrap_or(0)) as char);
            n = (n - 1) / 26;
        }
        letters.iter().rev().collect()
    };

    let mut sheet_xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (row_index, row) in rows.iter().enumerate() {
        sheet_xml.push_str(&format!(r#"<row r="{}">"#, row_index + 1));
        for (column_index, value) in row.iter().enumerate() {
            let cell_ref = format!("{}{}", column_letter(column_index), row_index + 1);
            sheet_xml.push_str(&format!(
                r#"<c r="{cell_ref}" t="inlineStr"><is><t>{value}</t></is></c>"#
            ));
        }
        sheet_xml.push_str("</row>");
    }
    sheet_xml.push_str("</sheetData></worksheet>");

    writer.start_file("xl/worksheets/sheet1.xml", options)?;
    writer.write_all(sheet_xml.as_bytes())?;
    writer.finish()?;
    Ok(())
}

#[test]
fn parses_the_first_worksheet_of_a_workbook() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("book.xlsx");
    write_minimal_xlsx(
        &path,
        &string_rows(&[&["name", "value"], &["alpha", "1"], &["beta", "2"]]),
    )?;

    let (headers, rows, truncated) = parse_excel_table(&path).map_err(|error| error.to_string())?;

    assert_eq!(headers, vec!["name", "value"]);
    assert_eq!(
        rows,
        vec![
            vec!["alpha".to_owned(), "1".to_owned()],
            vec!["beta".to_owned(), "2".to_owned()],
        ]
    );
    assert!(!truncated);
    Ok(())
}

#[test]
fn truncates_excel_rows_past_the_limit() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("book.xlsx");
    let mut all_rows = vec![vec!["id".to_owned()]];
    for row in 0..250 {
        all_rows.push(vec![row.to_string()]);
    }
    write_minimal_xlsx(&path, &all_rows)?;

    let (_, rows, truncated) = parse_excel_table(&path).map_err(|error| error.to_string())?;

    assert_eq!(rows.len(), super::TABLE_ROW_LIMIT);
    assert!(truncated);
    Ok(())
}

#[test]
fn rejects_oversized_workbooks() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("big.xlsx");
    let file = std::fs::File::create(&path)?;
    file.set_len(EXCEL_BYTE_LIMIT + 1)?;

    let error = parse_excel_table(&path).expect_err("oversized workbook should be rejected");

    assert!(error.contains("too large"));
    Ok(())
}
