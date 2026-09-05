// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    MEDIA_PLUGIN_INSTALL_COMMAND, PDF_MAX_ZOOM, PDF_MIN_ZOOM, column_decl_for,
    database_count_badge, declares_blob, declares_numeric_affinity, format_database_cell,
    format_file_size, format_media_time, is_numeric_cell, media_error_feedback,
    pdf_zoom_after_scroll, preview_drag_entries, preview_width_for_empty_space, print_fit,
    print_page_starts, print_progress_for_page,
};

#[test]
fn print_fit_centers_landscape_image_on_portrait_page() {
    let (x, y, width, height, scale) = print_fit(595.0, 842.0, 1920.0, 1080.0)
        .expect("print_fit with valid dimensions should return a layout");
    assert!((scale - 0.3098).abs() < 0.001);
    assert!((width - 595.0).abs() < 0.001);
    assert!((height - 334.7).abs() < 0.5);
    assert!(x.abs() < f64::EPSILON);
    assert!((y - (842.0 - height) / 2.0).abs() < 0.5);
}

#[test]
fn print_fit_keeps_tall_image_inside_page() {
    let (x, y, width, height, scale) = print_fit(595.0, 842.0, 1000.0, 2000.0)
        .expect("print_fit with valid dimensions should return a layout");
    let page_scale = 842.0 / 2000.0;
    assert!((scale - page_scale).abs() < 0.001);
    assert!((height - 842.0).abs() < 0.001);
    assert!((width - 1000.0 * page_scale).abs() < 0.001);
    assert!((x - (595.0 - width) / 2.0).abs() < 0.5);
    assert!(y.abs() < f64::EPSILON);
    assert!(
        (y + height / 2.0 - 842.0 / 2.0).abs() < 0.5,
        "image is vertically centered"
    );
}

#[test]
fn print_fit_rejects_zero_sized_inputs() {
    assert_eq!(print_fit(0.0, 842.0, 100.0, 100.0), None);
    assert_eq!(print_fit(595.0, 0.0, 100.0, 100.0), None);
    assert_eq!(print_fit(595.0, 842.0, 0.0, 100.0), None);
    assert_eq!(print_fit(595.0, 842.0, 100.0, -1.0), None);
}

#[test]
fn text_print_pages_start_on_line_boundaries() {
    assert_eq!(
        print_page_starts(&[(0.0, 12.0), (12.0, 24.0), (24.0, 36.0)], 25.0),
        vec![0.0, 24.0]
    );
}

#[test]
fn print_progress_reports_completed_pages() {
    assert_eq!(
        print_progress_for_page(3, 8),
        ("Rendering page 3 of 8".to_owned(), 0.375)
    );
}

#[test]
fn print_progress_clamps_invalid_counts() {
    assert_eq!(
        print_progress_for_page(3, 0),
        ("Rendering page 1 of 1".to_owned(), 1.0)
    );
}

#[test]
fn formats_preview_file_sizes() {
    assert_eq!(format_file_size(999), "999 B");
    assert_eq!(format_file_size(1_200), "1.2 kB");
    assert_eq!(format_file_size(2_500_000), "2.5 MB");
}

#[test]
fn media_errors_explain_missing_runtime_plugins() {
    let (title, detail, command) =
        media_error_feedback("Your GStreamer installation is missing a plug-in.");
    assert_eq!(title, "Additional media support required");
    assert!(detail.contains("GStreamer plugins"));
    assert_eq!(command, Some(MEDIA_PLUGIN_INSTALL_COMMAND));
    assert_eq!(
        command,
        Some("sudo pacman -S --needed gst-plugins-good gst-libav")
    );

    let (title, detail, command) = media_error_feedback("The media data is corrupt");
    assert_eq!(title, "Preview unavailable");
    assert!(detail.contains("The media data is corrupt"));
    assert_eq!(command, None);
}

#[test]
fn initial_preview_uses_most_of_the_unoccupied_width() {
    assert_eq!(preview_width_for_empty_space(2_000, 500), 1_350);
    assert_eq!(preview_width_for_empty_space(700, 650), 280);
}

#[test]
fn pdf_scroll_zoom_stays_within_its_supported_range() {
    assert!(pdf_zoom_after_scroll(1.0, -1.0) > 1.0);
    assert!(pdf_zoom_after_scroll(2.0, 1.0) < 2.0);
    assert_eq!(pdf_zoom_after_scroll(PDF_MIN_ZOOM, 100.0), PDF_MIN_ZOOM);
    assert_eq!(pdf_zoom_after_scroll(PDF_MAX_ZOOM, -100.0), PDF_MAX_ZOOM);
}

#[test]
fn media_time_formats_minutes_and_seconds() {
    assert_eq!(format_media_time(0, 0), "0:00/0:00");
    assert_eq!(format_media_time(1_500_000, 65_000_000), "0:01/1:05");
    assert_eq!(format_media_time(125_000_000, 125_000_000), "2:05/2:05");
}

#[test]
fn media_time_clamps_negative_timestamps_to_zero() {
    assert_eq!(format_media_time(-500_000, 10_000_000), "0:00/0:10");
}

#[test]
fn parse_csv_rows_handles_basic_table() {
    let csv = "id,name,role\n1,Alice,admin\n2,Bob,member";
    let (headers, rows) = super::parse_csv_rows(csv);
    assert_eq!(headers, vec!["id", "name", "role"]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], vec!["1", "Alice", "admin"]);
    assert_eq!(rows[1], vec!["2", "Bob", "member"]);
}

#[test]
fn parse_csv_rows_handles_quotes_and_commas() {
    let csv = "id,description\n1,\"Item, with comma\"\n2,\"Item with \"\"quotes\"\"\"";
    let (headers, rows) = super::parse_csv_rows(csv);
    assert_eq!(headers, vec!["id", "description"]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], vec!["1", "Item, with comma"]);
    assert_eq!(rows[1], vec!["2", "Item with \"quotes\""]);
}

