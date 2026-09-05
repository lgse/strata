// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;

#[test]
fn renders_requested_pdf_pages_within_the_pixel_budget() {
    let path = std::env::temp_dir().join(format!(
        "strata-preview-{}-{}.pdf",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let surface = cairo::PdfSurface::new(612.0, 792.0, &path).expect("create PDF surface");
    {
        let context = cairo::Context::new(&surface).expect("create PDF context");
        context.set_source_rgb(0.2, 0.4, 0.8);
        context.paint().expect("paint PDF page");
        context.show_page().expect("finish first PDF page");
        context.set_source_rgb(0.8, 0.4, 0.2);
        context.paint().expect("paint second PDF page");
        context.show_page().expect("finish second PDF page");
    }
    surface.finish();

    let output_directory = path.with_extension("output");
    fs::create_dir(&output_directory).expect("create output directory");
    let output = output_directory.join("result.png");
    crate::sandbox_helper::run(&[
        "preview-pdf".to_owned(),
        path.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "1".to_owned(),
        "software".to_owned(),
    ])
    .expect("render second PDF page");
    let png = fs::read(&output).expect("read rendered page");
    let metadata =
        fs::read_to_string(output_directory.join("result.meta")).expect("read PDF metadata");
    let _removed = fs::remove_file(path);
    let _removed = fs::remove_dir_all(output_directory);

    assert_eq!(metadata, "1 2");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
}

#[test]
fn parses_database_output_on_initial_load() {
    let raw = b"users\ttable\nactive_users\tview\n---DATA---\nusers\n---SCHEMA---\nCREATE TABLE users (id INTEGER);\n---COUNT---\n42\n---ROWS---\nid,name\n1,Alice\n2,Bob";
    let content = super::parse_database_output(raw, -1);
    let crate::services::PreviewContent::Database { tables, selected } = content else {
        panic!("expected PreviewContent::Database");
    };
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].name, "users");
    assert!(!tables[0].is_view);
    assert_eq!(tables[1].name, "active_users");
    assert!(tables[1].is_view);

    let selected = selected.expect("selected table data");
    assert_eq!(selected.name, "users");
    assert_eq!(selected.schema, "CREATE TABLE users (id INTEGER);");
    assert_eq!(selected.total_rows, Some(42));
    assert_eq!(selected.page, 0);
    assert!(selected.rows_csv.contains("1,Alice"));
}

#[test]
fn parses_database_output_on_table_query() {
    let raw = b"orders\n---SCHEMA---\nCREATE TABLE orders (id INT);\n---TYPES---\nid\tINT\n---COUNT---\n100\n---ROWS---\nid,total\n1,50.0";
    let content = super::parse_database_output(raw, 200_001);
    let crate::services::PreviewContent::DatabaseTable(data) = content else {
        panic!("expected PreviewContent::DatabaseTable");
    };
    assert_eq!(data.name, "orders");
    assert_eq!(data.schema, "CREATE TABLE orders (id INT);");
    assert_eq!(data.total_rows, Some(100));
    assert_eq!(data.page, 1);
    assert_eq!(data.columns.len(), 1);
    assert_eq!(data.columns[0].name, "id");
    assert_eq!(data.columns[0].decl_type, "INT");
    assert!(data.rows_csv.contains("50.0"));
}

#[test]
fn parses_database_output_without_types_marker() {
    let raw =
        b"orders\n---SCHEMA---\nCREATE TABLE orders (id INT);\n---COUNT---\n2\n---ROWS---\nid\n1";
    let content = super::parse_database_output(raw, 0);
    let crate::services::PreviewContent::DatabaseTable(data) = content else {
        panic!("expected PreviewContent::DatabaseTable");
    };
    assert_eq!(data.schema, "CREATE TABLE orders (id INT);");
    assert!(data.columns.is_empty());
    assert_eq!(data.page, 0);
}

#[test]
fn parses_database_output_empty_returns_empty_tables() {
    let content = super::parse_database_output(b"", -1);
    let crate::services::PreviewContent::Database { tables, selected } = content else {
        panic!("expected PreviewContent::Database");
    };
    assert!(tables.is_empty());
    assert!(selected.is_none());
}