#[test]
fn parse_csv_rows_handles_multiline_cells() {
    let csv = "id,notes\n1,\"Line 1\nLine 2\"\n2,Single line";
    let (headers, rows) = super::parse_csv_rows(csv);
    assert_eq!(headers, vec!["id", "notes"]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], vec!["1", "Line 1\nLine 2"]);
    assert_eq!(rows[1], vec!["2", "Single line"]);
}

#[test]
fn parse_csv_rows_handles_empty_input() {
    let (headers, rows) = super::parse_csv_rows("");
    assert!(headers.is_empty());
    assert!(rows.is_empty());
}

#[test]
fn numeric_cells_cover_plain_and_scientific_notation() {
    for cell in ["42", "-3.5", "+0.25", "1e6", "007"] {
        assert!(is_numeric_cell(cell), "{cell} should align numerically");
    }
    for cell in ["", "12px", "1,000", "2024-01-02", "NULL", "  "] {
        assert!(!is_numeric_cell(cell), "{cell} should align as text");
    }
}

#[test]
fn database_cells_flatten_newlines_and_truncate() {
    assert_eq!(format_database_cell("Line 1\nLine 2"), "Line 1 ⏎ Line 2");
    assert_eq!(format_database_cell("a\rb\nc"), "ab ⏎ c");
    let long = "x".repeat(250);
    let display = format_database_cell(&long);
    assert_eq!(display.chars().count(), 201);
    assert!(display.ends_with('…'));
    assert_eq!(format_database_cell("short"), "short");
}

#[test]
fn database_count_badge_combines_rows_and_columns() {
    assert_eq!(database_count_badge(Some(1), 3), "1 row · 3 cols");
    assert_eq!(database_count_badge(Some(42), 1), "42 rows · 1 col");
    assert_eq!(database_count_badge(Some(0), 2), "0 rows · 2 cols");
    assert_eq!(database_count_badge(None, 4), "4 cols");
    assert_eq!(database_count_badge(Some(7), 0), "7 rows");
    assert_eq!(database_count_badge(None, 0), "");
}

#[test]
fn declared_types_drive_numeric_and_blob_cells() {
    for decl in [
        "INTEGER",
        "INT",
        "BIGINT",
        "REAL",
        "DOUBLE PRECISION",
        "FLOAT",
        "DECIMAL(10,2)",
        "NUMERIC",
        "BOOLEAN",
    ] {
        assert!(
            declares_numeric_affinity(decl),
            "{decl} should align numerically"
        );
        assert!(!declares_blob(decl));
    }
    for decl in ["TEXT", "VARCHAR(10)", "CHAR(1)", "CLOB", "BLOB", "", "  "] {
        assert!(
            !declares_numeric_affinity(decl),
            "{decl} should not force alignment"
        );
    }
    assert!(declares_blob("BLOB"));
    assert!(!declares_blob("TEXT"));
    assert!(!declares_blob(""));
}

#[test]
fn column_decls_prefer_position_then_fall_back_to_name() {
    let columns = vec![
        crate::services::DatabaseColumn {
            name: "id".to_owned(),
            decl_type: "INTEGER".to_owned(),
        },
        crate::services::DatabaseColumn {
            name: "name".to_owned(),
            decl_type: "TEXT".to_owned(),
        },
    ];
    let headers = vec!["id".to_owned(), "name".to_owned()];
    assert_eq!(column_decl_for(&columns, &headers, 0), "INTEGER");
    assert_eq!(column_decl_for(&columns, &headers, 5), "");

    let reordered = vec!["name".to_owned(), "id".to_owned()];
    assert_eq!(column_decl_for(&columns, &reordered, 0), "TEXT");
    assert_eq!(column_decl_for(&[], &headers, 0), "");
}

#[test]
fn parse_csv_rows_preserves_single_column_empty_and_null_rows() {
    let csv = "name\n\x01\n\"\"\nvisible\n\"\"\n";
    let (headers, rows) = super::parse_csv_rows(csv);
    assert_eq!(headers, vec!["name"]);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0], vec!["\x01"]);
    assert_eq!(rows[1], vec![""]);
    assert_eq!(rows[2], vec!["visible"]);
    assert_eq!(rows[3], vec![""]);
}

#[test]
fn parse_csv_rows_handles_multi_column_empty_and_null_cells() {
    let csv = "c1,c2\r\n\x01,\"\"\r\n\"\",\x01\r\n\x01,\x01\r\n\"\",\"\"\r\n";
    let (headers, rows) = super::parse_csv_rows(csv);
    assert_eq!(headers, vec!["c1", "c2"]);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0], vec!["\x01", ""]);
    assert_eq!(rows[1], vec!["", "\x01"]);
    assert_eq!(rows[2], vec!["\x01", "\x01"]);
    assert_eq!(rows[3], vec!["", ""]);
}

#[test]
fn preview_drag_entries_returns_none_when_no_entry_loaded() {
    assert_eq!(preview_drag_entries(None), None);
}

#[test]
fn preview_drag_entries_wraps_loaded_file_entry() {
    let entry = crate::model::FileEntry {
        location: crate::model::Location::local("/tmp/test.png"),
        native_name: std::ffi::OsString::from("test.png"),
        display_name: "test.png".to_owned(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Known(100),
        modified_unix_seconds: crate::model::MetadataValue::Known(1),
        mode: crate::model::MetadataValue::Unknown,
        is_hidden: false,
    };
    let dragged = preview_drag_entries(Some(&entry));
    assert_eq!(dragged, Some(vec![entry]));
}
